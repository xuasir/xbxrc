use std::any::Any;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use tauri::{AppHandle, Manager};
use xbxengine::{MacOsCVPixelBufferDescriptor, XbxEngineRenderFrame, XbxEngineRenderPixelData};

use super::scheduling::{HostCadenceTelemetry, ScheduledFrameSlot};
use super::{
    drop_display_layer, drop_wgpu_host_view, now_ms_f64, run_layer_present_tick,
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
    fn apply_viewport_diagnostics(&self, _viewport: &mut NativeVideoViewportState) {}
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
}

#[cfg(target_os = "macos")]
impl MacOsWgpuPresenter {
    pub(super) fn new(viewport_id: &str, window_label: &str, app_handle: AppHandle) -> Self {
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
            ) {
                self.display_link = Some(display_link);
                return;
            }
        }
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
                    let _ = window.run_on_main_thread(move || {
                        run_wgpu_render_tick(
                            &window_for_task,
                            &app_handle_for_task,
                            &viewport_id,
                            &renderer_state,
                            &telemetry,
                            &render_loop_pending,
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
        self.surface_id = surface_id.map(str::to_string);
        self.ensure_render_loop();
    }

    fn present(&mut self, surface_id: Option<&str>, frame: &XbxEngineRenderFrame) {
        self.surface_id = surface_id
            .map(str::to_string)
            .or_else(|| self.surface_id.clone());
        self.ensure_render_loop();
        let now_ms = now_ms_f64();
        if let Ok(mut telemetry) = self.telemetry.lock() {
            telemetry.present_submit_count_total =
                telemetry.present_submit_count_total.saturating_add(1);
        }
        if self.should_drop_submitted_frame(frame, now_ms) {
            if let Ok(mut telemetry) = self.telemetry.lock() {
                telemetry.record_drop();
            }
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
                .last_rendered_frame_seq
                .is_some_and(|rendered_seq| frame.frame_seq <= rendered_seq)
            {
                return;
            }
            if state.latest_frame.as_ref().is_some_and(|latest| {
                Some(latest.frame_seq) != state.last_rendered_frame_seq
                    && latest.frame_seq != frame.frame_seq
            }) {
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
                    state.last_rendered_frame_seq = None;
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
        viewport.host_present_submit_count_total = telemetry.present_submit_count_total;
        viewport.host_present_drop_count_total = telemetry.present_drop_count_total;
        viewport.host_present_overwrite_count_total = telemetry.present_overwrite_count_total;
        viewport.host_display_interval_ms = telemetry.display_interval_ms();
        viewport.host_frame_age_budget_ms = Some(telemetry.frame_age_budget_ms());
        viewport.host_descriptor_upload_mode = telemetry.descriptor_upload_mode.clone();
        viewport.host_descriptor_metal_import_count_total =
            telemetry.descriptor_metal_import_count_total;
        viewport.host_descriptor_cpu_upload_count_total =
            telemetry.descriptor_cpu_upload_count_total;
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
}

#[cfg(target_os = "macos")]
impl MacOsVideoPresenter {
    pub(super) fn new(viewport_id: &str, window_label: &str, app_handle: AppHandle) -> Self {
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
            ) {
                self.display_link = Some(display_link);
                return;
            }
        }
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
                    let _ = window.run_on_main_thread(move || {
                        run_layer_present_tick(
                            &window_for_task,
                            &viewport_id,
                            &layer_state,
                            &frame_slot,
                            &telemetry,
                            &render_loop_pending,
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
        self.surface_id = surface_id.map(str::to_string);
        self.ensure_render_loop();
    }

    fn present(&mut self, surface_id: Option<&str>, frame: &XbxEngineRenderFrame) {
        self.surface_id = surface_id
            .map(str::to_string)
            .or_else(|| self.surface_id.clone());
        self.ensure_render_loop();
        if !frame_has_cv_pixelbuffer(frame) {
            return;
        }
        let now_ms = now_ms_f64();
        if self.should_drop_submitted_frame(frame, now_ms) {
            if let Ok(mut telemetry) = self.telemetry.lock() {
                telemetry.present_submit_count_total =
                    telemetry.present_submit_count_total.saturating_add(1);
                telemetry.record_drop();
            }
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
            return;
        };
        telemetry.present_submit_count_total =
            telemetry.present_submit_count_total.saturating_add(1);
        let Ok(mut frame_slot) = self.frame_slot.lock() else {
            return;
        };
        let _ = frame_slot.submit_frame(frame, now_ms, &mut telemetry);
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
        viewport.host_present_submit_count_total = telemetry.present_submit_count_total;
        viewport.host_present_drop_count_total = telemetry.present_drop_count_total;
        viewport.host_present_overwrite_count_total = telemetry.present_overwrite_count_total;
        viewport.host_display_interval_ms = telemetry.display_interval_ms();
        viewport.host_frame_age_budget_ms = Some(telemetry.frame_age_budget_ms());
        viewport.host_descriptor_upload_mode = None;
        viewport.host_descriptor_metal_import_count_total = 0;
        viewport.host_descriptor_cpu_upload_count_total = 0;
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
