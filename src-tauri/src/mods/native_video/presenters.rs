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

use super::scheduling::{HostCadenceTelemetry, ScheduledFrameSlot};
#[cfg(target_os = "windows")]
use super::{
    calculate_recent_fps, calculate_recent_interval_ms, HostCadencePhase, HostFrameDropBacklog,
    HOST_RENDER_FPS_WINDOW_MS, HOST_RENDER_FRAME_AGE_MULTIPLIER, HOST_RENDER_MAX_FRAME_AGE_MS,
    HOST_RENDER_MIN_FRAME_AGE_MS, HOST_TIMING_QUEUE_WARN_MS, HOST_TIMING_TICK_WARN_MS,
};
use super::{
    drop_display_layer, drop_wgpu_host_view, ensure_display_layer, now_ms_f64,
    record_native_video_timing_event_lazy, record_native_video_trace, run_layer_present_tick,
    run_wgpu_render_tick, MacOsDisplayLinkHandle, MacOsLayerDisplayLinkHandle, MacOsLayerState,
    MacOsWgpuState, MacOsWgpuTelemetry, NativeVideoViewportState,
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

#[cfg(target_os = "windows")]
#[derive(Default)]
struct WindowsWgpuState {
    renderer: Option<super::wgpu_renderer::WgpuFrameRenderer>,
    latest_frame: Option<XbxEngineRenderFrame>,
    last_presented_frame_seq: Option<u64>,
    last_surface_size: Option<(u32, u32)>,
    render_loop_started: bool,
    init_failed_logged: bool,
}

#[cfg(target_os = "windows")]
#[derive(Default)]
struct WindowsWgpuTelemetry {
    latest_present_time_ms: Option<f64>,
    display_tick_epoch: u64,
    present_epoch: u64,
    cadence_phase: HostCadencePhase,
    recent_present_times_ms: std::collections::VecDeque<f64>,
    recent_display_tick_times_ms: std::collections::VecDeque<f64>,
    present_enqueue_count_total: u64,
    present_drop_count_total: u64,
    present_overwrite_count_total: u64,
    no_pending_take_count_total: u64,
    no_pending_streak: u32,
    no_pending_max_streak: u32,
    pending_frame_drops: HostFrameDropBacklog,
    descriptor_upload_mode: Option<String>,
    descriptor_import_count_total: u64,
    descriptor_cpu_upload_count_total: u64,
}

#[cfg(target_os = "windows")]
impl WindowsWgpuTelemetry {
    fn record_display_tick(&mut self, now_ms: f64) {
        self.recent_display_tick_times_ms.push_back(now_ms);
        self.trim_display_ticks(now_ms);
        self.display_tick_epoch = self.display_tick_epoch.saturating_add(1);
        if matches!(self.cadence_phase, HostCadencePhase::Idle) {
            self.cadence_phase = HostCadencePhase::Priming;
        }
    }

    fn record_present(&mut self, now_ms: f64) {
        self.latest_present_time_ms = Some(now_ms);
        self.recent_present_times_ms.push_back(now_ms);
        self.trim_recent(now_ms);
        self.present_epoch = self.present_epoch.saturating_add(1);
        self.cadence_phase = HostCadencePhase::Steady;
    }

    fn record_drop(&mut self) {
        self.present_drop_count_total = self.present_drop_count_total.saturating_add(1);
    }

    fn record_no_pending_take(&mut self) {
        self.no_pending_take_count_total = self.no_pending_take_count_total.saturating_add(1);
        self.no_pending_streak = self.no_pending_streak.saturating_add(1);
        self.no_pending_max_streak = self.no_pending_max_streak.max(self.no_pending_streak);
        if self.present_epoch > 0 {
            self.cadence_phase = HostCadencePhase::Starved;
        }
    }

    fn clear_no_pending_streak(&mut self) {
        self.no_pending_streak = 0;
        self.cadence_phase = if self.present_epoch > 0 {
            HostCadencePhase::Steady
        } else if self.display_tick_epoch > 0 {
            HostCadencePhase::Priming
        } else {
            HostCadencePhase::Idle
        };
    }

    fn record_stale_frame_drop(
        &mut self,
        frame: &XbxEngineRenderFrame,
        observed_at_ms: f64,
        detail: &str,
        queue_depth: usize,
    ) {
        self.record_drop();
        self.pending_frame_drops.record_stale_frame_drop(
            frame,
            observed_at_ms,
            detail,
            queue_depth,
        );
    }

    fn take_pending_frame_drops(&mut self) -> Vec<xbxengine::XbxEngineHostVideoFrameDropEvent> {
        self.pending_frame_drops.take_all()
    }

    fn present_fps(&self) -> f64 {
        calculate_recent_fps(&self.recent_present_times_ms)
    }

    fn display_interval_ms(&self) -> Option<f64> {
        calculate_recent_interval_ms(&self.recent_display_tick_times_ms)
    }

    fn frame_age_budget_ms(&self) -> f64 {
        self.display_interval_ms()
            .map(|interval_ms| {
                (interval_ms * HOST_RENDER_FRAME_AGE_MULTIPLIER)
                    .clamp(HOST_RENDER_MIN_FRAME_AGE_MS, HOST_RENDER_MAX_FRAME_AGE_MS)
            })
            .unwrap_or(HOST_RENDER_MAX_FRAME_AGE_MS)
    }

    fn reset_frame_slot(&mut self) {
        self.latest_present_time_ms = None;
        self.display_tick_epoch = 0;
        self.present_epoch = 0;
        self.cadence_phase = HostCadencePhase::Idle;
        self.recent_present_times_ms.clear();
        self.recent_display_tick_times_ms.clear();
        self.present_enqueue_count_total = 0;
        self.present_drop_count_total = 0;
        self.present_overwrite_count_total = 0;
        self.no_pending_take_count_total = 0;
        self.no_pending_streak = 0;
        self.no_pending_max_streak = 0;
        self.pending_frame_drops.reset();
        self.descriptor_upload_mode = None;
        self.descriptor_import_count_total = 0;
        self.descriptor_cpu_upload_count_total = 0;
    }

    fn trim_recent(&mut self, now_ms: f64) {
        while self
            .recent_present_times_ms
            .front()
            .is_some_and(|ts_ms| now_ms - *ts_ms > HOST_RENDER_FPS_WINDOW_MS)
        {
            self.recent_present_times_ms.pop_front();
        }
    }

    fn trim_display_ticks(&mut self, now_ms: f64) {
        while self
            .recent_display_tick_times_ms
            .front()
            .is_some_and(|ts_ms| now_ms - *ts_ms > HOST_RENDER_FPS_WINDOW_MS)
        {
            self.recent_display_tick_times_ms.pop_front();
        }
    }
}

#[cfg(target_os = "windows")]
struct WindowsPendingGuard {
    pending: Arc<std::sync::atomic::AtomicBool>,
}

#[cfg(target_os = "windows")]
impl WindowsPendingGuard {
    fn new(pending: Arc<std::sync::atomic::AtomicBool>) -> Self {
        Self { pending }
    }
}

#[cfg(target_os = "windows")]
impl Drop for WindowsPendingGuard {
    fn drop(&mut self) {
        self.pending.store(false, Ordering::Relaxed);
    }
}

#[cfg(target_os = "windows")]
pub(super) struct WindowsWgpuPresenter {
    viewport_id: String,
    window_label: String,
    surface_id: Option<String>,
    app_handle: AppHandle,
    renderer_state: Arc<Mutex<WindowsWgpuState>>,
    telemetry: Arc<Mutex<WindowsWgpuTelemetry>>,
    render_loop_stop: Arc<std::sync::atomic::AtomicBool>,
    render_loop_pending: Arc<std::sync::atomic::AtomicBool>,
    runtime_trace: Option<RuntimeTraceRecorderRef>,
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
            telemetry: Arc::new(Mutex::new(WindowsWgpuTelemetry::default())),
            render_loop_stop: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            render_loop_pending: Arc::new(std::sync::atomic::AtomicBool::new(false)),
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

        let viewport_id = self.viewport_id.clone();
        let window_label = self.window_label.clone();
        let app_handle = self.app_handle.clone();
        let renderer_state = self.renderer_state.clone();
        let telemetry = self.telemetry.clone();
        let render_loop_stop = self.render_loop_stop.clone();
        let render_loop_pending = self.render_loop_pending.clone();
        let runtime_trace = self.runtime_trace.clone();
        thread::Builder::new()
            .name(format!("XbxWindowsWgpuRenderLoop-{viewport_id}"))
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
                    let runtime_trace_for_task = runtime_trace.clone();
                    let dispatch_requested_at_ms = now_ms_f64();
                    let window_for_task = window.clone();
                    let _ = window.run_on_main_thread(move || {
                        run_windows_wgpu_render_tick(
                            &window_for_task,
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
            .expect("Failed to spawn Windows wgpu render loop");
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
            return;
        }
        if let Ok(mut state) = self.renderer_state.lock() {
            if state
                .last_presented_frame_seq
                .is_some_and(|presented_seq| frame.frame_seq <= presented_seq)
            {
                return;
            }
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
        }
    }

    fn detach(&mut self) {
        self.surface_id = None;
        self.render_loop_stop.store(true, Ordering::Relaxed);
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
        viewport.host_display_tick_epoch = telemetry.display_tick_epoch;
        viewport.host_present_epoch = telemetry.present_epoch;
        viewport.host_cadence_phase = Some(telemetry.cadence_phase.as_str().to_string());
        viewport.host_display_interval_ms = telemetry.display_interval_ms();
        viewport.host_frame_age_budget_ms = Some(telemetry.frame_age_budget_ms());
        viewport.host_descriptor_upload_mode = telemetry.descriptor_upload_mode.clone();
        viewport.host_descriptor_metal_import_count_total = telemetry.descriptor_import_count_total;
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

#[cfg(target_os = "windows")]
fn run_windows_wgpu_render_tick(
    window: &tauri::Window,
    viewport_id: &str,
    renderer_state: &Arc<Mutex<WindowsWgpuState>>,
    telemetry: &Arc<Mutex<WindowsWgpuTelemetry>>,
    render_loop_pending: &Arc<std::sync::atomic::AtomicBool>,
    dispatch_requested_at_ms: Option<f64>,
    runtime_trace: Option<RuntimeTraceRecorderRef>,
) {
    let tick_started_at_ms = now_ms_f64();
    let _pending_guard = WindowsPendingGuard::new(render_loop_pending.clone());
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
                state.last_surface_size = Some((surface_width, surface_height));
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

    let now_ms = now_ms_f64();
    let frame_age_budget_ms = if let Ok(mut telemetry) = telemetry.lock() {
        telemetry.record_display_tick(now_ms);
        telemetry.frame_age_budget_ms()
    } else {
        HOST_RENDER_MAX_FRAME_AGE_MS
    };
    let should_render = size_changed
        || state.latest_frame.as_ref().map(|frame| frame.frame_seq)
            != state.last_presented_frame_seq;
    let latest_frame = state.latest_frame.clone();
    if latest_frame.is_none() {
        if let Ok(mut telemetry) = telemetry.lock() {
            telemetry.record_no_pending_take();
        }
    } else if let Ok(mut telemetry) = telemetry.lock() {
        telemetry.clear_no_pending_streak();
    }
    let rendered_seq_before = state.last_presented_frame_seq;
    if size_changed {
        state.last_surface_size = Some((surface_width, surface_height));
    }
    let Some(renderer) = state.renderer.as_mut() else {
        return;
    };
    if size_changed {
        renderer.update_surface_size(surface_width, surface_height);
    }
    if let Some(frame) = latest_frame {
        if should_render {
            if now_ms - frame.rendered_at_ms > frame_age_budget_ms {
                state.last_presented_frame_seq = Some(frame.frame_seq);
                state.latest_frame = None;
                if let Ok(mut telemetry) = telemetry.lock() {
                    telemetry.record_stale_frame_drop(&frame, now_ms, "scheduledFrameStale", 1);
                }
                return;
            }
            let rendered_seq = frame.frame_seq;
            renderer.update_frame(frame);
            if let Err(error) = renderer.render() {
                log::warn!(
                    "[native_video][windows][wgpu] render failed for viewport={} window={} error={}",
                    viewport_id,
                    window.label(),
                    error
                );
            } else {
                let descriptor_upload = renderer.descriptor_upload_telemetry();
                state.last_presented_frame_seq = Some(rendered_seq);
                if let Ok(mut telemetry) = telemetry.lock() {
                    telemetry.record_present(now_ms);
                    telemetry.descriptor_upload_mode = descriptor_upload.last_mode;
                    telemetry.descriptor_import_count_total =
                        descriptor_upload.metal_import_count_total;
                    telemetry.descriptor_cpu_upload_count_total =
                        descriptor_upload.cpu_upload_count_total;
                }
            }
        }
    } else if rendered_seq_before.is_none() && size_changed {
        let _ = renderer.render();
    }
    let tick_total_ms = (now_ms_f64() - tick_started_at_ms).max(0.0);
    if tick_total_ms >= HOST_TIMING_TICK_WARN_MS {
        record_native_video_timing_event_lazy(
            runtime_trace.as_ref(),
            "wgpu",
            "tick_total",
            viewport_id,
            window.label(),
            || serde_json::json!({ "totalMs": tick_total_ms }),
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
            record_native_video_timing_event_lazy(
                self.runtime_trace.as_ref(),
                "wgpu",
                "frame_submit",
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
            return;
        }
        if let Ok(mut state) = self.renderer_state.lock() {
            if state
                .last_presented_frame_seq
                .is_some_and(|presented_seq| frame.frame_seq <= presented_seq)
            {
                record_native_video_timing_event_lazy(
                    self.runtime_trace.as_ref(),
                    "wgpu",
                    "frame_submit",
                    &self.viewport_id,
                    &self.window_label,
                    || {
                        serde_json::json!({
                            "outcome": "already_presented",
                            "frameSeq": frame.frame_seq,
                            "lastPresentedFrameSeq": state.last_presented_frame_seq,
                        })
                    },
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
            record_native_video_timing_event_lazy(
                self.runtime_trace.as_ref(),
                "wgpu",
                "frame_submit",
                &self.viewport_id,
                &self.window_label,
                || {
                    serde_json::json!({
                        "outcome": "accepted",
                        "frameSeq": frame.frame_seq,
                        "frameAgeMs": (now_ms - frame.rendered_at_ms).max(0.0),
                        "overwrotePending": overwrote_pending,
                        "replacedFrameSeq": replaced_frame_seq,
                    })
                },
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

    fn ensure_layer_ready_on_main_thread(&self) {
        let Some(window) = self.app_handle.get_window(&self.window_label) else {
            return;
        };
        let layer_state = self.layer_state.clone();
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
            if ensure_display_layer(&window_for_task, &mut state).is_none() {
                record_native_video_timing_event_lazy(
                    runtime_trace.as_ref(),
                    "layer",
                    "display_layer_init_failed",
                    &viewport_id,
                    &window_label,
                    || serde_json::json!({ "reason": "displayLayerUnavailable" }),
                );
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
                    run_layer_present_tick(
                        &viewport_id,
                        &window_label,
                        &layer_state,
                        &frame_slot,
                        &telemetry,
                        &render_loop_pending,
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
            record_native_video_timing_event_lazy(
                self.runtime_trace.as_ref(),
                "layer",
                "frame_submit",
                &self.viewport_id,
                &self.window_label,
                || {
                    serde_json::json!({
                        "outcome": "rejected_non_cv_pixelbuffer",
                        "frameSeq": frame.frame_seq,
                    })
                },
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
            record_native_video_timing_event_lazy(
                self.runtime_trace.as_ref(),
                "layer",
                "frame_submit",
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
            return;
        }
        let Ok(mut telemetry) = self.telemetry.lock() else {
            record_native_video_timing_event_lazy(
                self.runtime_trace.as_ref(),
                "layer",
                "frame_submit_failed",
                &self.viewport_id,
                &self.window_label,
                || {
                    serde_json::json!({
                        "reason": "telemetryLockFailed",
                        "frameSeq": frame.frame_seq,
                    })
                },
            );
            return;
        };
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
                "frame_submit_failed",
                &self.viewport_id,
                &self.window_label,
                || {
                    serde_json::json!({
                        "reason": "frameSlotLockFailed",
                        "frameSeq": frame.frame_seq,
                    })
                },
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
                let slot_diag = frame_slot.diagnostics_snapshot();
                record_native_video_timing_event_lazy(
                    self.runtime_trace.as_ref(),
                    "layer",
                    "frame_submit",
                    &self.viewport_id,
                    &self.window_label,
                    || {
                        serde_json::json!({
                            "outcome": "accepted",
                            "frameSeq": frame_seq,
                            "frameAgeMs": frame_age_ms,
                            "frameAgeBudgetMs": frame_age_budget_ms,
                            "submitGapMs": submit_gap_ms,
                            "noPendingStreakBeforeSubmit": no_pending_streak_before_submit,
                            "overwrotePending": overwrote_pending,
                            "replacedFrameSeq": replaced_frame_seq,
                            "displayedFrameSeq": slot_diag.displayed_frame_seq,
                            "pendingFrameSeqs": slot_diag.pending_frame_seqs,
                            "lastPresentedFrameSeq": slot_diag.last_presented_frame_seq,
                            "queueDepth": slot_diag.queue_depth,
                            "pendingQueueDepth": slot_diag.pending_queue_depth,
                            "hostDisplayTickEpoch": telemetry_diag.display_tick_epoch,
                            "hostPresentEpoch": telemetry_diag.present_epoch,
                            "hostCadencePhase": telemetry_diag.cadence_phase.as_str(),
                        })
                    },
                );
                if let Some(gap_ms) = submit_gap_ms.filter(|_| should_warn_submit_gap) {
                    // 只在供帧节奏明显掉速时额外留痕，避免健康阶段刷屏。
                    record_native_video_timing_event_lazy(
                        self.runtime_trace.as_ref(),
                        "layer",
                        "frame_submit_gap",
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
                                "pendingFrameSeqs": slot_diag.pending_frame_seqs,
                                "queueDepth": slot_diag.queue_depth,
                                "pendingQueueDepth": slot_diag.pending_queue_depth,
                                "hostDisplayTickEpoch": telemetry_diag.display_tick_epoch,
                                "hostPresentEpoch": telemetry_diag.present_epoch,
                            })
                        },
                    );
                }
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
                    "frame_submit",
                    &self.viewport_id,
                    &self.window_label,
                    || {
                        serde_json::json!({
                            "outcome": "stale",
                            "frameSeq": frame_seq,
                            "frameAgeMs": frame_age_ms,
                            "frameAgeBudgetMs": frame_age_budget_ms,
                            "submitGapMs": submit_gap_ms,
                            "noPendingStreakBeforeSubmit": no_pending_streak_before_submit,
                            "displayedFrameSeq": slot_diag.displayed_frame_seq,
                            "pendingFrameSeqs": slot_diag.pending_frame_seqs,
                            "queueDepth": slot_diag.queue_depth,
                            "pendingQueueDepth": slot_diag.pending_queue_depth,
                            "hostDisplayTickEpoch": telemetry_diag.display_tick_epoch,
                            "hostPresentEpoch": telemetry_diag.present_epoch,
                            "hostCadencePhase": telemetry_diag.cadence_phase.as_str(),
                        })
                    },
                );
            }
            super::scheduling::ScheduledFrameSubmitOutcome::RejectedAlreadyPresented {
                frame_seq,
                last_presented_frame_seq,
            } => {
                let slot_diag = frame_slot.diagnostics_snapshot();
                record_native_video_timing_event_lazy(
                    self.runtime_trace.as_ref(),
                    "layer",
                    "frame_submit",
                    &self.viewport_id,
                    &self.window_label,
                    || {
                        serde_json::json!({
                            "outcome": "already_presented",
                            "frameSeq": frame_seq,
                            "lastPresentedFrameSeq": last_presented_frame_seq,
                            "submitGapMs": submit_gap_ms,
                            "noPendingStreakBeforeSubmit": no_pending_streak_before_submit,
                            "displayedFrameSeq": slot_diag.displayed_frame_seq,
                            "pendingFrameSeqs": slot_diag.pending_frame_seqs,
                            "queueDepth": slot_diag.queue_depth,
                            "pendingQueueDepth": slot_diag.pending_queue_depth,
                            "hostDisplayTickEpoch": telemetry_diag.display_tick_epoch,
                            "hostPresentEpoch": telemetry_diag.present_epoch,
                            "hostCadencePhase": telemetry_diag.cadence_phase.as_str(),
                        })
                    },
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
