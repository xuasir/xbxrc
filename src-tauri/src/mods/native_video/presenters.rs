use std::any::Any;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::mods::runtime_trace::RuntimeTraceRecorderRef;
use tauri::{AppHandle, Manager};
use xbxengine::{MacOsCVPixelBufferDescriptor, XbxEngineRenderFrame, XbxEngineRenderPixelData};

use super::scheduling::{HostCadenceTelemetry, ScheduledFrameSlot};
use super::{
    drop_display_layer, drop_wgpu_host_view, now_ms_f64, prepare_layer_sample_for_present,
    record_native_video_timing_event, record_native_video_trace, run_layer_present_tick,
    run_wgpu_render_tick, LayerSamplePrepareOutcome, MacOsDisplayLinkHandle,
    MacOsLayerDisplayLinkHandle, MacOsLayerState, MacOsWgpuState, MacOsWgpuTelemetry,
    NativeVideoViewportState,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum NativeVideoPresenterKind {
    Noop,
    PlatformNative,
    Wgpu,
}

pub(super) trait NativeVideoPresenter: Send {
    fn kind(&self) -> NativeVideoPresenterKind;
    fn attach(&mut self, surface_id: Option<&str>);
    fn present(&mut self, surface_id: Option<&str>, frame: &XbxEngineRenderFrame);
    fn detach(&mut self);
    fn begin_media_epoch(&mut self) {}
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
}

impl NoopVideoPresenter {
    pub(super) fn new(viewport_id: &str, window_label: &str) -> Self {
        Self {
            viewport_id: viewport_id.to_string(),
            window_label: window_label.to_string(),
            surface_id: None,
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

    fn present(&mut self, surface_id: Option<&str>, _frame: &XbxEngineRenderFrame) {
        self.surface_id = surface_id
            .map(str::to_string)
            .or_else(|| self.surface_id.clone());
    }

    fn detach(&mut self) {
        self.surface_id = None;
    }
}

#[cfg(target_os = "macos")]
pub(super) struct MacOsWgpuPresenter {
    viewport_id: String,
    window_label: String,
    surface_id: Option<String>,
    app_handle: AppHandle,
    renderer_state: Arc<Mutex<MacOsWgpuState>>,
    telemetry: Arc<Mutex<MacOsWgpuTelemetry>>,
    render_loop_stop: Arc<std::sync::atomic::AtomicBool>,
    render_loop_pending: Arc<std::sync::atomic::AtomicBool>,
    display_link: Option<MacOsDisplayLinkHandle>,
    runtime_trace: Option<RuntimeTraceRecorderRef>,
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
            telemetry: Arc::new(Mutex::new(MacOsWgpuTelemetry::default())),
            render_loop_stop: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            render_loop_pending: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            display_link: None,
            runtime_trace,
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
                self.telemetry.clone(),
                self.render_loop_pending.clone(),
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
        let telemetry = self.telemetry.clone();
        let render_loop_stop = self.render_loop_stop.clone();
        let render_loop_pending = self.render_loop_pending.clone();
        let runtime_trace = self.runtime_trace.clone();
        thread::Builder::new()
            .name(format!("XbxWgpuRenderLoop-{viewport_id}"))
            .spawn(move || {
                let tick = Duration::from_millis(16);
                while !render_loop_stop.load(Ordering::Relaxed) {
                    thread::sleep(tick);
                    if render_loop_pending.swap(true, Ordering::Relaxed) {
                        continue;
                    }
                    let Some(window) = app_handle.get_window(&window_label) else {
                        render_loop_pending.store(false, Ordering::Relaxed);
                        continue;
                    };
                    let renderer_state = renderer_state.clone();
                    let telemetry = telemetry.clone();
                    let render_loop_pending = render_loop_pending.clone();
                    let viewport_id = viewport_id.clone();
                    let app_handle_for_task = app_handle.clone();
                    let window_for_task = window.clone();
                    let runtime_trace_for_task = runtime_trace.clone();
                    let dispatch_requested_at_ms = now_ms_f64();
                    let _ = window.run_on_main_thread(move || {
                        run_wgpu_render_tick(
                            &window_for_task,
                            &app_handle_for_task,
                            &viewport_id,
                            &renderer_state,
                            &telemetry,
                            &render_loop_pending,
                            Some(dispatch_requested_at_ms),
                            runtime_trace_for_task,
                        );
                    });
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
        }
        if let Ok(mut telemetry) = self.telemetry.lock() {
            telemetry.reset_frame_slot();
        }
        self.render_loop_pending.store(false, Ordering::Relaxed);
    }

    fn present(&mut self, surface_id: Option<&str>, frame: &XbxEngineRenderFrame) {
        self.surface_id = surface_id
            .map(str::to_string)
            .or_else(|| self.surface_id.clone());
        self.ensure_render_loop();
        let now_ms = now_ms_f64();
        if let Ok(mut telemetry) = self.telemetry.lock() {
            telemetry.present_enqueue_count_total =
                telemetry.present_enqueue_count_total.saturating_add(1);
        }
        if self.should_drop_submitted_frame(frame, now_ms) {
            if let Ok(mut telemetry) = self.telemetry.lock() {
                telemetry.record_stale_frame_drop(frame, now_ms, "submittedFrameStale", 0);
            }
            record_native_video_timing_event(
                self.runtime_trace.as_ref(),
                "wgpu",
                "frame_submit",
                &self.viewport_id,
                &self.window_label,
                serde_json::json!({
                    "outcome": "stale",
                    "frameSeq": frame.frame_seq,
                    "frameAgeMs": (now_ms - frame.rendered_at_ms).max(0.0),
                }),
            );
            log::debug!(
                "[native_video][wgpu] reject stale submitted frame viewport={} window={} frame_seq={} age_ms={:.2}",
                self.viewport_id,
                self.window_label,
                frame.frame_seq,
                now_ms - frame.rendered_at_ms
            );
            return;
        }
        if let Ok(mut state) = self.renderer_state.lock() {
            if state
                .last_presented_frame_seq
                .is_some_and(|presented_seq| frame.frame_seq <= presented_seq)
            {
                record_native_video_timing_event(
                    self.runtime_trace.as_ref(),
                    "wgpu",
                    "frame_submit",
                    &self.viewport_id,
                    &self.window_label,
                    serde_json::json!({
                        "outcome": "already_presented",
                        "frameSeq": frame.frame_seq,
                        "lastPresentedFrameSeq": state.last_presented_frame_seq,
                    }),
                );
                return;
            }
            let replaced_frame_seq = state.latest_frame.as_ref().map(|latest| latest.frame_seq);
            let overwrote_pending = state.latest_frame.as_ref().is_some_and(|latest| {
                Some(latest.frame_seq) != state.last_presented_frame_seq
                    && latest.frame_seq != frame.frame_seq
            });
            if overwrote_pending {
                if let Ok(mut telemetry) = self.telemetry.lock() {
                    telemetry.present_overwrite_count_total =
                        telemetry.present_overwrite_count_total.saturating_add(1);
                }
            }
            state.latest_frame = Some(frame.clone());
            record_native_video_timing_event(
                self.runtime_trace.as_ref(),
                "wgpu",
                "frame_submit",
                &self.viewport_id,
                &self.window_label,
                serde_json::json!({
                    "outcome": "accepted",
                    "frameSeq": frame.frame_seq,
                    "frameAgeMs": (now_ms - frame.rendered_at_ms).max(0.0),
                    "overwrotePending": overwrote_pending,
                    "replacedFrameSeq": replaced_frame_seq,
                }),
            );
        }
    }

    fn detach(&mut self) {
        self.surface_id = None;
        self.render_loop_stop.store(true, Ordering::Relaxed);
        if let Some(display_link) = self.display_link.take() {
            display_link.stop();
        }
        let renderer_state = self.renderer_state.clone();
        let telemetry = self.telemetry.clone();
        let window_label = self.window_label.clone();
        let app_handle = self.app_handle.clone();
        if let Some(window) = app_handle.get_window(&window_label) {
            let _ = window.run_on_main_thread(move || {
                if let Ok(mut state) = renderer_state.lock() {
                    state.renderer = None;
                    state.latest_frame = None;
                    state.last_presented_frame_seq = None;
                    state.last_surface_size = None;
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
        viewport.latest_host_present_time_ms = telemetry.latest_present_time_ms;
        viewport.host_present_fps = telemetry.present_fps();
        viewport.host_present_enqueue_count_total = telemetry.present_enqueue_count_total;
        viewport.host_present_drop_count_total = telemetry.present_drop_count_total;
        viewport.host_present_overwrite_count_total = telemetry.present_overwrite_count_total;
        viewport.host_no_pending_take_count_total = telemetry.no_pending_take_count_total;
        viewport.host_no_pending_streak = telemetry.no_pending_streak;
        viewport.host_no_pending_max_streak = telemetry.no_pending_max_streak;
        viewport.host_display_tick_epoch = telemetry.display_tick_epoch();
        viewport.host_present_epoch = telemetry.present_epoch();
        viewport.host_cadence_phase = Some(telemetry.cadence_phase().as_str().to_string());
        viewport.host_display_interval_ms = telemetry.display_interval_ms();
        viewport.host_frame_age_budget_ms = Some(telemetry.frame_age_budget_ms());
        viewport.host_descriptor_upload_mode = telemetry.descriptor_upload_mode.clone();
        viewport.host_descriptor_metal_import_count_total =
            telemetry.descriptor_metal_import_count_total;
        viewport.host_descriptor_cpu_upload_count_total =
            telemetry.descriptor_cpu_upload_count_total;
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
    display_link: Option<MacOsLayerDisplayLinkHandle>,
    runtime_trace: Option<RuntimeTraceRecorderRef>,
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
            display_link: None,
            runtime_trace,
        }
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
        if self.display_link.is_none() {
            if let Ok(display_link) = MacOsLayerDisplayLinkHandle::start(
                self.viewport_id.clone(),
                self.window_label.clone(),
                self.app_handle.clone(),
                self.layer_state.clone(),
                self.frame_slot.clone(),
                self.telemetry.clone(),
                self.render_loop_pending.clone(),
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
        let app_handle = self.app_handle.clone();
        let layer_state = self.layer_state.clone();
        let frame_slot = self.frame_slot.clone();
        let telemetry = self.telemetry.clone();
        let render_loop_stop = self.render_loop_stop.clone();
        let render_loop_pending = self.render_loop_pending.clone();
        let runtime_trace = self.runtime_trace.clone();
        thread::Builder::new()
            .name(format!("XbxLayerRenderLoop-{viewport_id}"))
            .spawn(move || {
                let tick = Duration::from_millis(16);
                while !render_loop_stop.load(Ordering::Relaxed) {
                    thread::sleep(tick);
                    if render_loop_pending.swap(true, Ordering::Relaxed) {
                        continue;
                    }
                    let Some(window) = app_handle.get_window(&window_label) else {
                        render_loop_pending.store(false, Ordering::Relaxed);
                        continue;
                    };
                    let layer_state = layer_state.clone();
                    let frame_slot = frame_slot.clone();
                    let telemetry = telemetry.clone();
                    let render_loop_pending = render_loop_pending.clone();
                    let viewport_id = viewport_id.clone();
                    let window_for_task = window.clone();
                    let runtime_trace_for_task = runtime_trace.clone();
                    let dispatch_requested_at_ms = now_ms_f64();
                    let prepare_outcome = prepare_layer_sample_for_present(
                        &layer_state,
                        &frame_slot,
                        &telemetry,
                        &viewport_id,
                        &window_label,
                        runtime_trace_for_task.as_ref(),
                    );
                    if !matches!(prepare_outcome, LayerSamplePrepareOutcome::Prepared) {
                        render_loop_pending.store(false, Ordering::Relaxed);
                        continue;
                    }
                    let _ = window.run_on_main_thread(move || {
                        run_layer_present_tick(
                            &window_for_task,
                            &viewport_id,
                            &layer_state,
                            &telemetry,
                            &render_loop_pending,
                            Some(dispatch_requested_at_ms),
                            runtime_trace_for_task,
                        );
                    });
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
        self.render_loop_pending.store(false, Ordering::Relaxed);
    }

    fn present(&mut self, surface_id: Option<&str>, frame: &XbxEngineRenderFrame) {
        self.surface_id = surface_id
            .map(str::to_string)
            .or_else(|| self.surface_id.clone());
        self.ensure_render_loop();
        if !frame_has_cv_pixelbuffer(frame) {
            record_native_video_timing_event(
                self.runtime_trace.as_ref(),
                "layer",
                "frame_submit",
                &self.viewport_id,
                &self.window_label,
                serde_json::json!({
                    "outcome": "rejected_non_cv_pixelbuffer",
                    "frameSeq": frame.frame_seq,
                }),
            );
            return;
        }
        let now_ms = now_ms_f64();
        if self.should_drop_submitted_frame(frame, now_ms) {
            if let Ok(mut telemetry) = self.telemetry.lock() {
                telemetry.present_enqueue_count_total =
                    telemetry.present_enqueue_count_total.saturating_add(1);
                telemetry.record_stale_frame_drop(frame, now_ms, "submittedFrameStale", 0);
            }
            record_native_video_timing_event(
                self.runtime_trace.as_ref(),
                "layer",
                "frame_submit",
                &self.viewport_id,
                &self.window_label,
                serde_json::json!({
                    "outcome": "stale",
                    "frameSeq": frame.frame_seq,
                    "frameAgeMs": (now_ms - frame.rendered_at_ms).max(0.0),
                }),
            );
            log::debug!(
                "[native_video][layer] reject stale submitted frame viewport={} window={} frame_seq={} age_ms={:.2}",
                self.viewport_id,
                self.window_label,
                frame.frame_seq,
                now_ms - frame.rendered_at_ms
            );
            return;
        }
        let Ok(mut telemetry) = self.telemetry.lock() else {
            record_native_video_timing_event(
                self.runtime_trace.as_ref(),
                "layer",
                "frame_submit_failed",
                &self.viewport_id,
                &self.window_label,
                serde_json::json!({
                    "reason": "telemetryLockFailed",
                    "frameSeq": frame.frame_seq,
                }),
            );
            return;
        };
        let submit_gap_ms = telemetry.record_submit(now_ms);
        let no_pending_streak_before_submit = telemetry.no_pending_streak;
        let should_warn_submit_gap =
            submit_gap_ms.is_some_and(|gap_ms| telemetry.should_warn_submit_gap(gap_ms));
        telemetry.present_enqueue_count_total =
            telemetry.present_enqueue_count_total.saturating_add(1);
        let Ok(mut frame_slot) = self.frame_slot.lock() else {
            record_native_video_timing_event(
                self.runtime_trace.as_ref(),
                "layer",
                "frame_submit_failed",
                &self.viewport_id,
                &self.window_label,
                serde_json::json!({
                    "reason": "frameSlotLockFailed",
                    "frameSeq": frame.frame_seq,
                }),
            );
            return;
        };
        match frame_slot.submit_frame(frame, now_ms, &mut telemetry) {
            super::scheduling::ScheduledFrameSubmitOutcome::Accepted {
                frame_seq,
                overwrote_pending,
                replaced_frame_seq,
                frame_age_ms,
                frame_age_budget_ms,
            } => {
                record_native_video_timing_event(
                    self.runtime_trace.as_ref(),
                    "layer",
                    "frame_submit",
                    &self.viewport_id,
                    &self.window_label,
                    serde_json::json!({
                        "outcome": "accepted",
                        "frameSeq": frame_seq,
                        "frameAgeMs": frame_age_ms,
                        "frameAgeBudgetMs": frame_age_budget_ms,
                        "submitGapMs": submit_gap_ms,
                        "noPendingStreakBeforeSubmit": no_pending_streak_before_submit,
                        "overwrotePending": overwrote_pending,
                        "replacedFrameSeq": replaced_frame_seq,
                    }),
                );
                if let Some(gap_ms) = submit_gap_ms.filter(|_| should_warn_submit_gap) {
                    // 只在供帧节奏明显掉速时额外留痕，避免健康阶段刷屏。
                    record_native_video_timing_event(
                        self.runtime_trace.as_ref(),
                        "layer",
                        "frame_submit_gap",
                        &self.viewport_id,
                        &self.window_label,
                        serde_json::json!({
                            "frameSeq": frame_seq,
                            "submitGapMs": gap_ms,
                            "frameAgeMs": frame_age_ms,
                            "frameAgeBudgetMs": frame_age_budget_ms,
                            "noPendingStreakBeforeSubmit": no_pending_streak_before_submit,
                            "overwrotePending": overwrote_pending,
                            "replacedFrameSeq": replaced_frame_seq,
                        }),
                    );
                }
            }
            super::scheduling::ScheduledFrameSubmitOutcome::DroppedStale {
                frame_seq,
                frame_age_ms,
                frame_age_budget_ms,
            } => {
                record_native_video_timing_event(
                    self.runtime_trace.as_ref(),
                    "layer",
                    "frame_submit",
                    &self.viewport_id,
                    &self.window_label,
                    serde_json::json!({
                        "outcome": "stale",
                        "frameSeq": frame_seq,
                        "frameAgeMs": frame_age_ms,
                        "frameAgeBudgetMs": frame_age_budget_ms,
                        "submitGapMs": submit_gap_ms,
                        "noPendingStreakBeforeSubmit": no_pending_streak_before_submit,
                    }),
                );
            }
            super::scheduling::ScheduledFrameSubmitOutcome::RejectedAlreadyPresented {
                frame_seq,
                last_presented_frame_seq,
            } => {
                record_native_video_timing_event(
                    self.runtime_trace.as_ref(),
                    "layer",
                    "frame_submit",
                    &self.viewport_id,
                    &self.window_label,
                    serde_json::json!({
                        "outcome": "already_presented",
                        "frameSeq": frame_seq,
                        "lastPresentedFrameSeq": last_presented_frame_seq,
                        "submitGapMs": submit_gap_ms,
                        "noPendingStreakBeforeSubmit": no_pending_streak_before_submit,
                    }),
                );
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
        viewport.latest_host_present_time_ms = telemetry.latest_present_time_ms;
        viewport.host_present_fps = telemetry.present_fps();
        viewport.host_present_enqueue_count_total = telemetry.present_enqueue_count_total;
        viewport.host_present_drop_count_total = telemetry.present_drop_count_total;
        viewport.host_present_overwrite_count_total = telemetry.present_overwrite_count_total;
        viewport.host_no_pending_take_count_total = telemetry.no_pending_take_count_total;
        viewport.host_no_pending_streak = telemetry.no_pending_streak;
        viewport.host_no_pending_max_streak = telemetry.no_pending_max_streak;
        viewport.host_display_tick_epoch = telemetry.display_tick_epoch();
        viewport.host_present_epoch = telemetry.present_epoch();
        viewport.host_cadence_phase = Some(telemetry.cadence_phase().as_str().to_string());
        viewport.host_display_interval_ms = telemetry.display_interval_ms();
        viewport.host_frame_age_budget_ms = Some(telemetry.frame_age_budget_ms());
        viewport.host_descriptor_upload_mode = None;
        viewport.host_descriptor_metal_import_count_total = 0;
        viewport.host_descriptor_cpu_upload_count_total = 0;
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
