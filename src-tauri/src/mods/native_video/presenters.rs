use std::any::Any;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::mods::runtime_trace::RuntimeTraceRecorderRef;
#[cfg(target_os = "windows")]
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use tauri::{AppHandle, Manager};
use xbxengine::{
    MacOsCVPixelBufferDescriptor, WindowsD3d11TextureDescriptor, XbxEngineRenderFrame,
    XbxEngineRenderPixelData,
};

#[cfg(target_os = "windows")]
use super::scheduling::ScheduledFrameTakeOutcome;
use super::scheduling::{HostCadenceTelemetry, ScheduledFrameSlot, ScheduledFrameSlotDiagnostics};
use super::{
    clear_host_present_tick_dispatch, now_ms_f64, record_native_video_timing_event_lazy,
    record_native_video_trace, request_host_present_tick_dispatch, NativeVideoDisplayState,
    NativeVideoViewportState,
};
#[cfg(target_os = "macos")]
use super::{
    dispatch_wgpu_render_tick_on_main_thread, drop_display_layer, drop_wgpu_host_view,
    ensure_display_layer, run_layer_present_tick, MacOsDisplayLinkHandle,
    MacOsLayerDisplayLinkHandle, MacOsLayerState, MacOsWgpuState,
};
#[cfg(any(target_os = "macos", target_os = "windows"))]
use super::{record_host_mailbox_take_decision, HostPresentTickGuard};
#[cfg(target_os = "windows")]
use super::{HOST_TIMING_QUEUE_WARN_MS, HOST_TIMING_TICK_WARN_MS};

fn apply_host_mailbox_viewport_diagnostics(
    viewport: &mut NativeVideoViewportState,
    telemetry: &HostCadenceTelemetry,
    slot_diag: Option<&ScheduledFrameSlotDiagnostics>,
    latest_host_view_created_at_ms: Option<f64>,
    host_descriptor_upload_mode: Option<String>,
    host_descriptor_metal_import_count_total: u64,
    host_descriptor_cpu_upload_count_total: u64,
) {
    viewport.latest_host_present_time_ms = telemetry.latest_present_time_ms;
    viewport.latest_host_submit_time_ms = telemetry.latest_submit_time_ms;
    viewport.latest_host_submit_rtp_timestamp = slot_diag
        .and_then(|diag| diag.pending_frame_rtp_timestamp)
        .or_else(|| slot_diag.and_then(|diag| diag.displayed_frame_rtp_timestamp));
    viewport.host_present_fps = telemetry.present_fps();
    viewport.host_mailbox_submit_epoch = telemetry.present_enqueue_count_total;
    viewport.host_mailbox_enqueue_count_total = telemetry.present_enqueue_count_total;
    viewport.host_mailbox_drop_count_total = telemetry.present_drop_count_total;
    viewport.host_mailbox_overwrite_count_total = telemetry.present_overwrite_count_total;
    viewport.host_no_pending_take_count_total = telemetry.no_pending_take_count_total;
    viewport.host_no_pending_streak = telemetry.no_pending_streak;
    viewport.host_no_pending_max_streak = telemetry.no_pending_max_streak;
    viewport.host_display_tick_epoch = telemetry.display_tick_epoch();
    viewport.host_frame_present_epoch = telemetry.present_epoch();
    viewport.host_cadence_phase = Some(telemetry.cadence_phase().as_str().to_string());
    viewport.last_displayed_frame_seq = slot_diag.and_then(|diag| diag.displayed_frame_seq);
    viewport.last_displayed_frame_rtp_timestamp =
        slot_diag.and_then(|diag| diag.displayed_frame_rtp_timestamp);
    viewport.last_displayed_at_ms = telemetry.latest_present_time_ms;
    viewport.host_view_generation = slot_diag.map(|diag| diag.view_epoch).unwrap_or_default();
    viewport.latest_host_view_created_at_ms = latest_host_view_created_at_ms;
    viewport.host_display_interval_ms = telemetry.display_interval_ms();
    viewport.host_frame_age_budget_ms = Some(telemetry.frame_age_budget_ms());
    viewport.host_descriptor_upload_mode = host_descriptor_upload_mode;
    viewport.host_descriptor_metal_import_count_total = host_descriptor_metal_import_count_total;
    viewport.host_descriptor_cpu_upload_count_total = host_descriptor_cpu_upload_count_total;
}

fn pending_frame_seq(slot_diag: &ScheduledFrameSlotDiagnostics) -> Option<u64> {
    slot_diag.pending_frame_seqs.first().copied()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum NativeVideoPresenterKind {
    Noop,
    PlatformNative,
    Wgpu,
}

pub(super) trait NativeVideoPresenter: Send {
    fn kind(&self) -> NativeVideoPresenterKind;
    fn attach(&mut self, surface_id: Option<&str>);
    fn present(&mut self, surface_id: Option<&str>, frame: &XbxEngineRenderFrame) -> bool;
    fn detach(&mut self);
    /// Host present 停滞时优先走邮箱重置，避免拆掉 wgpu host view / renderer 引发主线程重初始化卡死。
    fn reset_mailbox_for_host_stall_recovery(&mut self) -> bool {
        false
    }
    fn begin_media_epoch(&mut self) {}
    fn apply_display_state(&mut self, _state: &NativeVideoDisplayState) {}
    fn apply_viewport_diagnostics(&self, _viewport: &mut NativeVideoViewportState) {}
    fn take_pending_frame_drops(&mut self) -> Vec<xbxengine::XbxEngineHostVideoFrameDropEvent> {
        Vec::new()
    }
}

pub(super) struct NoopVideoPresenter {
    #[allow(dead_code)]
    viewport_id: String,
    #[allow(dead_code)]
    window_label: String,
    #[allow(dead_code)]
    surface_id: Option<String>,
    #[allow(dead_code)]
    display_state: NativeVideoDisplayState,
}

impl NoopVideoPresenter {
    pub(super) fn new(viewport_id: &str, window_label: &str) -> Self {
        Self {
            viewport_id: viewport_id.to_string(),
            window_label: window_label.to_string(),
            surface_id: None,
            display_state: NativeVideoDisplayState::default(),
        }
    }
}

impl NativeVideoPresenter for NoopVideoPresenter {
    fn kind(&self) -> NativeVideoPresenterKind {
        NativeVideoPresenterKind::Noop
    }

    fn attach(&mut self, surface_id: Option<&str>) {
        self.surface_id = surface_id.map(str::to_string);
    }

    fn present(&mut self, surface_id: Option<&str>, _frame: &XbxEngineRenderFrame) -> bool {
        self.surface_id = surface_id
            .map(str::to_string)
            .or_else(|| self.surface_id.clone());
        true
    }

    fn detach(&mut self) {
        self.surface_id = None;
    }

    fn apply_display_state(&mut self, state: &NativeVideoDisplayState) {
        self.display_state = state.clone();
    }
}

#[cfg(target_os = "windows")]
#[derive(Default)]
struct WindowsWgpuState {
    renderer: Option<super::wgpu_renderer::WgpuFrameRenderer>,
    latest_frame: Option<XbxEngineRenderFrame>,
    view_generation: u64,
    latest_view_created_at_ms: Option<f64>,
    last_surface_size: Option<(u32, u32)>,
    descriptor_upload_mode: Option<String>,
    descriptor_metal_import_count_total: u64,
    descriptor_cpu_upload_count_total: u64,
    render_loop_started: bool,
    init_failed_logged: bool,
}

#[cfg(target_os = "windows")]
pub(super) struct WindowsWgpuPresenter {
    viewport_id: String,
    window_label: String,
    surface_id: Option<String>,
    app_handle: AppHandle,
    renderer_state: Arc<Mutex<WindowsWgpuState>>,
    frame_slot: Arc<Mutex<ScheduledFrameSlot>>,
    telemetry: Arc<Mutex<HostCadenceTelemetry>>,
    render_loop_stop: Arc<std::sync::atomic::AtomicBool>,
    render_loop_pending: Arc<std::sync::atomic::AtomicBool>,
    render_loop_rerun_requested: Arc<std::sync::atomic::AtomicBool>,
    runtime_trace: Option<RuntimeTraceRecorderRef>,
    display_state: NativeVideoDisplayState,
}

#[cfg(target_os = "windows")]
impl WindowsWgpuPresenter {
    pub(super) fn new(
        viewport_id: &str,
        window_label: &str,
        app_handle: AppHandle,
        runtime_trace: Option<RuntimeTraceRecorderRef>,
    ) -> Self {
        Self {
            viewport_id: viewport_id.to_string(),
            window_label: window_label.to_string(),
            surface_id: None,
            app_handle,
            renderer_state: Arc::new(Mutex::new(WindowsWgpuState::default())),
            frame_slot: Arc::new(Mutex::new(ScheduledFrameSlot::default())),
            telemetry: Arc::new(Mutex::new(HostCadenceTelemetry::default())),
            render_loop_stop: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            render_loop_pending: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            render_loop_rerun_requested: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            runtime_trace,
            display_state: NativeVideoDisplayState::default(),
        }
    }

    fn ensure_render_loop(&mut self) {
        let Ok(mut state) = self.renderer_state.lock() else {
            return;
        };
        if state.render_loop_started {
            return;
        }
        state.render_loop_started = true;
        self.render_loop_stop.store(false, Ordering::Relaxed);

        let viewport_id = self.viewport_id.clone();
        let window_label = self.window_label.clone();
        let app_handle = self.app_handle.clone();
        let renderer_state = self.renderer_state.clone();
        let frame_slot = self.frame_slot.clone();
        let telemetry = self.telemetry.clone();
        let render_loop_stop = self.render_loop_stop.clone();
        let render_loop_pending = self.render_loop_pending.clone();
        let render_loop_rerun_requested = self.render_loop_rerun_requested.clone();
        let runtime_trace = self.runtime_trace.clone();
        thread::Builder::new()
            .name(format!("XbxWindowsWgpuRenderLoop-{viewport_id}"))
            .spawn(move || {
                while !render_loop_stop.load(Ordering::Relaxed) {
                    let tick = telemetry
                        .lock()
                        .map(|state| state.present_tick_sleep_duration())
                        .unwrap_or(Duration::from_millis(16));
                    thread::sleep(tick);
                    if !request_host_present_tick_dispatch(
                        &render_loop_pending,
                        &render_loop_rerun_requested,
                    ) {
                        continue;
                    }
                    let Some(window) = app_handle.get_window(&window_label) else {
                        clear_host_present_tick_dispatch(
                            &render_loop_pending,
                            &render_loop_rerun_requested,
                        );
                        continue;
                    };
                    let renderer_state = renderer_state.clone();
                    let frame_slot = frame_slot.clone();
                    let telemetry = telemetry.clone();
                    let render_loop_pending = render_loop_pending.clone();
                    let render_loop_rerun_requested = render_loop_rerun_requested.clone();
                    let viewport_id = viewport_id.clone();
                    let runtime_trace_for_task = runtime_trace.clone();
                    let dispatch_requested_at_ms = now_ms_f64();
                    let window_for_task = window.clone();
                    let _ = window.run_on_main_thread(move || {
                        run_windows_wgpu_render_tick(
                            &window_for_task,
                            &viewport_id,
                            &renderer_state,
                            &frame_slot,
                            &telemetry,
                            &render_loop_pending,
                            &render_loop_rerun_requested,
                            Some(dispatch_requested_at_ms),
                            runtime_trace_for_task,
                        );
                    });
                }
            })
            .expect("Failed to spawn Windows wgpu render loop");
    }

    fn request_immediate_render_tick(&self) {
        if !request_host_present_tick_dispatch(
            &self.render_loop_pending,
            &self.render_loop_rerun_requested,
        ) {
            return;
        }
        let Some(window) = self.app_handle.get_window(&self.window_label) else {
            clear_host_present_tick_dispatch(
                &self.render_loop_pending,
                &self.render_loop_rerun_requested,
            );
            return;
        };
        let renderer_state = self.renderer_state.clone();
        let frame_slot = self.frame_slot.clone();
        let telemetry = self.telemetry.clone();
        let render_loop_pending = self.render_loop_pending.clone();
        let render_loop_rerun_requested = self.render_loop_rerun_requested.clone();
        let viewport_id = self.viewport_id.clone();
        let runtime_trace = self.runtime_trace.clone();
        let window_for_task = window.clone();
        let dispatch_requested_at_ms = now_ms_f64();
        let _ = window.run_on_main_thread(move || {
            run_windows_wgpu_render_tick(
                &window_for_task,
                &viewport_id,
                &renderer_state,
                &frame_slot,
                &telemetry,
                &render_loop_pending,
                &render_loop_rerun_requested,
                Some(dispatch_requested_at_ms),
                runtime_trace,
            );
        });
    }

    fn should_drop_submitted_frame(&self, frame: &XbxEngineRenderFrame, now_ms: f64) -> bool {
        let Ok(telemetry) = self.telemetry.lock() else {
            return false;
        };
        now_ms - frame.rendered_at_ms > telemetry.frame_age_budget_ms()
    }
}

#[cfg(target_os = "windows")]
impl NativeVideoPresenter for WindowsWgpuPresenter {
    fn kind(&self) -> NativeVideoPresenterKind {
        NativeVideoPresenterKind::Wgpu
    }

    fn attach(&mut self, surface_id: Option<&str>) {
        self.begin_media_epoch();
        self.surface_id = surface_id.map(str::to_string);
        self.ensure_render_loop();
    }

    fn begin_media_epoch(&mut self) {
        if let Ok(mut state) = self.renderer_state.lock() {
            state.latest_frame = None;
            state.last_surface_size = None;
            state.descriptor_upload_mode = None;
            state.descriptor_metal_import_count_total = 0;
            state.descriptor_cpu_upload_count_total = 0;
        }
        if let Ok(mut frame_slot) = self.frame_slot.lock() {
            frame_slot.begin_media_epoch();
        }
        if let Ok(mut telemetry) = self.telemetry.lock() {
            telemetry.reset_frame_slot();
        }
        self.render_loop_pending.store(false, Ordering::Relaxed);
    }

    fn reset_mailbox_for_host_stall_recovery(&mut self) -> bool {
        self.begin_media_epoch();
        clear_host_present_tick_dispatch(
            &self.render_loop_pending,
            &self.render_loop_rerun_requested,
        );
        self.render_loop_stop.store(false, Ordering::Relaxed);
        self.ensure_render_loop();
        true
    }

    fn apply_display_state(&mut self, state: &NativeVideoDisplayState) {
        self.display_state = state.clone();
    }

    fn present(&mut self, surface_id: Option<&str>, frame: &XbxEngineRenderFrame) -> bool {
        self.surface_id = surface_id
            .map(str::to_string)
            .or_else(|| self.surface_id.clone());
        self.ensure_render_loop();
        let now_ms = now_ms_f64();
        if self.should_drop_submitted_frame(frame, now_ms) {
            if let Ok(mut telemetry) = self.telemetry.lock() {
                telemetry.present_enqueue_count_total =
                    telemetry.present_enqueue_count_total.saturating_add(1);
                telemetry.record_stale_frame_drop(frame, now_ms, "submittedFrameStale", 0);
            }
            record_native_video_timing_event_lazy(
                self.runtime_trace.as_ref(),
                "wgpu-windows",
                "hostMailboxRejected",
                &self.viewport_id,
                &self.window_label,
                || {
                    serde_json::json!({
                        "outcome": "stale",
                        "frameSeq": frame.frame_seq,
                        "frameAgeMs": (now_ms - frame.rendered_at_ms).max(0.0),
                    })
                },
            );
            return false;
        }
        let Ok(mut telemetry) = self.telemetry.lock() else {
            record_native_video_timing_event_lazy(
                self.runtime_trace.as_ref(),
                "wgpu",
                "hostMailboxUpdateFailed",
                &self.viewport_id,
                &self.window_label,
                || serde_json::json!({ "reason": "telemetryLockFailed", "frameSeq": frame.frame_seq }),
            );
            return false;
        };
        let previous_submit_time_ms = telemetry.latest_submit_time_ms;
        let submit_gap_ms = telemetry.record_submit(now_ms);
        let no_pending_streak_before_submit = telemetry.no_pending_streak;
        let should_warn_submit_gap =
            submit_gap_ms.is_some_and(|gap_ms| telemetry.should_warn_submit_gap(gap_ms));
        telemetry.present_enqueue_count_total =
            telemetry.present_enqueue_count_total.saturating_add(1);
        let telemetry_diag = telemetry.diagnostics_snapshot();
        let Ok(mut frame_slot) = self.frame_slot.lock() else {
            record_native_video_timing_event_lazy(
                self.runtime_trace.as_ref(),
                "wgpu",
                "hostMailboxUpdateFailed",
                &self.viewport_id,
                &self.window_label,
                || serde_json::json!({ "reason": "frameSlotLockFailed", "frameSeq": frame.frame_seq }),
            );
            return false;
        };
        match frame_slot.submit_frame(frame, now_ms, &mut telemetry) {
            super::scheduling::ScheduledFrameSubmitOutcome::Accepted {
                frame_seq,
                overwrote_pending,
                replaced_frame_seq,
                frame_age_ms,
                frame_age_budget_ms,
            } => {
                let slot_diag = frame_slot.diagnostics_snapshot();
                record_native_video_timing_event_lazy(
                    self.runtime_trace.as_ref(),
                    "wgpu",
                    "hostMailboxAccepted",
                    &self.viewport_id,
                    &self.window_label,
                    || {
                        serde_json::json!({
                            "outcome": "accepted",
                            "frameSeq": frame_seq,
                            "frameRtpTimestamp": frame.rtp_timestamp,
                            "frameRecoveryEpoch": frame.recovery_epoch_tag,
                            "frameRecoveryOwnerRtpTimestamp": frame.recovery_owner_rtp_timestamp,
                            "frameRecoveryDisposition": frame.frame_recovery_disposition,
                            "frameIsKeyframe": frame.is_keyframe,
                            "frameAgeMs": frame_age_ms,
                            "frameAgeBudgetMs": frame_age_budget_ms,
                            "submitGapMs": submit_gap_ms,
                            "noPendingStreakBeforeSubmit": no_pending_streak_before_submit,
                            "overwrotePending": overwrote_pending,
                            "replacedFrameSeq": replaced_frame_seq,
                            "displayedFrameSeq": slot_diag.displayed_frame_seq,
                            "pendingFrameSeq": pending_frame_seq(&slot_diag),
                            "hasPendingFrame": slot_diag.pending_queue_depth > 0,
                            "pendingFrameSeqs": slot_diag.pending_frame_seqs,
                            "lastPresentedFrameSeq": slot_diag.last_presented_frame_seq,
                            "queueDepth": slot_diag.queue_depth,
                            "pendingQueueDepth": slot_diag.pending_queue_depth,
                            "hostDisplayTickEpoch": telemetry_diag.display_tick_epoch,
                            "hostFramePresentEpoch": telemetry_diag.present_epoch,
                            "hostCadencePhase": telemetry_diag.cadence_phase.as_str(),
                        })
                    },
                );
                if let Some(gap_ms) = submit_gap_ms.filter(|_| should_warn_submit_gap) {
                    record_native_video_timing_event_lazy(
                        self.runtime_trace.as_ref(),
                        "wgpu",
                        "hostMailboxSubmitGap",
                        &self.viewport_id,
                        &self.window_label,
                        || {
                            serde_json::json!({
                                "frameSeq": frame_seq,
                                "submitGapMs": gap_ms,
                                "frameAgeMs": frame_age_ms,
                                "frameAgeBudgetMs": frame_age_budget_ms,
                                "noPendingStreakBeforeSubmit": no_pending_streak_before_submit,
                                "overwrotePending": overwrote_pending,
                                "replacedFrameSeq": replaced_frame_seq,
                                "displayedFrameSeq": slot_diag.displayed_frame_seq,
                                "pendingFrameSeq": pending_frame_seq(&slot_diag),
                                "hasPendingFrame": slot_diag.pending_queue_depth > 0,
                                "pendingFrameSeqs": slot_diag.pending_frame_seqs,
                                "queueDepth": slot_diag.queue_depth,
                                "pendingQueueDepth": slot_diag.pending_queue_depth,
                                "hostDisplayTickEpoch": telemetry_diag.display_tick_epoch,
                                "hostFramePresentEpoch": telemetry_diag.present_epoch,
                            })
                        },
                    );
                }
                self.request_immediate_render_tick();
                true
            }
            super::scheduling::ScheduledFrameSubmitOutcome::DroppedStale {
                frame_seq,
                frame_age_ms,
                frame_age_budget_ms,
            } => {
                let slot_diag = frame_slot.diagnostics_snapshot();
                record_native_video_timing_event_lazy(
                    self.runtime_trace.as_ref(),
                    "wgpu",
                    "hostMailboxRejected",
                    &self.viewport_id,
                    &self.window_label,
                    || {
                        serde_json::json!({
                            "outcome": "stale",
                            "frameSeq": frame_seq,
                            "frameRtpTimestamp": frame.rtp_timestamp,
                            "frameRecoveryEpoch": frame.recovery_epoch_tag,
                            "frameRecoveryOwnerRtpTimestamp": frame.recovery_owner_rtp_timestamp,
                            "frameRecoveryDisposition": frame.frame_recovery_disposition,
                            "frameIsKeyframe": frame.is_keyframe,
                            "frameAgeMs": frame_age_ms,
                            "frameAgeBudgetMs": frame_age_budget_ms,
                            "submitGapMs": submit_gap_ms,
                            "noPendingStreakBeforeSubmit": no_pending_streak_before_submit,
                            "displayedFrameSeq": slot_diag.displayed_frame_seq,
                            "pendingFrameSeq": pending_frame_seq(&slot_diag),
                            "hasPendingFrame": slot_diag.pending_queue_depth > 0,
                            "pendingFrameSeqs": slot_diag.pending_frame_seqs,
                            "queueDepth": slot_diag.queue_depth,
                            "pendingQueueDepth": slot_diag.pending_queue_depth,
                            "hostDisplayTickEpoch": telemetry_diag.display_tick_epoch,
                            "hostFramePresentEpoch": telemetry_diag.present_epoch,
                            "hostCadencePhase": telemetry_diag.cadence_phase.as_str(),
                        })
                    },
                );
                telemetry.latest_submit_time_ms = previous_submit_time_ms;
                false
            }
            super::scheduling::ScheduledFrameSubmitOutcome::RejectedAlreadyPresented {
                frame_seq,
                last_presented_frame_seq,
            } => {
                let slot_diag = frame_slot.diagnostics_snapshot();
                record_native_video_timing_event_lazy(
                    self.runtime_trace.as_ref(),
                    "wgpu",
                    "hostMailboxRejected",
                    &self.viewport_id,
                    &self.window_label,
                    || {
                        serde_json::json!({
                            "outcome": "already_presented",
                            "frameSeq": frame_seq,
                            "frameRtpTimestamp": frame.rtp_timestamp,
                            "frameRecoveryEpoch": frame.recovery_epoch_tag,
                            "frameRecoveryOwnerRtpTimestamp": frame.recovery_owner_rtp_timestamp,
                            "frameRecoveryDisposition": frame.frame_recovery_disposition,
                            "frameIsKeyframe": frame.is_keyframe,
                            "lastPresentedFrameSeq": last_presented_frame_seq,
                            "submitGapMs": submit_gap_ms,
                            "noPendingStreakBeforeSubmit": no_pending_streak_before_submit,
                            "displayedFrameSeq": slot_diag.displayed_frame_seq,
                            "pendingFrameSeq": pending_frame_seq(&slot_diag),
                            "hasPendingFrame": slot_diag.pending_queue_depth > 0,
                            "pendingFrameSeqs": slot_diag.pending_frame_seqs,
                            "queueDepth": slot_diag.queue_depth,
                            "pendingQueueDepth": slot_diag.pending_queue_depth,
                            "hostDisplayTickEpoch": telemetry_diag.display_tick_epoch,
                            "hostFramePresentEpoch": telemetry_diag.present_epoch,
                            "hostCadencePhase": telemetry_diag.cadence_phase.as_str(),
                        })
                    },
                );
                telemetry.latest_submit_time_ms = previous_submit_time_ms;
                false
            }
        }
    }

    fn detach(&mut self) {
        self.surface_id = None;
        self.render_loop_stop.store(true, Ordering::Relaxed);
        let renderer_state = self.renderer_state.clone();
        let frame_slot = self.frame_slot.clone();
        let telemetry = self.telemetry.clone();
        let window_label = self.window_label.clone();
        let app_handle = self.app_handle.clone();
        if let Some(window) = app_handle.get_window(&window_label) {
            let _ = window.run_on_main_thread(move || {
                if let Ok(mut frame_slot) = frame_slot.lock() {
                    frame_slot.reset();
                }
                if let Ok(mut state) = renderer_state.lock() {
                    state.renderer = None;
                    state.latest_frame = None;
                    state.last_surface_size = None;
                    state.descriptor_upload_mode = None;
                    state.descriptor_metal_import_count_total = 0;
                    state.descriptor_cpu_upload_count_total = 0;
                    state.render_loop_started = false;
                }
                if let Ok(mut telemetry) = telemetry.lock() {
                    telemetry.reset_frame_slot();
                }
            });
        }
    }

    fn apply_viewport_diagnostics(&self, viewport: &mut NativeVideoViewportState) {
        let Ok(telemetry) = self.telemetry.lock() else {
            return;
        };
        let slot_diag = self
            .frame_slot
            .lock()
            .ok()
            .map(|frame_slot| frame_slot.diagnostics_snapshot());
        let renderer_state = self.renderer_state.lock().ok();
        apply_host_mailbox_viewport_diagnostics(
            viewport,
            &telemetry,
            slot_diag.as_ref(),
            renderer_state
                .as_ref()
                .and_then(|state| state.latest_view_created_at_ms),
            renderer_state
                .as_ref()
                .and_then(|state| state.descriptor_upload_mode.clone()),
            renderer_state
                .as_ref()
                .map(|state| state.descriptor_metal_import_count_total)
                .unwrap_or_default(),
            renderer_state
                .as_ref()
                .map(|state| state.descriptor_cpu_upload_count_total)
                .unwrap_or_default(),
        );
    }

    fn take_pending_frame_drops(&mut self) -> Vec<xbxengine::XbxEngineHostVideoFrameDropEvent> {
        self.telemetry
            .lock()
            .map(|mut telemetry| telemetry.take_pending_frame_drops())
            .unwrap_or_default()
    }
}

#[cfg(target_os = "windows")]
fn run_windows_wgpu_render_tick(
    window: &tauri::Window,
    viewport_id: &str,
    renderer_state: &Arc<Mutex<WindowsWgpuState>>,
    frame_slot: &Arc<Mutex<ScheduledFrameSlot>>,
    telemetry: &Arc<Mutex<HostCadenceTelemetry>>,
    render_loop_pending: &Arc<std::sync::atomic::AtomicBool>,
    render_loop_rerun_requested: &Arc<std::sync::atomic::AtomicBool>,
    dispatch_requested_at_ms: Option<f64>,
    runtime_trace: Option<RuntimeTraceRecorderRef>,
) {
    let tick_started_at_ms = now_ms_f64();
    let mut tick_dispatch_guard = HostPresentTickGuard::new(
        render_loop_pending.clone(),
        render_loop_rerun_requested.clone(),
    );
    if let Some(dispatch_ms) = dispatch_requested_at_ms {
        let queue_delay_ms = (tick_started_at_ms - dispatch_ms).max(0.0);
        if queue_delay_ms >= HOST_TIMING_QUEUE_WARN_MS {
            record_native_video_timing_event_lazy(
                runtime_trace.as_ref(),
                "wgpu",
                "run_on_main_thread_delay",
                viewport_id,
                window.label(),
                || serde_json::json!({ "queueDelayMs": queue_delay_ms }),
            );
        }
    }

    let Ok(mut state) = renderer_state.lock() else {
        return;
    };
    let view_generation_before_tick = state.view_generation;
    let Ok(surface_size) = window.inner_size() else {
        return;
    };
    let surface_width = surface_size.width.max(1);
    let surface_height = surface_size.height.max(1);
    let size_changed = state.last_surface_size != Some((surface_width, surface_height));
    if state.renderer.is_none() {
        match pollster::block_on(
            super::wgpu_renderer::WgpuFrameRenderer::new_from_win32_window(
                window,
                surface_width,
                surface_height,
            ),
        ) {
            Ok(renderer) => {
                state.renderer = Some(renderer);
                state.view_generation = state.view_generation.saturating_add(1);
                state.latest_view_created_at_ms = Some(now_ms_f64());
                state.last_surface_size = Some((surface_width, surface_height));
                state.descriptor_upload_mode = None;
                state.descriptor_metal_import_count_total = 0;
                state.descriptor_cpu_upload_count_total = 0;
            }
            Err(error) => {
                if !state.init_failed_logged {
                    state.init_failed_logged = true;
                    log::warn!(
                        "[native_video][windows][wgpu] failed to create renderer for viewport={} window={} error={}",
                        viewport_id,
                        window.label(),
                        error
                    );
                }
                return;
            }
        }
    }
    let view_generation_changed = state.view_generation != view_generation_before_tick;

    let now_ms = now_ms_f64();
    let (take_outcome, take_slot_diag, take_telemetry_diag) = {
        let Ok(mut telemetry) = telemetry.lock() else {
            return;
        };
        telemetry.record_display_tick(now_ms);
        let Ok(mut frame_slot) = frame_slot.lock() else {
            return;
        };
        if view_generation_changed {
            frame_slot.begin_view_epoch();
        }
        let outcome = frame_slot.take_ready_frame(now_ms, &mut telemetry);
        let slot_diag = frame_slot.diagnostics_snapshot();
        let telemetry_diag = telemetry.diagnostics_snapshot();
        (outcome, slot_diag, telemetry_diag)
    };
    record_host_mailbox_take_decision(
        runtime_trace.as_ref(),
        "wgpu-windows",
        viewport_id,
        window.label(),
        &take_outcome,
        &take_slot_diag,
        &take_telemetry_diag,
    );
    if size_changed {
        state.last_surface_size = Some((surface_width, surface_height));
    }
    let cached_frame_for_repaint = state.latest_frame.clone();
    let has_cached_frame = cached_frame_for_repaint.is_some();
    let Some(renderer) = state.renderer.as_mut() else {
        return;
    };
    if size_changed {
        renderer.update_surface_size(surface_width, surface_height);
    }
    match take_outcome {
        ScheduledFrameTakeOutcome::Ready(frame) => {
            renderer.update_frame(frame.clone());
            if let Err(error) = renderer.render() {
                log::warn!(
                    "[native_video][windows][wgpu] render failed for viewport={} window={} error={}",
                    viewport_id,
                    window.label(),
                    error
                );
            } else {
                let descriptor_upload = renderer.descriptor_upload_telemetry();
                state.latest_frame = Some(frame);
                state.descriptor_upload_mode = descriptor_upload.last_mode;
                state.descriptor_metal_import_count_total =
                    descriptor_upload.metal_import_count_total;
                state.descriptor_cpu_upload_count_total = descriptor_upload.cpu_upload_count_total;
                if let Ok(mut telemetry) = telemetry.lock() {
                    telemetry.record_present(now_ms);
                }
            }
        }
        ScheduledFrameTakeOutcome::RetainedDisplayedFrame => {
            if size_changed {
                if let Some(frame) = cached_frame_for_repaint {
                    renderer.update_frame(frame);
                    let _ = renderer.render();
                }
            }
        }
        ScheduledFrameTakeOutcome::NoPendingFrame => {
            if !has_cached_frame && size_changed {
                let _ = renderer.render();
            }
        }
        ScheduledFrameTakeOutcome::DroppedStale { .. } => {}
    }
    let tick_total_ms = (now_ms_f64() - tick_started_at_ms).max(0.0);
    if tick_total_ms >= HOST_TIMING_TICK_WARN_MS {
        record_native_video_timing_event_lazy(
            runtime_trace.as_ref(),
            "wgpu-windows",
            "tick_total",
            viewport_id,
            window.label(),
            || serde_json::json!({ "totalMs": tick_total_ms }),
        );
    }
    if tick_dispatch_guard.finish_dispatch() {
        run_windows_wgpu_render_tick(
            window,
            viewport_id,
            renderer_state,
            frame_slot,
            telemetry,
            render_loop_pending,
            render_loop_rerun_requested,
            None,
            runtime_trace,
        );
    }
}

#[cfg(target_os = "macos")]
pub(super) struct MacOsWgpuPresenter {
    viewport_id: String,
    window_label: String,
    surface_id: Option<String>,
    app_handle: AppHandle,
    renderer_state: Arc<Mutex<MacOsWgpuState>>,
    frame_slot: Arc<Mutex<ScheduledFrameSlot>>,
    telemetry: Arc<Mutex<HostCadenceTelemetry>>,
    render_loop_stop: Arc<std::sync::atomic::AtomicBool>,
    render_loop_pending: Arc<std::sync::atomic::AtomicBool>,
    render_loop_rerun_requested: Arc<std::sync::atomic::AtomicBool>,
    display_link: Option<MacOsDisplayLinkHandle>,
    runtime_trace: Option<RuntimeTraceRecorderRef>,
    display_state: NativeVideoDisplayState,
}

#[cfg(target_os = "macos")]
impl MacOsWgpuPresenter {
    pub(super) fn new(
        viewport_id: &str,
        window_label: &str,
        app_handle: AppHandle,
        runtime_trace: Option<RuntimeTraceRecorderRef>,
    ) -> Self {
        Self {
            viewport_id: viewport_id.to_string(),
            window_label: window_label.to_string(),
            surface_id: None,
            app_handle,
            renderer_state: Arc::new(Mutex::new(MacOsWgpuState::default())),
            frame_slot: Arc::new(Mutex::new(ScheduledFrameSlot::default())),
            telemetry: Arc::new(Mutex::new(HostCadenceTelemetry::default())),
            render_loop_stop: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            render_loop_pending: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            render_loop_rerun_requested: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            display_link: None,
            runtime_trace,
            display_state: NativeVideoDisplayState::default(),
        }
    }

    fn ensure_render_loop(&mut self) {
        let Ok(mut state) = self.renderer_state.lock() else {
            return;
        };
        if state.render_loop_started {
            return;
        }
        state.render_loop_started = true;
        self.render_loop_stop.store(false, Ordering::Relaxed);
        if self.display_link.is_none() {
            if let Ok(display_link) = MacOsDisplayLinkHandle::start(
                self.viewport_id.clone(),
                self.window_label.clone(),
                self.app_handle.clone(),
                self.renderer_state.clone(),
                self.frame_slot.clone(),
                self.telemetry.clone(),
                self.render_loop_pending.clone(),
                self.render_loop_rerun_requested.clone(),
                self.runtime_trace.clone(),
            ) {
                self.display_link = Some(display_link);
                return;
            }
        }
        record_native_video_trace(
            "display_link_unavailable",
            serde_json::json!({
                "pipeline": "wgpu",
                "viewportId": self.viewport_id,
                "windowLabel": self.window_label,
            }),
        );
        log::warn!(
            "[native_video][wgpu] display link unavailable for viewport={}, fallback to 16ms loop",
            self.viewport_id
        );
        let viewport_id = self.viewport_id.clone();
        let window_label = self.window_label.clone();
        let app_handle = self.app_handle.clone();
        let renderer_state = self.renderer_state.clone();
        let frame_slot = self.frame_slot.clone();
        let telemetry = self.telemetry.clone();
        let render_loop_stop = self.render_loop_stop.clone();
        let render_loop_pending = self.render_loop_pending.clone();
        let render_loop_rerun_requested = self.render_loop_rerun_requested.clone();
        let runtime_trace = self.runtime_trace.clone();
        thread::Builder::new()
            .name(format!("XbxWgpuRenderLoop-{viewport_id}"))
            .spawn(move || {
                while !render_loop_stop.load(Ordering::Relaxed) {
                    let tick = telemetry
                        .lock()
                        .map(|state| state.present_tick_sleep_duration())
                        .unwrap_or(Duration::from_millis(16));
                    thread::sleep(tick);
                    if !request_host_present_tick_dispatch(
                        &render_loop_pending,
                        &render_loop_rerun_requested,
                    ) {
                        continue;
                    }
                    let Some(window) = app_handle.get_window(&window_label) else {
                        clear_host_present_tick_dispatch(
                            &render_loop_pending,
                            &render_loop_rerun_requested,
                        );
                        continue;
                    };
                    if dispatch_wgpu_render_tick_on_main_thread(
                        &window,
                        &app_handle,
                        &viewport_id,
                        &renderer_state,
                        &frame_slot,
                        &telemetry,
                        &render_loop_pending,
                        &render_loop_rerun_requested,
                        runtime_trace.clone(),
                    )
                    .is_err()
                    {
                        clear_host_present_tick_dispatch(
                            &render_loop_pending,
                            &render_loop_rerun_requested,
                        );
                    }
                }
            })
            .expect("Failed to spawn macOS wgpu render loop");
    }

    fn should_drop_submitted_frame(&self, frame: &XbxEngineRenderFrame, now_ms: f64) -> bool {
        let Ok(telemetry) = self.telemetry.lock() else {
            return false;
        };
        now_ms - frame.rendered_at_ms > telemetry.frame_age_budget_ms()
    }

    fn request_immediate_render_tick(&self) {
        if !request_host_present_tick_dispatch(
            &self.render_loop_pending,
            &self.render_loop_rerun_requested,
        ) {
            return;
        }
        let Some(window) = self.app_handle.get_window(&self.window_label) else {
            clear_host_present_tick_dispatch(
                &self.render_loop_pending,
                &self.render_loop_rerun_requested,
            );
            return;
        };
        if dispatch_wgpu_render_tick_on_main_thread(
            &window,
            &self.app_handle,
            &self.viewport_id,
            &self.renderer_state,
            &self.frame_slot,
            &self.telemetry,
            &self.render_loop_pending,
            &self.render_loop_rerun_requested,
            self.runtime_trace.clone(),
        )
        .is_err()
        {
            clear_host_present_tick_dispatch(
                &self.render_loop_pending,
                &self.render_loop_rerun_requested,
            );
        }
    }
}

#[cfg(target_os = "macos")]
impl NativeVideoPresenter for MacOsWgpuPresenter {
    fn kind(&self) -> NativeVideoPresenterKind {
        NativeVideoPresenterKind::Wgpu
    }

    fn attach(&mut self, surface_id: Option<&str>) {
        self.begin_media_epoch();
        self.surface_id = surface_id.map(str::to_string);
        self.ensure_render_loop();
    }

    fn begin_media_epoch(&mut self) {
        if let Ok(mut state) = self.renderer_state.lock() {
            state.latest_frame = None;
            state.last_presented_frame_seq = None;
            state.last_surface_size = None;
            state.descriptor_upload_mode = None;
            state.descriptor_metal_import_count_total = 0;
            state.descriptor_cpu_upload_count_total = 0;
        }
        if let Ok(mut frame_slot) = self.frame_slot.lock() {
            frame_slot.begin_media_epoch();
        }
        if let Ok(mut telemetry) = self.telemetry.lock() {
            telemetry.reset_frame_slot();
        }
        clear_host_present_tick_dispatch(
            &self.render_loop_pending,
            &self.render_loop_rerun_requested,
        );
    }

    fn reset_mailbox_for_host_stall_recovery(&mut self) -> bool {
        self.begin_media_epoch();
        self.render_loop_stop.store(false, Ordering::Relaxed);
        self.ensure_render_loop();
        true
    }

    fn apply_display_state(&mut self, state: &NativeVideoDisplayState) {
        self.display_state = state.clone();
    }

    fn present(&mut self, surface_id: Option<&str>, frame: &XbxEngineRenderFrame) -> bool {
        self.surface_id = surface_id
            .map(str::to_string)
            .or_else(|| self.surface_id.clone());
        self.ensure_render_loop();
        let now_ms = now_ms_f64();
        if self.should_drop_submitted_frame(frame, now_ms) {
            if let Ok(mut telemetry) = self.telemetry.lock() {
                telemetry.present_enqueue_count_total =
                    telemetry.present_enqueue_count_total.saturating_add(1);
                telemetry.record_stale_frame_drop(frame, now_ms, "submittedFrameStale", 0);
            }
            record_native_video_timing_event_lazy(
                self.runtime_trace.as_ref(),
                "wgpu",
                "hostMailboxRejected",
                &self.viewport_id,
                &self.window_label,
                || {
                    serde_json::json!({
                        "outcome": "stale",
                        "frameSeq": frame.frame_seq,
                        "frameAgeMs": (now_ms - frame.rendered_at_ms).max(0.0),
                    })
                },
            );
            log::debug!(
                "[native_video][wgpu] reject stale submitted frame viewport={} window={} frame_seq={} age_ms={:.2}",
                self.viewport_id,
                self.window_label,
                frame.frame_seq,
                now_ms - frame.rendered_at_ms
            );
            return false;
        }
        let Ok(mut telemetry) = self.telemetry.lock() else {
            record_native_video_timing_event_lazy(
                self.runtime_trace.as_ref(),
                "wgpu",
                "hostMailboxUpdateFailed",
                &self.viewport_id,
                &self.window_label,
                || serde_json::json!({ "reason": "telemetryLockFailed", "frameSeq": frame.frame_seq }),
            );
            return false;
        };
        let previous_submit_time_ms = telemetry.latest_submit_time_ms;
        let submit_gap_ms = telemetry.record_submit(now_ms);
        let no_pending_streak_before_submit = telemetry.no_pending_streak;
        let should_warn_submit_gap =
            submit_gap_ms.is_some_and(|gap_ms| telemetry.should_warn_submit_gap(gap_ms));
        telemetry.present_enqueue_count_total =
            telemetry.present_enqueue_count_total.saturating_add(1);
        let telemetry_diag = telemetry.diagnostics_snapshot();
        let Ok(mut frame_slot) = self.frame_slot.lock() else {
            record_native_video_timing_event_lazy(
                self.runtime_trace.as_ref(),
                "wgpu",
                "hostMailboxUpdateFailed",
                &self.viewport_id,
                &self.window_label,
                || serde_json::json!({ "reason": "frameSlotLockFailed", "frameSeq": frame.frame_seq }),
            );
            return false;
        };
        match frame_slot.submit_frame(frame, now_ms, &mut telemetry) {
            super::scheduling::ScheduledFrameSubmitOutcome::Accepted {
                frame_seq,
                overwrote_pending,
                replaced_frame_seq,
                frame_age_ms,
                frame_age_budget_ms,
            } => {
                let slot_diag = frame_slot.diagnostics_snapshot();
                record_native_video_timing_event_lazy(
                    self.runtime_trace.as_ref(),
                    "wgpu",
                    "hostMailboxAccepted",
                    &self.viewport_id,
                    &self.window_label,
                    || {
                        serde_json::json!({
                            "outcome": "accepted",
                            "frameSeq": frame_seq,
                            "frameRtpTimestamp": frame.rtp_timestamp,
                            "frameRecoveryEpoch": frame.recovery_epoch_tag,
                            "frameRecoveryOwnerRtpTimestamp": frame.recovery_owner_rtp_timestamp,
                            "frameRecoveryDisposition": frame.frame_recovery_disposition,
                            "frameIsKeyframe": frame.is_keyframe,
                            "frameAgeMs": frame_age_ms,
                            "frameAgeBudgetMs": frame_age_budget_ms,
                            "submitGapMs": submit_gap_ms,
                            "noPendingStreakBeforeSubmit": no_pending_streak_before_submit,
                            "overwrotePending": overwrote_pending,
                            "replacedFrameSeq": replaced_frame_seq,
                            "displayedFrameSeq": slot_diag.displayed_frame_seq,
                            "pendingFrameSeq": pending_frame_seq(&slot_diag),
                            "hasPendingFrame": slot_diag.pending_queue_depth > 0,
                            "pendingFrameSeqs": slot_diag.pending_frame_seqs,
                            "lastPresentedFrameSeq": slot_diag.last_presented_frame_seq,
                            "queueDepth": slot_diag.queue_depth,
                            "pendingQueueDepth": slot_diag.pending_queue_depth,
                            "hostDisplayTickEpoch": telemetry_diag.display_tick_epoch,
                            "hostFramePresentEpoch": telemetry_diag.present_epoch,
                            "hostCadencePhase": telemetry_diag.cadence_phase.as_str(),
                        })
                    },
                );
                if let Some(gap_ms) = submit_gap_ms.filter(|_| should_warn_submit_gap) {
                    record_native_video_timing_event_lazy(
                        self.runtime_trace.as_ref(),
                        "wgpu",
                        "hostMailboxSubmitGap",
                        &self.viewport_id,
                        &self.window_label,
                        || {
                            serde_json::json!({
                                "frameSeq": frame_seq,
                                "submitGapMs": gap_ms,
                                "frameAgeMs": frame_age_ms,
                                "frameAgeBudgetMs": frame_age_budget_ms,
                                "noPendingStreakBeforeSubmit": no_pending_streak_before_submit,
                                "overwrotePending": overwrote_pending,
                                "replacedFrameSeq": replaced_frame_seq,
                                "displayedFrameSeq": slot_diag.displayed_frame_seq,
                                "pendingFrameSeq": pending_frame_seq(&slot_diag),
                                "hasPendingFrame": slot_diag.pending_queue_depth > 0,
                                "pendingFrameSeqs": slot_diag.pending_frame_seqs,
                                "queueDepth": slot_diag.queue_depth,
                                "pendingQueueDepth": slot_diag.pending_queue_depth,
                                "hostDisplayTickEpoch": telemetry_diag.display_tick_epoch,
                                "hostFramePresentEpoch": telemetry_diag.present_epoch,
                            })
                        },
                    );
                }
                self.request_immediate_render_tick();
                true
            }
            super::scheduling::ScheduledFrameSubmitOutcome::DroppedStale {
                frame_seq,
                frame_age_ms,
                frame_age_budget_ms,
            } => {
                let slot_diag = frame_slot.diagnostics_snapshot();
                record_native_video_timing_event_lazy(
                    self.runtime_trace.as_ref(),
                    "wgpu",
                    "hostMailboxRejected",
                    &self.viewport_id,
                    &self.window_label,
                    || {
                        serde_json::json!({
                            "outcome": "stale",
                            "frameSeq": frame_seq,
                            "frameRtpTimestamp": frame.rtp_timestamp,
                            "frameRecoveryEpoch": frame.recovery_epoch_tag,
                            "frameRecoveryOwnerRtpTimestamp": frame.recovery_owner_rtp_timestamp,
                            "frameRecoveryDisposition": frame.frame_recovery_disposition,
                            "frameIsKeyframe": frame.is_keyframe,
                            "frameAgeMs": frame_age_ms,
                            "frameAgeBudgetMs": frame_age_budget_ms,
                            "submitGapMs": submit_gap_ms,
                            "noPendingStreakBeforeSubmit": no_pending_streak_before_submit,
                            "displayedFrameSeq": slot_diag.displayed_frame_seq,
                            "pendingFrameSeq": pending_frame_seq(&slot_diag),
                            "hasPendingFrame": slot_diag.pending_queue_depth > 0,
                            "pendingFrameSeqs": slot_diag.pending_frame_seqs,
                            "queueDepth": slot_diag.queue_depth,
                            "pendingQueueDepth": slot_diag.pending_queue_depth,
                            "hostDisplayTickEpoch": telemetry_diag.display_tick_epoch,
                            "hostFramePresentEpoch": telemetry_diag.present_epoch,
                            "hostCadencePhase": telemetry_diag.cadence_phase.as_str(),
                        })
                    },
                );
                telemetry.latest_submit_time_ms = previous_submit_time_ms;
                false
            }
            super::scheduling::ScheduledFrameSubmitOutcome::RejectedAlreadyPresented {
                frame_seq,
                last_presented_frame_seq,
            } => {
                let slot_diag = frame_slot.diagnostics_snapshot();
                record_native_video_timing_event_lazy(
                    self.runtime_trace.as_ref(),
                    "wgpu",
                    "hostMailboxRejected",
                    &self.viewport_id,
                    &self.window_label,
                    || {
                        serde_json::json!({
                            "outcome": "already_presented",
                            "frameSeq": frame_seq,
                            "frameRtpTimestamp": frame.rtp_timestamp,
                            "frameRecoveryEpoch": frame.recovery_epoch_tag,
                            "frameRecoveryOwnerRtpTimestamp": frame.recovery_owner_rtp_timestamp,
                            "frameRecoveryDisposition": frame.frame_recovery_disposition,
                            "frameIsKeyframe": frame.is_keyframe,
                            "lastPresentedFrameSeq": last_presented_frame_seq,
                            "submitGapMs": submit_gap_ms,
                            "noPendingStreakBeforeSubmit": no_pending_streak_before_submit,
                            "displayedFrameSeq": slot_diag.displayed_frame_seq,
                            "pendingFrameSeq": pending_frame_seq(&slot_diag),
                            "hasPendingFrame": slot_diag.pending_queue_depth > 0,
                            "pendingFrameSeqs": slot_diag.pending_frame_seqs,
                            "queueDepth": slot_diag.queue_depth,
                            "pendingQueueDepth": slot_diag.pending_queue_depth,
                            "hostDisplayTickEpoch": telemetry_diag.display_tick_epoch,
                            "hostFramePresentEpoch": telemetry_diag.present_epoch,
                            "hostCadencePhase": telemetry_diag.cadence_phase.as_str(),
                        })
                    },
                );
                telemetry.latest_submit_time_ms = previous_submit_time_ms;
                false
            }
        }
    }

    fn detach(&mut self) {
        self.surface_id = None;
        self.render_loop_stop.store(true, Ordering::Relaxed);
        if let Some(display_link) = self.display_link.take() {
            display_link.stop();
        }
        let renderer_state = self.renderer_state.clone();
        let frame_slot = self.frame_slot.clone();
        let telemetry = self.telemetry.clone();
        let window_label = self.window_label.clone();
        let app_handle = self.app_handle.clone();
        if let Some(window) = app_handle.get_window(&window_label) {
            let _ = window.run_on_main_thread(move || {
                if let Ok(mut frame_slot) = frame_slot.lock() {
                    frame_slot.reset();
                }
                if let Ok(mut state) = renderer_state.lock() {
                    state.renderer = None;
                    state.latest_frame = None;
                    state.last_presented_frame_seq = None;
                    state.last_surface_size = None;
                    state.descriptor_upload_mode = None;
                    state.descriptor_metal_import_count_total = 0;
                    state.descriptor_cpu_upload_count_total = 0;
                    if let Some(host_view_ptr) = state.host_view_ptr.take() {
                        if state.host_view_managed {
                            drop_wgpu_host_view(host_view_ptr);
                        }
                    }
                    state.host_view_managed = false;
                    state.render_loop_started = false;
                }
                if let Ok(mut telemetry) = telemetry.lock() {
                    telemetry.reset_frame_slot();
                }
            });
        }
    }

    fn apply_viewport_diagnostics(&self, viewport: &mut NativeVideoViewportState) {
        let Ok(telemetry) = self.telemetry.lock() else {
            return;
        };
        let slot_diag = self
            .frame_slot
            .lock()
            .ok()
            .map(|frame_slot| frame_slot.diagnostics_snapshot());
        let renderer_state = self.renderer_state.lock().ok();
        apply_host_mailbox_viewport_diagnostics(
            viewport,
            &telemetry,
            slot_diag.as_ref(),
            renderer_state
                .as_ref()
                .and_then(|state| state.latest_host_view_created_at_ms),
            renderer_state
                .as_ref()
                .and_then(|state| state.descriptor_upload_mode.clone()),
            renderer_state
                .as_ref()
                .map(|state| state.descriptor_metal_import_count_total)
                .unwrap_or_default(),
            renderer_state
                .as_ref()
                .map(|state| state.descriptor_cpu_upload_count_total)
                .unwrap_or_default(),
        );
    }

    fn take_pending_frame_drops(&mut self) -> Vec<xbxengine::XbxEngineHostVideoFrameDropEvent> {
        self.telemetry
            .lock()
            .map(|mut telemetry| telemetry.take_pending_frame_drops())
            .unwrap_or_default()
    }
}

#[cfg(target_os = "macos")]
pub(super) struct MacOsVideoPresenter {
    viewport_id: String,
    window_label: String,
    surface_id: Option<String>,
    app_handle: AppHandle,
    layer_state: Arc<Mutex<MacOsLayerState>>,
    frame_slot: Arc<Mutex<ScheduledFrameSlot>>,
    telemetry: Arc<Mutex<HostCadenceTelemetry>>,
    render_loop_stop: Arc<std::sync::atomic::AtomicBool>,
    render_loop_pending: Arc<std::sync::atomic::AtomicBool>,
    render_loop_rerun_requested: Arc<std::sync::atomic::AtomicBool>,
    display_link: Option<MacOsLayerDisplayLinkHandle>,
    runtime_trace: Option<RuntimeTraceRecorderRef>,
    display_state: NativeVideoDisplayState,
}

#[cfg(target_os = "macos")]
impl MacOsVideoPresenter {
    pub(super) fn new(
        viewport_id: &str,
        window_label: &str,
        app_handle: AppHandle,
        runtime_trace: Option<RuntimeTraceRecorderRef>,
    ) -> Self {
        Self {
            viewport_id: viewport_id.to_string(),
            window_label: window_label.to_string(),
            surface_id: None,
            app_handle,
            layer_state: Arc::new(Mutex::new(MacOsLayerState::default())),
            frame_slot: Arc::new(Mutex::new(ScheduledFrameSlot::default())),
            telemetry: Arc::new(Mutex::new(HostCadenceTelemetry::default())),
            render_loop_stop: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            render_loop_pending: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            render_loop_rerun_requested: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            display_link: None,
            runtime_trace,
            display_state: NativeVideoDisplayState::default(),
        }
    }

    fn ensure_layer_ready_on_main_thread(&self) {
        let Some(window) = self.app_handle.get_window(&self.window_label) else {
            return;
        };
        let layer_state = self.layer_state.clone();
        let frame_slot = self.frame_slot.clone();
        let viewport_id = self.viewport_id.clone();
        let window_label = self.window_label.clone();
        let runtime_trace = self.runtime_trace.clone();
        let window_for_task = window.clone();
        let _ = window.run_on_main_thread(move || {
            let Ok(mut state) = layer_state.lock() else {
                record_native_video_timing_event_lazy(
                    runtime_trace.as_ref(),
                    "layer",
                    "display_layer_init_failed",
                    &viewport_id,
                    &window_label,
                    || serde_json::json!({ "reason": "layerStateLockFailed" }),
                );
                return;
            };
            let previous_generation = state.display_layer_generation;
            if ensure_display_layer(&window_for_task, &mut state).is_none() {
                record_native_video_timing_event_lazy(
                    runtime_trace.as_ref(),
                    "layer",
                    "display_layer_init_failed",
                    &viewport_id,
                    &window_label,
                    || serde_json::json!({ "reason": "displayLayerUnavailable" }),
                );
            } else if state.display_layer_generation != previous_generation {
                if let Ok(mut slot) = frame_slot.lock() {
                    slot.begin_view_epoch();
                }
            }
        });
    }

    fn ensure_render_loop(&mut self) {
        let Ok(mut frame_slot) = self.frame_slot.lock() else {
            return;
        };
        if frame_slot.render_loop_started {
            return;
        }
        frame_slot.render_loop_started = true;
        self.render_loop_stop.store(false, Ordering::Relaxed);
        self.ensure_layer_ready_on_main_thread();
        if self.display_link.is_none() {
            if let Ok(display_link) = MacOsLayerDisplayLinkHandle::start(
                self.viewport_id.clone(),
                self.window_label.clone(),
                self.layer_state.clone(),
                self.frame_slot.clone(),
                self.telemetry.clone(),
                self.render_loop_pending.clone(),
                self.render_loop_rerun_requested.clone(),
                self.runtime_trace.clone(),
            ) {
                self.display_link = Some(display_link);
                return;
            }
        }
        record_native_video_trace(
            "display_link_unavailable",
            serde_json::json!({
                "pipeline": "layer",
                "viewportId": self.viewport_id,
                "windowLabel": self.window_label,
            }),
        );
        log::warn!(
            "[native_video][layer] display link unavailable for viewport={}, fallback to 16ms loop",
            self.viewport_id
        );
        let viewport_id = self.viewport_id.clone();
        let window_label = self.window_label.clone();
        let layer_state = self.layer_state.clone();
        let frame_slot = self.frame_slot.clone();
        let telemetry = self.telemetry.clone();
        let render_loop_stop = self.render_loop_stop.clone();
        let render_loop_pending = self.render_loop_pending.clone();
        let render_loop_rerun_requested = self.render_loop_rerun_requested.clone();
        let runtime_trace = self.runtime_trace.clone();
        thread::Builder::new()
            .name(format!("XbxLayerRenderLoop-{viewport_id}"))
            .spawn(move || {
                while !render_loop_stop.load(Ordering::Relaxed) {
                    let tick = telemetry
                        .lock()
                        .map(|state| state.present_tick_sleep_duration())
                        .unwrap_or(Duration::from_millis(16));
                    thread::sleep(tick);
                    if !request_host_present_tick_dispatch(
                        &render_loop_pending,
                        &render_loop_rerun_requested,
                    ) {
                        continue;
                    }
                    run_layer_present_tick(
                        &viewport_id,
                        &window_label,
                        &layer_state,
                        &frame_slot,
                        &telemetry,
                        &render_loop_pending,
                        &render_loop_rerun_requested,
                        runtime_trace.clone(),
                    );
                }
            })
            .expect("Failed to spawn macOS layer render loop");
    }

    fn should_drop_submitted_frame(&self, frame: &XbxEngineRenderFrame, now_ms: f64) -> bool {
        let Ok(telemetry) = self.telemetry.lock() else {
            return false;
        };
        now_ms - frame.rendered_at_ms > telemetry.frame_age_budget_ms()
    }

    fn request_immediate_present_tick(&self) {
        if !request_host_present_tick_dispatch(
            &self.render_loop_pending,
            &self.render_loop_rerun_requested,
        ) {
            return;
        }
        let Some(window) = self.app_handle.get_window(&self.window_label) else {
            clear_host_present_tick_dispatch(
                &self.render_loop_pending,
                &self.render_loop_rerun_requested,
            );
            return;
        };
        let viewport_id = self.viewport_id.clone();
        let window_label = self.window_label.clone();
        let layer_state = self.layer_state.clone();
        let frame_slot = self.frame_slot.clone();
        let telemetry = self.telemetry.clone();
        let render_loop_pending = self.render_loop_pending.clone();
        let render_loop_rerun_requested = self.render_loop_rerun_requested.clone();
        let runtime_trace = self.runtime_trace.clone();
        let _ = window.run_on_main_thread(move || {
            run_layer_present_tick(
                &viewport_id,
                &window_label,
                &layer_state,
                &frame_slot,
                &telemetry,
                &render_loop_pending,
                &render_loop_rerun_requested,
                runtime_trace,
            );
        });
    }
}

#[cfg(target_os = "macos")]
impl NativeVideoPresenter for MacOsVideoPresenter {
    fn kind(&self) -> NativeVideoPresenterKind {
        NativeVideoPresenterKind::PlatformNative
    }

    fn attach(&mut self, surface_id: Option<&str>) {
        self.begin_media_epoch();
        self.surface_id = surface_id.map(str::to_string);
        self.ensure_render_loop();
    }

    fn begin_media_epoch(&mut self) {
        if let Ok(mut frame_slot) = self.frame_slot.lock() {
            frame_slot.begin_media_epoch();
        }
        if let Ok(mut telemetry) = self.telemetry.lock() {
            telemetry.reset_frame_slot();
        }
        clear_host_present_tick_dispatch(
            &self.render_loop_pending,
            &self.render_loop_rerun_requested,
        );
    }

    fn reset_mailbox_for_host_stall_recovery(&mut self) -> bool {
        self.begin_media_epoch();
        self.render_loop_stop.store(false, Ordering::Relaxed);
        self.ensure_render_loop();
        true
    }

    fn apply_display_state(&mut self, state: &NativeVideoDisplayState) {
        self.display_state = state.clone();
        if let Ok(mut layer_state) = self.layer_state.lock() {
            layer_state.display_state = state.clone();
            layer_state.layout_dirty = true;
        }
        self.ensure_layer_ready_on_main_thread();
    }

    fn present(&mut self, surface_id: Option<&str>, frame: &XbxEngineRenderFrame) -> bool {
        self.surface_id = surface_id
            .map(str::to_string)
            .or_else(|| self.surface_id.clone());
        self.ensure_render_loop();
        let source_size = Some((frame.width, frame.height));
        let should_refresh_layout = self
            .layer_state
            .lock()
            .ok()
            .map(|mut layer_state| {
                let changed = layer_state.latest_source_size != source_size;
                layer_state.latest_source_size = source_size;
                if changed {
                    layer_state.layout_dirty = true;
                }
                changed
            })
            .unwrap_or(false);
        if should_refresh_layout {
            self.ensure_layer_ready_on_main_thread();
        }
        if !frame_has_cv_pixelbuffer(frame) {
            record_native_video_timing_event_lazy(
                self.runtime_trace.as_ref(),
                "layer",
                "hostMailboxRejected",
                &self.viewport_id,
                &self.window_label,
                || {
                    serde_json::json!({
                        "outcome": "rejected_non_cv_pixelbuffer",
                        "frameSeq": frame.frame_seq,
                    })
                },
            );
            return false;
        }
        let now_ms = now_ms_f64();
        if self.should_drop_submitted_frame(frame, now_ms) {
            if let Ok(mut telemetry) = self.telemetry.lock() {
                telemetry.present_enqueue_count_total =
                    telemetry.present_enqueue_count_total.saturating_add(1);
                telemetry.record_stale_frame_drop(frame, now_ms, "submittedFrameStale", 0);
            }
            record_native_video_timing_event_lazy(
                self.runtime_trace.as_ref(),
                "layer",
                "hostMailboxRejected",
                &self.viewport_id,
                &self.window_label,
                || {
                    serde_json::json!({
                        "outcome": "stale",
                        "frameSeq": frame.frame_seq,
                        "frameAgeMs": (now_ms - frame.rendered_at_ms).max(0.0),
                    })
                },
            );
            return false;
        }
        let Ok(mut telemetry) = self.telemetry.lock() else {
            record_native_video_timing_event_lazy(
                self.runtime_trace.as_ref(),
                "layer",
                "hostMailboxUpdateFailed",
                &self.viewport_id,
                &self.window_label,
                || {
                    serde_json::json!({
                        "reason": "telemetryLockFailed",
                        "frameSeq": frame.frame_seq,
                    })
                },
            );
            return false;
        };
        let previous_submit_time_ms = telemetry.latest_submit_time_ms;
        let submit_gap_ms = telemetry.record_submit(now_ms);
        let no_pending_streak_before_submit = telemetry.no_pending_streak;
        let should_warn_submit_gap =
            submit_gap_ms.is_some_and(|gap_ms| telemetry.should_warn_submit_gap(gap_ms));
        telemetry.present_enqueue_count_total =
            telemetry.present_enqueue_count_total.saturating_add(1);
        let telemetry_diag = telemetry.diagnostics_snapshot();
        let Ok(mut frame_slot) = self.frame_slot.lock() else {
            record_native_video_timing_event_lazy(
                self.runtime_trace.as_ref(),
                "layer",
                "hostMailboxUpdateFailed",
                &self.viewport_id,
                &self.window_label,
                || {
                    serde_json::json!({
                        "reason": "frameSlotLockFailed",
                        "frameSeq": frame.frame_seq,
                    })
                },
            );
            return false;
        };
        match frame_slot.submit_frame(frame, now_ms, &mut telemetry) {
            super::scheduling::ScheduledFrameSubmitOutcome::Accepted {
                frame_seq,
                overwrote_pending,
                replaced_frame_seq,
                frame_age_ms,
                frame_age_budget_ms,
            } => {
                let slot_diag = frame_slot.diagnostics_snapshot();
                record_native_video_timing_event_lazy(
                    self.runtime_trace.as_ref(),
                    "layer",
                    "hostMailboxAccepted",
                    &self.viewport_id,
                    &self.window_label,
                    || {
                        serde_json::json!({
                            "outcome": "accepted",
                            "frameSeq": frame_seq,
                            "frameRtpTimestamp": frame.rtp_timestamp,
                            "frameRecoveryEpoch": frame.recovery_epoch_tag,
                            "frameRecoveryOwnerRtpTimestamp": frame.recovery_owner_rtp_timestamp,
                            "frameRecoveryDisposition": frame.frame_recovery_disposition,
                            "frameIsKeyframe": frame.is_keyframe,
                            "frameAgeMs": frame_age_ms,
                            "frameAgeBudgetMs": frame_age_budget_ms,
                            "submitGapMs": submit_gap_ms,
                            "noPendingStreakBeforeSubmit": no_pending_streak_before_submit,
                            "overwrotePending": overwrote_pending,
                            "replacedFrameSeq": replaced_frame_seq,
                            "displayedFrameSeq": slot_diag.displayed_frame_seq,
                            "pendingFrameSeq": pending_frame_seq(&slot_diag),
                            "hasPendingFrame": slot_diag.pending_queue_depth > 0,
                            "pendingFrameSeqs": slot_diag.pending_frame_seqs,
                            "lastPresentedFrameSeq": slot_diag.last_presented_frame_seq,
                            "queueDepth": slot_diag.queue_depth,
                            "pendingQueueDepth": slot_diag.pending_queue_depth,
                            "hostDisplayTickEpoch": telemetry_diag.display_tick_epoch,
                            "hostFramePresentEpoch": telemetry_diag.present_epoch,
                            "hostCadencePhase": telemetry_diag.cadence_phase.as_str(),
                        })
                    },
                );
                if let Some(gap_ms) = submit_gap_ms.filter(|_| should_warn_submit_gap) {
                    // 只在供帧节奏明显掉速时额外留痕，避免健康阶段刷屏。
                    record_native_video_timing_event_lazy(
                        self.runtime_trace.as_ref(),
                        "layer",
                        "hostMailboxSubmitGap",
                        &self.viewport_id,
                        &self.window_label,
                        || {
                            serde_json::json!({
                                "frameSeq": frame_seq,
                                "submitGapMs": gap_ms,
                                "frameAgeMs": frame_age_ms,
                                "frameAgeBudgetMs": frame_age_budget_ms,
                                "noPendingStreakBeforeSubmit": no_pending_streak_before_submit,
                                "overwrotePending": overwrote_pending,
                                "replacedFrameSeq": replaced_frame_seq,
                                "displayedFrameSeq": slot_diag.displayed_frame_seq,
                                "pendingFrameSeq": pending_frame_seq(&slot_diag),
                                "hasPendingFrame": slot_diag.pending_queue_depth > 0,
                                "pendingFrameSeqs": slot_diag.pending_frame_seqs,
                                "queueDepth": slot_diag.queue_depth,
                                "pendingQueueDepth": slot_diag.pending_queue_depth,
                                "hostDisplayTickEpoch": telemetry_diag.display_tick_epoch,
                                "hostFramePresentEpoch": telemetry_diag.present_epoch,
                            })
                        },
                    );
                }
                self.request_immediate_present_tick();
                true
            }
            super::scheduling::ScheduledFrameSubmitOutcome::DroppedStale {
                frame_seq,
                frame_age_ms,
                frame_age_budget_ms,
            } => {
                let slot_diag = frame_slot.diagnostics_snapshot();
                record_native_video_timing_event_lazy(
                    self.runtime_trace.as_ref(),
                    "layer",
                    "hostMailboxRejected",
                    &self.viewport_id,
                    &self.window_label,
                    || {
                        serde_json::json!({
                            "outcome": "stale",
                            "frameSeq": frame_seq,
                            "frameRtpTimestamp": frame.rtp_timestamp,
                            "frameRecoveryEpoch": frame.recovery_epoch_tag,
                            "frameRecoveryOwnerRtpTimestamp": frame.recovery_owner_rtp_timestamp,
                            "frameRecoveryDisposition": frame.frame_recovery_disposition,
                            "frameIsKeyframe": frame.is_keyframe,
                            "frameAgeMs": frame_age_ms,
                            "frameAgeBudgetMs": frame_age_budget_ms,
                            "submitGapMs": submit_gap_ms,
                            "noPendingStreakBeforeSubmit": no_pending_streak_before_submit,
                            "displayedFrameSeq": slot_diag.displayed_frame_seq,
                            "pendingFrameSeq": pending_frame_seq(&slot_diag),
                            "hasPendingFrame": slot_diag.pending_queue_depth > 0,
                            "pendingFrameSeqs": slot_diag.pending_frame_seqs,
                            "queueDepth": slot_diag.queue_depth,
                            "pendingQueueDepth": slot_diag.pending_queue_depth,
                            "hostDisplayTickEpoch": telemetry_diag.display_tick_epoch,
                            "hostFramePresentEpoch": telemetry_diag.present_epoch,
                            "hostCadencePhase": telemetry_diag.cadence_phase.as_str(),
                        })
                    },
                );
                telemetry.latest_submit_time_ms = previous_submit_time_ms;
                false
            }
            super::scheduling::ScheduledFrameSubmitOutcome::RejectedAlreadyPresented {
                frame_seq,
                last_presented_frame_seq,
            } => {
                let slot_diag = frame_slot.diagnostics_snapshot();
                record_native_video_timing_event_lazy(
                    self.runtime_trace.as_ref(),
                    "layer",
                    "hostMailboxRejected",
                    &self.viewport_id,
                    &self.window_label,
                    || {
                        serde_json::json!({
                            "outcome": "already_presented",
                            "frameSeq": frame_seq,
                            "frameRtpTimestamp": frame.rtp_timestamp,
                            "frameRecoveryEpoch": frame.recovery_epoch_tag,
                            "frameRecoveryOwnerRtpTimestamp": frame.recovery_owner_rtp_timestamp,
                            "frameRecoveryDisposition": frame.frame_recovery_disposition,
                            "frameIsKeyframe": frame.is_keyframe,
                            "lastPresentedFrameSeq": last_presented_frame_seq,
                            "submitGapMs": submit_gap_ms,
                            "noPendingStreakBeforeSubmit": no_pending_streak_before_submit,
                            "displayedFrameSeq": slot_diag.displayed_frame_seq,
                            "pendingFrameSeq": pending_frame_seq(&slot_diag),
                            "hasPendingFrame": slot_diag.pending_queue_depth > 0,
                            "pendingFrameSeqs": slot_diag.pending_frame_seqs,
                            "queueDepth": slot_diag.queue_depth,
                            "pendingQueueDepth": slot_diag.pending_queue_depth,
                            "hostDisplayTickEpoch": telemetry_diag.display_tick_epoch,
                            "hostFramePresentEpoch": telemetry_diag.present_epoch,
                            "hostCadencePhase": telemetry_diag.cadence_phase.as_str(),
                        })
                    },
                );
                telemetry.latest_submit_time_ms = previous_submit_time_ms;
                false
            }
        }
    }

    fn detach(&mut self) {
        self.surface_id = None;
        self.render_loop_stop.store(true, Ordering::Relaxed);
        if let Some(display_link) = self.display_link.take() {
            display_link.stop();
        }
        let window_label = self.window_label.clone();
        let app_handle = self.app_handle.clone();
        let layer_state = self.layer_state.clone();
        let frame_slot = self.frame_slot.clone();
        let telemetry = self.telemetry.clone();
        if let Some(window) = app_handle.get_window(&window_label) {
            let window_for_task = window.clone();
            let _ = window.run_on_main_thread(move || {
                if let Ok(mut frame_slot) = frame_slot.lock() {
                    frame_slot.reset();
                }
                if let Ok(mut telemetry) = telemetry.lock() {
                    telemetry.reset_frame_slot();
                }
                let mut state = match layer_state.lock() {
                    Ok(state) => state,
                    Err(_) => return,
                };
                drop_display_layer(&window_for_task, &mut state);
            });
        }
    }

    fn apply_viewport_diagnostics(&self, viewport: &mut NativeVideoViewportState) {
        let Ok(telemetry) = self.telemetry.lock() else {
            return;
        };
        let slot_diag = self
            .frame_slot
            .lock()
            .ok()
            .map(|frame_slot| frame_slot.diagnostics_snapshot());
        apply_host_mailbox_viewport_diagnostics(
            viewport,
            &telemetry,
            slot_diag.as_ref(),
            self.layer_state
                .lock()
                .ok()
                .and_then(|state| state.latest_display_layer_created_at_ms),
            None,
            0,
            0,
        );
    }

    fn take_pending_frame_drops(&mut self) -> Vec<xbxengine::XbxEngineHostVideoFrameDropEvent> {
        self.telemetry
            .lock()
            .map(|mut telemetry| telemetry.take_pending_frame_drops())
            .unwrap_or_default()
    }
}

pub(super) fn resolve_present_kind(frame: &XbxEngineRenderFrame) -> String {
    match &frame.pixel_data {
        XbxEngineRenderPixelData::Descriptor { .. } => {
            if frame_has_cv_pixelbuffer(frame) {
                "cvpixelbuffer".to_string()
            } else if frame_has_windows_d3d11_texture(frame) {
                "d3d11-texture".to_string()
            } else {
                "descriptor".to_string()
            }
        }
        XbxEngineRenderPixelData::Rgba { .. } => "rgba".to_string(),
        XbxEngineRenderPixelData::Bgra { .. } => "bgra".to_string(),
        XbxEngineRenderPixelData::Nv12 { .. } => "nv12".to_string(),
    }
}

pub(super) fn frame_has_cv_pixelbuffer(frame: &XbxEngineRenderFrame) -> bool {
    let XbxEngineRenderPixelData::Descriptor { handle } = &frame.pixel_data else {
        return false;
    };
    let any_ref = handle.as_ref() as &dyn Any;
    any_ref.is::<MacOsCVPixelBufferDescriptor>()
}

pub(super) fn frame_has_windows_d3d11_texture(frame: &XbxEngineRenderFrame) -> bool {
    let XbxEngineRenderPixelData::Descriptor { handle } = &frame.pixel_data else {
        return false;
    };
    let any_ref = handle.as_ref() as &dyn Any;
    any_ref.is::<WindowsD3d11TextureDescriptor>()
}

#[cfg(test)]
mod tests {
    use super::apply_host_mailbox_viewport_diagnostics;
    use crate::mods::native_video::{
        scheduling::{HostCadenceTelemetry, ScheduledFrameSlot},
        NativeVideoViewportState,
    };
    use std::sync::Arc;
    use xbxengine::{XbxEngineRenderFrame, XbxEngineRenderPixelData};

    fn mk_frame(frame_seq: u64) -> XbxEngineRenderFrame {
        XbxEngineRenderFrame {
            width: 1920,
            height: 1080,
            frame_seq,
            rendered_at_ms: 1_000.0 + frame_seq as f64,
            rtp_timestamp: Some(10_000 + frame_seq as u32),
            recovery_epoch_tag: None,
            recovery_owner_rtp_timestamp: None,
            is_keyframe: false,
            frame_recovery_disposition: None,
            frame_unrecoverable_reason: None,
            presentation_value_role: None,
            pixel_data: XbxEngineRenderPixelData::Rgba {
                bytes: Arc::from(vec![0_u8; 4].into_boxed_slice()),
            },
        }
    }

    #[test]
    fn mailbox_viewport_diagnostics_prioritize_pending_and_displayed_slot_facts() {
        let mut telemetry = HostCadenceTelemetry::default();
        let mut frame_slot = ScheduledFrameSlot::default();
        let submitted_frame = mk_frame(7);
        let older_frame = mk_frame(6);

        let _ = telemetry.record_submit(1_010.0);
        telemetry.present_enqueue_count_total = 1;
        let _ = frame_slot.submit_frame(&submitted_frame, 1_010.0, &mut telemetry);
        let _ = frame_slot.submit_frame(&older_frame, 1_005.0, &mut telemetry);
        let _ = frame_slot.take_ready_frame(1_006.0, &mut telemetry);
        frame_slot.begin_view_epoch();
        let slot_diag = frame_slot.diagnostics_snapshot();

        let mut viewport = NativeVideoViewportState::default();
        apply_host_mailbox_viewport_diagnostics(
            &mut viewport,
            &telemetry,
            Some(&slot_diag),
            Some(2_000.0),
            Some("metal-import".to_string()),
            3,
            1,
        );

        assert_eq!(
            viewport.latest_host_submit_rtp_timestamp,
            submitted_frame.rtp_timestamp
        );
        assert_eq!(
            viewport.last_displayed_frame_seq,
            Some(submitted_frame.frame_seq)
        );
        assert_eq!(
            viewport.last_displayed_frame_rtp_timestamp,
            submitted_frame.rtp_timestamp
        );
        assert_eq!(viewport.host_view_generation, slot_diag.view_epoch);
        assert_eq!(
            viewport.host_descriptor_upload_mode.as_deref(),
            Some("metal-import")
        );
        assert_eq!(viewport.host_descriptor_metal_import_count_total, 3);
        assert_eq!(viewport.host_descriptor_cpu_upload_count_total, 1);
    }

    #[test]
    fn mailbox_viewport_diagnostics_falls_back_to_displayed_rtp_when_pending_is_empty() {
        let mut telemetry = HostCadenceTelemetry::default();
        let mut frame_slot = ScheduledFrameSlot::default();
        let displayed_frame = mk_frame(9);

        let _ = telemetry.record_submit(1_020.0);
        telemetry.present_enqueue_count_total = 1;
        let _ = frame_slot.submit_frame(&displayed_frame, 1_020.0, &mut telemetry);
        let _ = frame_slot.take_ready_frame(1_021.0, &mut telemetry);
        let slot_diag = frame_slot.diagnostics_snapshot();

        let mut viewport = NativeVideoViewportState::default();
        apply_host_mailbox_viewport_diagnostics(
            &mut viewport,
            &telemetry,
            Some(&slot_diag),
            Some(3_000.0),
            None,
            0,
            0,
        );

        assert_eq!(
            viewport.latest_host_submit_rtp_timestamp,
            displayed_frame.rtp_timestamp
        );
        assert_eq!(
            viewport.last_displayed_frame_seq,
            Some(displayed_frame.frame_seq)
        );
        assert_eq!(
            viewport.last_displayed_frame_rtp_timestamp,
            displayed_frame.rtp_timestamp
        );
    }
}
