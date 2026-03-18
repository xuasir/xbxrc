use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use tauri::{AppHandle, Manager, Window};
use xbxengine::{XbxEngineRenderFrame, XbxEngineRenderPixelData};

mod effects;
mod native_video_policy;
mod presenters;
mod scheduling;
mod types;
#[cfg(target_os = "macos")]
mod wgpu_renderer;

use self::effects::{NoopVideoEffectPipeline, VideoEffectPipeline, WgpuVideoEffectPipeline};
use self::native_video_policy::{resolve_initial_video_pipeline_plan, resolve_video_pipeline_plan};
use self::presenters::{
    resolve_present_kind, MacOsVideoPresenter, MacOsWgpuPresenter, NativeVideoPresenter,
    NativeVideoPresenterKind, NoopVideoPresenter,
};
use self::scheduling::{HostCadenceTelemetry, ScheduledFrameSlot};
use self::types::{
    DecodedVideoSurface, VideoEffectPipelineKind, VideoPlatformCapabilities, VideoPresenterMode,
};

const MAIN_WINDOW_LABEL: &str = "main";
const STREAM_VIEWPORT_ID: &str = "stream-page-video";
const HOST_RENDER_FPS_WINDOW_MS: f64 = 1_000.0;
const HOST_RENDER_MIN_FRAME_AGE_MS: f64 = 24.0;
const HOST_RENDER_MAX_FRAME_AGE_MS: f64 = 75.0;
const HOST_RENDER_FRAME_AGE_MULTIPLIER: f64 = 2.25;

#[derive(Clone, Debug, PartialEq, Eq)]
enum NativeVideoViewportTarget {
    MainWindow,
}

impl NativeVideoViewportTarget {
    fn window_label(&self) -> &str {
        MAIN_WINDOW_LABEL
    }
}

#[derive(Clone, Debug, Default)]
pub struct NativeVideoViewportState {
    pub viewport_id: String,
    pub window_label: Option<String>,
    pub surface_id: Option<String>,
    pub latest_frame_seq: Option<u64>,
    pub latest_frame_width: Option<u32>,
    pub latest_frame_height: Option<u32>,
    pub latest_frame_rendered_at_ms: Option<f64>,
    pub present_count_total: u64,
    pub last_present_kind: Option<String>,
    pub latest_host_present_time_ms: Option<f64>,
    pub host_present_fps: f64,
    pub host_present_submit_count_total: u64,
    pub host_present_drop_count_total: u64,
    pub host_present_overwrite_count_total: u64,
    pub host_display_interval_ms: Option<f64>,
    pub host_frame_age_budget_ms: Option<f64>,
    pub host_descriptor_upload_mode: Option<String>,
    pub host_descriptor_metal_import_count_total: u64,
    pub host_descriptor_cpu_upload_count_total: u64,
}

pub struct NativeVideoRegistry {
    app_handle: Option<AppHandle>,
    platform_capabilities: VideoPlatformCapabilities,
    viewports: HashMap<String, NativeVideoViewportState>,
    presenters: HashMap<String, Box<dyn NativeVideoPresenter>>,
    effect_pipelines: HashMap<String, Box<dyn VideoEffectPipeline>>,
}

impl NativeVideoRegistry {
    pub fn new(app_handle: AppHandle) -> Self {
        Self {
            app_handle: Some(app_handle),
            platform_capabilities: VideoPlatformCapabilities::current(),
            viewports: HashMap::new(),
            presenters: HashMap::new(),
            effect_pipelines: HashMap::new(),
        }
    }

    pub fn attach_viewport(&mut self, viewport_id: &str, surface_id: Option<&str>) -> bool {
        let target = self.resolve_viewport_target(viewport_id);
        let desired_kind =
            resolve_presenter_kind_for_initial_attach(surface_id, self.platform_capabilities);
        let presenter_kind_changed = self
            .presenters
            .get(viewport_id)
            .is_some_and(|presenter| presenter.kind() != desired_kind);
        if presenter_kind_changed {
            if let Some(mut presenter) = self.presenters.remove(viewport_id) {
                presenter.detach();
            }
        }
        let presenter_missing = !self.presenters.contains_key(viewport_id);
        let surface_changed = self
            .viewports
            .get(viewport_id)
            .and_then(|viewport| viewport.surface_id.as_deref())
            != surface_id;
        {
            let entry = self
                .viewports
                .entry(viewport_id.to_string())
                .or_insert_with(|| NativeVideoViewportState {
                    viewport_id: viewport_id.to_string(),
                    ..Default::default()
                });
            entry.surface_id = surface_id.map(str::to_string);
            entry.window_label = Some(target.window_label().to_string());
        }
        if presenter_missing {
            let presenter = self.create_presenter(viewport_id, &target, desired_kind);
            self.presenters.insert(viewport_id.to_string(), presenter);
        }
        let attach_plan =
            resolve_initial_video_pipeline_plan(surface_id, self.platform_capabilities);
        self.ensure_effect_pipeline(viewport_id, attach_plan.effect_pipeline);
        let presenter = match self.presenters.get_mut(viewport_id) {
            Some(presenter) => presenter,
            None => return false,
        };
        if presenter_missing || surface_changed {
            presenter.attach(surface_id);
        }
        presenter_missing || presenter_kind_changed || surface_changed
    }

    pub fn detach_viewport(&mut self, viewport_id: &str) {
        if let Some(mut presenter) = self.presenters.remove(viewport_id) {
            presenter.detach();
        }
        self.effect_pipelines.remove(viewport_id);
        self.viewports.remove(viewport_id);
    }

    /**
     * 当前先把 frame 所有权收归 Tauri 宿主，并记录最近一帧状态。
     * 后续接入 Metal/CALayer 时，直接在这里替换成真实 native presenter。
     */
    pub fn present_frame(
        &mut self,
        viewport_id: &str,
        surface_id: Option<&str>,
        frame: &XbxEngineRenderFrame,
    ) {
        let target = self.resolve_viewport_target(viewport_id);
        let decoded_surface = DecodedVideoSurface::from_render_frame(frame);
        let pipeline_plan =
            resolve_video_pipeline_plan(&decoded_surface, surface_id, self.platform_capabilities);
        let desired_kind =
            resolve_presenter_kind_for_mode(pipeline_plan.presenter_mode, viewport_id, &target);
        let entry = self
            .viewports
            .entry(viewport_id.to_string())
            .or_insert_with(|| NativeVideoViewportState {
                viewport_id: viewport_id.to_string(),
                ..Default::default()
            });
        entry.surface_id = surface_id
            .map(str::to_string)
            .or_else(|| entry.surface_id.clone());
        entry.window_label = Some(target.window_label().to_string());
        entry.latest_frame_seq = Some(frame.frame_seq);
        entry.latest_frame_width = Some(frame.width);
        entry.latest_frame_height = Some(frame.height);
        entry.latest_frame_rendered_at_ms = Some(frame.rendered_at_ms);
        entry.present_count_total = entry.present_count_total.saturating_add(1);
        entry.last_present_kind = Some(resolve_present_kind(frame));

        let presenter_kind_changed = self
            .presenters
            .get(viewport_id)
            .is_some_and(|presenter| presenter.kind() != desired_kind);
        if presenter_kind_changed {
            if let Some(mut presenter) = self.presenters.remove(viewport_id) {
                presenter.detach();
            }
        }
        if !self.presenters.contains_key(viewport_id) {
            let presenter = self.create_presenter(viewport_id, &target, desired_kind);
            self.presenters.insert(viewport_id.to_string(), presenter);
        }
        self.ensure_effect_pipeline(viewport_id, pipeline_plan.effect_pipeline);
        if self
            .effect_pipelines
            .get(viewport_id)
            .is_some_and(|pipeline| !pipeline.can_process(&decoded_surface))
        {
            return;
        }
        let presenter = match self.presenters.get_mut(viewport_id) {
            Some(presenter) => presenter,
            None => return,
        };
        presenter.present(surface_id, frame);
        self.sync_presenter_diagnostics(viewport_id);
    }

    #[allow(dead_code)]
    pub fn snapshot(&self, viewport_id: &str) -> Option<NativeVideoViewportState> {
        let mut snapshot = self.viewports.get(viewport_id).cloned()?;
        if let Some(presenter) = self.presenters.get(viewport_id) {
            presenter.apply_viewport_diagnostics(&mut snapshot);
        }
        Some(snapshot)
    }

    fn resolve_viewport_target(&self, _viewport_id: &str) -> NativeVideoViewportTarget {
        // 当前统一走单窗口宿主；保留 target 抽象，便于后续扩展其他宿主类型。
        NativeVideoViewportTarget::MainWindow
    }

    fn create_presenter(
        &self,
        viewport_id: &str,
        target: &NativeVideoViewportTarget,
        kind: NativeVideoPresenterKind,
    ) -> Box<dyn NativeVideoPresenter> {
        #[cfg(target_os = "macos")]
        {
            if let Some(app_handle) = self.app_handle.clone() {
                if kind == NativeVideoPresenterKind::Wgpu {
                    return Box::new(MacOsWgpuPresenter::new(
                        viewport_id,
                        target.window_label(),
                        app_handle,
                    ));
                }
                return Box::new(MacOsVideoPresenter::new(
                    viewport_id,
                    target.window_label(),
                    app_handle,
                ));
            }
        }
        Box::new(NoopVideoPresenter::new(viewport_id, target.window_label()))
    }

    fn ensure_effect_pipeline(&mut self, viewport_id: &str, desired_kind: VideoEffectPipelineKind) {
        let kind_changed = self
            .effect_pipelines
            .get(viewport_id)
            .is_some_and(|pipeline| pipeline.kind() != desired_kind);
        if kind_changed {
            self.effect_pipelines.remove(viewport_id);
        }
        if !self.effect_pipelines.contains_key(viewport_id) {
            let pipeline: Box<dyn VideoEffectPipeline> = match desired_kind {
                VideoEffectPipelineKind::Noop => Box::new(NoopVideoEffectPipeline::new()),
                VideoEffectPipelineKind::Wgpu => Box::new(WgpuVideoEffectPipeline::new()),
            };
            self.effect_pipelines
                .insert(viewport_id.to_string(), pipeline);
        }
    }

    fn sync_presenter_diagnostics(&mut self, viewport_id: &str) {
        let Some(viewport) = self.viewports.get_mut(viewport_id) else {
            return;
        };
        let Some(presenter) = self.presenters.get(viewport_id) else {
            return;
        };
        presenter.apply_viewport_diagnostics(viewport);
    }
}

impl Default for NativeVideoRegistry {
    fn default() -> Self {
        Self {
            app_handle: None,
            platform_capabilities: VideoPlatformCapabilities::current(),
            viewports: HashMap::new(),
            presenters: HashMap::new(),
            effect_pipelines: HashMap::new(),
        }
    }
}

pub type NativeVideoRegistryRef = Arc<Mutex<NativeVideoRegistry>>;

fn resolve_presenter_kind_for_initial_attach(
    surface_id: Option<&str>,
    capabilities: VideoPlatformCapabilities,
) -> NativeVideoPresenterKind {
    let plan = resolve_initial_video_pipeline_plan(surface_id, capabilities);
    resolve_presenter_kind_for_mode(
        plan.presenter_mode,
        STREAM_VIEWPORT_ID,
        &NativeVideoViewportTarget::MainWindow,
    )
}

fn resolve_presenter_kind_for_mode(
    presenter_mode: VideoPresenterMode,
    _viewport_id: &str,
    _target: &NativeVideoViewportTarget,
) -> NativeVideoPresenterKind {
    match presenter_mode {
        VideoPresenterMode::NativeDirect => NativeVideoPresenterKind::PlatformNative,
        VideoPresenterMode::GpuDirect => NativeVideoPresenterKind::Wgpu,
    }
}

pub fn configure_main_window_video_host(app_handle: &AppHandle) {
    #[cfg(target_os = "macos")]
    configure_macos_main_window_video_host(app_handle);
}

#[cfg(target_os = "macos")]
fn configure_macos_main_window_video_host(app_handle: &AppHandle) {
    configure_macos_window_video_host(app_handle, MAIN_WINDOW_LABEL, true);
}

#[cfg(target_os = "macos")]
fn configure_macos_window_video_host(
    app_handle: &AppHandle,
    window_label: &str,
    transparent_window: bool,
) {
    let Some(window) = app_handle.get_webview_window(window_label) else {
        return;
    };
    if let Err(error) = window.with_webview(move |webview| unsafe {
        use objc2::runtime::{AnyClass, AnyObject};
        use objc2::{msg_send, rc::autoreleasepool};
        use objc2_foundation::NSString;
        use std::ffi::CStr;

        fn ns_color_class() -> Option<&'static AnyClass> {
            let class_name = CStr::from_bytes_with_nul(b"NSColor\0").ok()?;
            AnyClass::get(class_name)
        }

        fn ns_number_class() -> Option<&'static AnyClass> {
            let class_name = CStr::from_bytes_with_nul(b"NSNumber\0").ok()?;
            AnyClass::get(class_name)
        }

        autoreleasepool(|_| {
            let webview_ptr = webview.inner().cast::<AnyObject>();
            let window_ptr = webview.ns_window().cast::<AnyObject>();
            let Some(color_class) = ns_color_class() else {
                return;
            };
            let clear_color: *mut AnyObject = msg_send![color_class, clearColor];
            if clear_color.is_null() {
                return;
            }
            let black_color: *mut AnyObject = msg_send![color_class, blackColor];
            if black_color.is_null() {
                return;
            }

            // 主窗口承载透明 Web UI；独立视频窗则让 webview 透明、contentView 保持黑底。
            let _: () = msg_send![webview_ptr, setOpaque: false];
            let _: () = msg_send![webview_ptr, setBackgroundColor: clear_color];
            let _: () = msg_send![window_ptr, setOpaque: !transparent_window];
            let _: () = msg_send![
                window_ptr,
                setBackgroundColor: if transparent_window { clear_color } else { black_color }
            ];

            if let Some(number_class) = ns_number_class() {
                let no_number: *mut AnyObject = msg_send![number_class, numberWithBool: false];
                if !no_number.is_null() {
                    let key = NSString::from_str("drawsBackground");
                    let key_ref: &NSString = key.as_ref();
                    let _: () = msg_send![webview_ptr, setValue: no_number, forKey: key_ref];
                }
            }

            // macOS 12+ 的 overscroll 区域也需要透明，否则边缘仍会露出系统底色。
            let _: () = msg_send![webview_ptr, setUnderPageBackgroundColor: clear_color];

            let content_view: *mut AnyObject = msg_send![window_ptr, contentView];
            if !content_view.is_null() {
                let _: () = msg_send![content_view, setWantsLayer: true];
                let content_layer: *mut AnyObject = msg_send![content_view, layer];
                if !content_layer.is_null() {
                    // 透明主窗口需要自己承担裁剪，否则 macOS 原生圆角会被透明内容层“抹平”。
                    let corner_radius = if transparent_window { 14.0f64 } else { 0.0f64 };
                    let _: () = msg_send![content_layer, setCornerRadius: corner_radius];
                    let _: () = msg_send![content_layer, setMasksToBounds: transparent_window];
                    let _: () = msg_send![
                        content_layer,
                        setBackgroundColor: if transparent_window { clear_color } else { black_color }
                    ];
                }
                let _: () = msg_send![content_view, setNeedsDisplay: true];
            }
        });
    }) {
        log::warn!(
            "[native_video][macos] failed to configure window video host label={} error={}",
            window_label,
            error
        );
    }
}

#[cfg(target_os = "macos")]
#[derive(Default)]
pub(super) struct MacOsWgpuState {
    renderer: Option<wgpu_renderer::WgpuFrameRenderer>,
    latest_frame: Option<XbxEngineRenderFrame>,
    last_rendered_frame_seq: Option<u64>,
    last_surface_size: Option<(u32, u32)>,
    host_view_ptr: Option<*mut objc2::runtime::AnyObject>,
    host_view_managed: bool,
    render_loop_started: bool,
    init_failed_logged: bool,
}

#[cfg(target_os = "macos")]
#[derive(Default)]
pub(super) struct MacOsWgpuTelemetry {
    latest_present_time_ms: Option<f64>,
    recent_present_times_ms: VecDeque<f64>,
    recent_display_tick_times_ms: VecDeque<f64>,
    present_submit_count_total: u64,
    present_drop_count_total: u64,
    present_overwrite_count_total: u64,
    descriptor_upload_mode: Option<String>,
    descriptor_metal_import_count_total: u64,
    descriptor_cpu_upload_count_total: u64,
}

#[cfg(target_os = "macos")]
impl MacOsWgpuTelemetry {
    fn record_display_tick(&mut self, now_ms: f64) {
        self.recent_display_tick_times_ms.push_back(now_ms);
        self.trim_display_ticks(now_ms);
    }

    fn record_present(&mut self, now_ms: f64) {
        self.latest_present_time_ms = Some(now_ms);
        self.recent_present_times_ms.push_back(now_ms);
        self.trim_recent(now_ms);
    }

    fn record_drop(&mut self) {
        self.present_drop_count_total = self.present_drop_count_total.saturating_add(1);
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
        self.recent_present_times_ms.clear();
        self.recent_display_tick_times_ms.clear();
        self.descriptor_upload_mode = None;
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

#[cfg(target_os = "macos")]
#[derive(Default)]
pub(super) struct MacOsLayerState {
    display_layer_ptr: Option<*mut objc2::runtime::AnyObject>,
    first_present_logged: bool,
}

#[cfg(target_os = "macos")]
unsafe impl Send for MacOsWgpuState {}

#[cfg(target_os = "macos")]
struct PendingFlagGuard {
    pending: Arc<AtomicBool>,
}

#[cfg(target_os = "macos")]
impl PendingFlagGuard {
    fn new(pending: Arc<AtomicBool>) -> Self {
        Self { pending }
    }
}

#[cfg(target_os = "macos")]
impl Drop for PendingFlagGuard {
    fn drop(&mut self) {
        self.pending.store(false, Ordering::Relaxed);
    }
}

#[cfg(target_os = "macos")]
pub(super) fn run_wgpu_render_tick(
    window: &Window,
    app_handle: &AppHandle,
    viewport_id: &str,
    renderer_state: &Arc<Mutex<MacOsWgpuState>>,
    telemetry: &Arc<Mutex<MacOsWgpuTelemetry>>,
    render_loop_pending: &Arc<AtomicBool>,
) {
    let _pending_guard = PendingFlagGuard::new(render_loop_pending.clone());
    let Ok(mut state) = renderer_state.lock() else {
        return;
    };
    let Some(host_view_ptr) = ensure_wgpu_host_view(app_handle, window, &mut state) else {
        return;
    };
    let Some((surface_width, surface_height)) =
        current_wgpu_surface_size_pixels(window, host_view_ptr)
    else {
        return;
    };
    let size_changed = state.last_surface_size != Some((surface_width, surface_height));
    if state.renderer.is_none() {
        match pollster::block_on(wgpu_renderer::WgpuFrameRenderer::new(
            host_view_ptr.cast::<std::ffi::c_void>(),
            surface_width,
            surface_height,
        )) {
            Ok(renderer) => {
                log::info!(
                    "[native_video][wgpu] renderer created for viewport={} window={}",
                    viewport_id,
                    window.label()
                );
                state.renderer = Some(renderer);
                state.last_surface_size = Some((surface_width, surface_height));
            }
            Err(error) => {
                if !state.init_failed_logged {
                    state.init_failed_logged = true;
                    log::warn!(
                        "[native_video][wgpu] failed to create renderer for viewport={} window={} error={}",
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
            != state.last_rendered_frame_seq;
    let latest_frame = state.latest_frame.clone();
    let rendered_seq_before = state.last_rendered_frame_seq;
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
                state.last_rendered_frame_seq = Some(frame.frame_seq);
                state.latest_frame = None;
                if let Ok(mut telemetry) = telemetry.lock() {
                    telemetry.record_drop();
                }
                log::debug!(
                    "[native_video][wgpu] drop stale frame viewport={} window={} frame_seq={} age_ms={:.2}",
                    viewport_id,
                    window.label(),
                    frame.frame_seq,
                    now_ms - frame.rendered_at_ms
                );
                return;
            }
            let rendered_seq = frame.frame_seq;
            renderer.update_frame(frame);
            if let Err(error) = renderer.render() {
                log::warn!(
                    "[native_video][wgpu] render failed for viewport={} window={} error={}",
                    viewport_id,
                    window.label(),
                    error
                );
            } else {
                let descriptor_upload = renderer.descriptor_upload_telemetry();
                state.last_rendered_frame_seq = Some(rendered_seq);
                if let Ok(mut telemetry) = telemetry.lock() {
                    telemetry.record_present(now_ms);
                    telemetry.descriptor_upload_mode = descriptor_upload.last_mode;
                    telemetry.descriptor_metal_import_count_total =
                        descriptor_upload.metal_import_count_total;
                    telemetry.descriptor_cpu_upload_count_total =
                        descriptor_upload.cpu_upload_count_total;
                }
            }
        }
    } else if rendered_seq_before.is_none() && size_changed {
        let _ = renderer.render();
    }
}

#[cfg(target_os = "macos")]
fn current_wgpu_surface_size_pixels(
    window: &Window,
    host_view_ptr: *mut objc2::runtime::AnyObject,
) -> Option<(u32, u32)> {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    use objc2_app_kit::NSView;
    use objc2_foundation::NSRect;

    unsafe {
        let host_view = host_view_ptr.cast::<NSView>().as_ref()?;
        let backing_bounds: NSRect = msg_send![host_view, convertRectToBacking: {
            let bounds: NSRect = msg_send![host_view, bounds];
            bounds
        }];
        let width = backing_bounds.size.width.round().max(1.0) as u32;
        let height = backing_bounds.size.height.round().max(1.0) as u32;
        if width > 0 && height > 0 {
            return Some((width, height));
        }

        let ns_view_ptr = window.ns_view().ok()? as *mut AnyObject;
        let ns_window_ptr: *mut AnyObject = msg_send![ns_view_ptr, window];
        if ns_window_ptr.is_null() {
            return None;
        }
        let scale_factor: f64 = msg_send![ns_window_ptr, backingScaleFactor];
        let bounds: NSRect = msg_send![host_view, bounds];
        Some((
            (bounds.size.width * scale_factor).round().max(1.0) as u32,
            (bounds.size.height * scale_factor).round().max(1.0) as u32,
        ))
    }
}

#[cfg(target_os = "macos")]
fn ensure_wgpu_host_view(
    app_handle: &AppHandle,
    window: &Window,
    state: &mut MacOsWgpuState,
) -> Option<*mut objc2::runtime::AnyObject> {
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
    use std::sync::Arc;

    use objc2::runtime::{AnyClass, AnyObject};
    use objc2::{msg_send, rc::autoreleasepool};
    use objc2_app_kit::NSView;
    use objc2_foundation::NSRect;
    use std::ffi::CStr;

    let window_label = window.label();

    // 独立视频窗创建明确的原生 host view，先验证它是否真的进入了可见层。
    if window_label != MAIN_WINDOW_LABEL {
        if let Some(host_view_ptr) = state.host_view_ptr {
            return Some(host_view_ptr);
        }
        let created_view_addr = Arc::new(AtomicUsize::new(0));
        let created_view_addr_for_task = created_view_addr.clone();
        let ns_view_ptr = window.ns_view().ok()? as *mut AnyObject;
        autoreleasepool(|_| unsafe {
            let ns_window_ptr: *mut AnyObject = msg_send![ns_view_ptr, window];
            if ns_window_ptr.is_null() {
                return;
            }
            let content_view_ptr: *mut AnyObject = msg_send![ns_window_ptr, contentView];
            if content_view_ptr.is_null() {
                return;
            }
            let Some(content_view) = content_view_ptr.cast::<NSView>().as_ref() else {
                return;
            };
            let _: () = msg_send![content_view_ptr, setWantsLayer: true];
            let class_name = match CStr::from_bytes_with_nul(b"NSView\0") {
                Ok(name) => name,
                Err(_) => return,
            };
            let Some(view_class) = AnyClass::get(class_name) else {
                return;
            };
            let bounds: NSRect = msg_send![content_view, bounds];
            let host_view: *mut AnyObject = msg_send![view_class, alloc];
            let host_view: *mut AnyObject = msg_send![host_view, initWithFrame: bounds];
            if host_view.is_null() {
                return;
            }

            const NS_VIEW_WIDTH_SIZABLE: usize = 1 << 1;
            const NS_VIEW_HEIGHT_SIZABLE: usize = 1 << 4;

            let _: () = msg_send![host_view, setWantsLayer: true];
            let _: () = msg_send![
                host_view,
                setAutoresizingMask: NS_VIEW_WIDTH_SIZABLE | NS_VIEW_HEIGHT_SIZABLE
            ];
            let _: () = msg_send![content_view_ptr, addSubview: host_view];
            let _: () = msg_send![host_view, setNeedsDisplay: true];
            created_view_addr_for_task.store(host_view as usize, AtomicOrdering::Relaxed);
        });
        let created_view_ptr = created_view_addr.load(AtomicOrdering::Relaxed) as *mut AnyObject;
        if created_view_ptr.is_null() {
            return None;
        }
        state.host_view_ptr = Some(created_view_ptr);
        state.host_view_managed = true;
        return state.host_view_ptr;
    }

    if let Some(host_view_ptr) = state.host_view_ptr {
        if !state.host_view_managed {
            return Some(host_view_ptr);
        }
        let host_view_addr = host_view_ptr as usize;
        let webview_window = app_handle.get_webview_window(window_label)?;
        let _ = webview_window.with_webview(move |webview| unsafe {
            let webview_ptr = webview.inner().cast::<AnyObject>();
            let Some(webview) = webview_ptr.cast::<NSView>().as_ref() else {
                return;
            };
            sync_wgpu_host_view_frame(webview, host_view_addr as *mut AnyObject);
        });
        return Some(host_view_ptr);
    }

    let created_view_addr = Arc::new(AtomicUsize::new(0));
    let webview_window = app_handle.get_webview_window(window_label)?;
    let created_view_addr_for_task = created_view_addr.clone();
    let _ = webview_window.with_webview(move |webview| unsafe {
        autoreleasepool(|_| {
            let webview_ptr = webview.inner().cast::<AnyObject>();
            let Some(webview) = webview_ptr.cast::<NSView>().as_ref() else {
                return;
            };
            let superview_ptr: *mut AnyObject = msg_send![webview, superview];
            if superview_ptr.is_null() {
                return;
            }
            let superview: &NSView = match superview_ptr.cast::<NSView>().as_ref() {
                Some(view) => view,
                None => return,
            };
            let class_name = match CStr::from_bytes_with_nul(b"NSView\0") {
                Ok(name) => name,
                Err(_) => return,
            };
            let Some(view_class) = AnyClass::get(class_name) else {
                return;
            };
            let bounds: NSRect = msg_send![superview, bounds];
            let host_view: *mut AnyObject = msg_send![view_class, alloc];
            let host_view: *mut AnyObject = msg_send![host_view, initWithFrame: bounds];
            if host_view.is_null() {
                return;
            }

            const NS_VIEW_WIDTH_SIZABLE: usize = 1 << 1;
            const NS_VIEW_HEIGHT_SIZABLE: usize = 1 << 4;
            const NS_WINDOW_ABOVE: isize = 1;

            let _: () = msg_send![host_view, setWantsLayer: true];
            let layer_ptr: *mut AnyObject = msg_send![host_view, layer];
            if !layer_ptr.is_null() {
                let color_class_name = match CStr::from_bytes_with_nul(b"NSColor\0") {
                    Ok(name) => name,
                    Err(_) => return,
                };
                if let Some(color_class) = AnyClass::get(color_class_name) {
                    let black_color: *mut AnyObject = msg_send![color_class, blackColor];
                    if !black_color.is_null() {
                        let _: () = msg_send![layer_ptr, setBackgroundColor: black_color];
                    }
                }
            }
            let _: () = msg_send![host_view, setAlphaValue: 1.0f64];
            let _: () = msg_send![host_view, setAutoresizingMask: NS_VIEW_WIDTH_SIZABLE | NS_VIEW_HEIGHT_SIZABLE];
            let _: () = msg_send![superview_ptr, addSubview: host_view, positioned: NS_WINDOW_ABOVE, relativeTo: webview_ptr];
            sync_wgpu_host_view_frame(webview, host_view);
            created_view_addr_for_task.store(host_view as usize, AtomicOrdering::Relaxed);
        });
    });

    let created_view_ptr = created_view_addr.load(AtomicOrdering::Relaxed) as *mut AnyObject;
    if !created_view_ptr.is_null() {
        state.host_view_ptr = Some(created_view_ptr);
        state.host_view_managed = true;
        log::info!(
            "[native_video][wgpu] host view created for window={}",
            window_label
        );
    }
    state.host_view_ptr
}

#[cfg(target_os = "macos")]
unsafe fn sync_wgpu_host_view_frame(
    webview: &objc2_app_kit::NSView,
    host_view_ptr: *mut objc2::runtime::AnyObject,
) {
    use objc2::msg_send;
    use objc2_app_kit::NSView;
    use objc2_foundation::NSRect;

    let superview_ptr: *mut objc2::runtime::AnyObject = msg_send![webview, superview];
    if superview_ptr.is_null() {
        return;
    }
    let Some(superview) = superview_ptr.cast::<NSView>().as_ref() else {
        return;
    };
    let bounds: NSRect = msg_send![superview, bounds];
    let _: () = msg_send![host_view_ptr, setFrame: bounds];
    let window_ptr: *mut objc2::runtime::AnyObject = msg_send![superview, window];
    if !window_ptr.is_null() {
        let scale_factor: f64 = msg_send![window_ptr, backingScaleFactor];
        let layer_ptr: *mut objc2::runtime::AnyObject = msg_send![host_view_ptr, layer];
        if !layer_ptr.is_null() {
            let _: () = msg_send![layer_ptr, setContentsScale: scale_factor];
        }
    }
    let _: () = msg_send![host_view_ptr, setHidden: false];
    let _: () = msg_send![host_view_ptr, setNeedsDisplay: true];
}

#[cfg(target_os = "macos")]
pub(super) fn drop_wgpu_host_view(host_view_ptr: *mut objc2::runtime::AnyObject) {
    use objc2::{msg_send, rc::autoreleasepool};

    autoreleasepool(|_| unsafe {
        let _: () = msg_send![host_view_ptr, removeFromSuperview];
        let _: () = msg_send![host_view_ptr, release];
    });
}

#[cfg(target_os = "macos")]
struct MacOsDisplayLinkContext {
    viewport_id: String,
    window_label: String,
    app_handle: AppHandle,
    renderer_state: Arc<Mutex<MacOsWgpuState>>,
    telemetry: Arc<Mutex<MacOsWgpuTelemetry>>,
    render_loop_pending: Arc<AtomicBool>,
}

#[cfg(target_os = "macos")]
pub(super) struct MacOsDisplayLinkHandle {
    display_link_ref: *mut std::ffi::c_void,
    context_ptr: *mut MacOsDisplayLinkContext,
}

#[cfg(target_os = "macos")]
unsafe impl Send for MacOsDisplayLinkHandle {}

#[cfg(target_os = "macos")]
impl MacOsDisplayLinkHandle {
    fn start(
        viewport_id: String,
        window_label: String,
        app_handle: AppHandle,
        renderer_state: Arc<Mutex<MacOsWgpuState>>,
        telemetry: Arc<Mutex<MacOsWgpuTelemetry>>,
        render_loop_pending: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        let context = Box::new(MacOsDisplayLinkContext {
            viewport_id,
            window_label,
            app_handle,
            renderer_state,
            telemetry,
            render_loop_pending,
        });
        let context_ptr = Box::into_raw(context);
        let mut display_link_ref: *mut std::ffi::c_void = std::ptr::null_mut();
        let status = unsafe { CVDisplayLinkCreateWithActiveCGDisplays(&mut display_link_ref) };
        if status != 0 || display_link_ref.is_null() {
            unsafe {
                drop(Box::from_raw(context_ptr));
            }
            return Err(format!("xbxEngineCreateDisplayLinkFailed:{status}"));
        }
        let status = unsafe {
            CVDisplayLinkSetOutputCallback(
                display_link_ref,
                macos_display_link_callback,
                context_ptr.cast(),
            )
        };
        if status != 0 {
            unsafe {
                CFRelease(display_link_ref);
                drop(Box::from_raw(context_ptr));
            }
            return Err(format!("xbxEngineSetDisplayLinkCallbackFailed:{status}"));
        }
        let status = unsafe { CVDisplayLinkStart(display_link_ref) };
        if status != 0 {
            unsafe {
                CFRelease(display_link_ref);
                drop(Box::from_raw(context_ptr));
            }
            return Err(format!("xbxEngineStartDisplayLinkFailed:{status}"));
        }
        Ok(Self {
            display_link_ref,
            context_ptr,
        })
    }

    fn stop(self) {
        unsafe {
            let _ = CVDisplayLinkStop(self.display_link_ref);
            CFRelease(self.display_link_ref);
            drop(Box::from_raw(self.context_ptr));
        }
    }
}

#[cfg(target_os = "macos")]
struct MacOsLayerDisplayLinkContext {
    viewport_id: String,
    window_label: String,
    app_handle: AppHandle,
    layer_state: Arc<Mutex<MacOsLayerState>>,
    frame_slot: Arc<Mutex<ScheduledFrameSlot>>,
    telemetry: Arc<Mutex<HostCadenceTelemetry>>,
    render_loop_pending: Arc<AtomicBool>,
}

#[cfg(target_os = "macos")]
pub(super) struct MacOsLayerDisplayLinkHandle {
    display_link_ref: *mut std::ffi::c_void,
    context_ptr: *mut MacOsLayerDisplayLinkContext,
}

#[cfg(target_os = "macos")]
unsafe impl Send for MacOsLayerDisplayLinkHandle {}

#[cfg(target_os = "macos")]
impl MacOsLayerDisplayLinkHandle {
    fn start(
        viewport_id: String,
        window_label: String,
        app_handle: AppHandle,
        layer_state: Arc<Mutex<MacOsLayerState>>,
        frame_slot: Arc<Mutex<ScheduledFrameSlot>>,
        telemetry: Arc<Mutex<HostCadenceTelemetry>>,
        render_loop_pending: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        let context = Box::new(MacOsLayerDisplayLinkContext {
            viewport_id,
            window_label,
            app_handle,
            layer_state,
            frame_slot,
            telemetry,
            render_loop_pending,
        });
        let context_ptr = Box::into_raw(context);
        let mut display_link_ref: *mut std::ffi::c_void = std::ptr::null_mut();
        let status = unsafe { CVDisplayLinkCreateWithActiveCGDisplays(&mut display_link_ref) };
        if status != 0 || display_link_ref.is_null() {
            unsafe {
                drop(Box::from_raw(context_ptr));
            }
            return Err(format!("xbxEngineCreateDisplayLinkFailed:{status}"));
        }
        let status = unsafe {
            CVDisplayLinkSetOutputCallback(
                display_link_ref,
                macos_layer_display_link_callback,
                context_ptr.cast(),
            )
        };
        if status != 0 {
            unsafe {
                CFRelease(display_link_ref);
                drop(Box::from_raw(context_ptr));
            }
            return Err(format!(
                "xbxEngineSetLayerDisplayLinkCallbackFailed:{status}"
            ));
        }
        let status = unsafe { CVDisplayLinkStart(display_link_ref) };
        if status != 0 {
            unsafe {
                CFRelease(display_link_ref);
                drop(Box::from_raw(context_ptr));
            }
            return Err(format!("xbxEngineStartLayerDisplayLinkFailed:{status}"));
        }
        Ok(Self {
            display_link_ref,
            context_ptr,
        })
    }

    fn stop(self) {
        unsafe {
            let _ = CVDisplayLinkStop(self.display_link_ref);
            CFRelease(self.display_link_ref);
            drop(Box::from_raw(self.context_ptr));
        }
    }
}

#[cfg(target_os = "macos")]
unsafe extern "C" fn macos_display_link_callback(
    _display_link: *mut std::ffi::c_void,
    _in_now: *const std::ffi::c_void,
    _in_output_time: *const std::ffi::c_void,
    _flags_in: u64,
    _flags_out: *mut u64,
    display_link_context: *mut std::ffi::c_void,
) -> i32 {
    let Some(context) = (display_link_context as *mut MacOsDisplayLinkContext).as_ref() else {
        return 0;
    };
    if context.render_loop_pending.swap(true, Ordering::Relaxed) {
        return 0;
    }
    let Some(window) = context.app_handle.get_window(&context.window_label) else {
        context.render_loop_pending.store(false, Ordering::Relaxed);
        return 0;
    };
    let renderer_state = context.renderer_state.clone();
    let telemetry = context.telemetry.clone();
    let render_loop_pending = context.render_loop_pending.clone();
    let viewport_id = context.viewport_id.clone();
    let app_handle = context.app_handle.clone();
    let window_for_task = window.clone();
    let _ = window.run_on_main_thread(move || {
        run_wgpu_render_tick(
            &window_for_task,
            &app_handle,
            &viewport_id,
            &renderer_state,
            &telemetry,
            &render_loop_pending,
        );
    });
    0
}

#[cfg(target_os = "macos")]
unsafe extern "C" fn macos_layer_display_link_callback(
    _display_link: *mut std::ffi::c_void,
    _in_now: *const std::ffi::c_void,
    _in_output_time: *const std::ffi::c_void,
    _flags_in: u64,
    _flags_out: *mut u64,
    display_link_context: *mut std::ffi::c_void,
) -> i32 {
    let Some(context) = (display_link_context as *mut MacOsLayerDisplayLinkContext).as_ref() else {
        return 0;
    };
    if context.render_loop_pending.swap(true, Ordering::Relaxed) {
        return 0;
    }
    let Some(window) = context.app_handle.get_window(&context.window_label) else {
        context.render_loop_pending.store(false, Ordering::Relaxed);
        return 0;
    };
    let layer_state = context.layer_state.clone();
    let frame_slot = context.frame_slot.clone();
    let telemetry = context.telemetry.clone();
    let render_loop_pending = context.render_loop_pending.clone();
    let viewport_id = context.viewport_id.clone();
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
    0
}

#[cfg(target_os = "macos")]
#[link(name = "CoreVideo", kind = "framework")]
extern "C" {
    fn CVDisplayLinkCreateWithActiveCGDisplays(display_link_out: *mut *mut std::ffi::c_void)
        -> i32;
    fn CVDisplayLinkSetOutputCallback(
        display_link: *mut std::ffi::c_void,
        callback: unsafe extern "C" fn(
            *mut std::ffi::c_void,
            *const std::ffi::c_void,
            *const std::ffi::c_void,
            u64,
            *mut u64,
            *mut std::ffi::c_void,
        ) -> i32,
        user_info: *mut std::ffi::c_void,
    ) -> i32;
    fn CVDisplayLinkStart(display_link: *mut std::ffi::c_void) -> i32;
    fn CVDisplayLinkStop(display_link: *mut std::ffi::c_void) -> i32;
}

#[cfg(target_os = "macos")]
#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(cf: *mut std::ffi::c_void);
}

#[cfg(target_os = "macos")]
unsafe impl Send for MacOsLayerState {}

pub(super) fn now_ms_f64() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0)
}

fn calculate_recent_fps(recent_times_ms: &VecDeque<f64>) -> f64 {
    if recent_times_ms.len() < 2 {
        return recent_times_ms.len() as f64;
    }
    let Some(first) = recent_times_ms.front() else {
        return 0.0;
    };
    let Some(last) = recent_times_ms.back() else {
        return 0.0;
    };
    let elapsed_ms = (last - first).max(1.0);
    ((recent_times_ms.len() - 1) as f64) * 1000.0 / elapsed_ms
}

fn calculate_recent_interval_ms(recent_times_ms: &VecDeque<f64>) -> Option<f64> {
    if recent_times_ms.len() < 2 {
        return None;
    }
    let first = *recent_times_ms.front()?;
    let last = *recent_times_ms.back()?;
    let elapsed_ms = (last - first).max(1.0);
    Some(elapsed_ms / (recent_times_ms.len() - 1) as f64)
}

#[cfg(target_os = "macos")]
pub(super) fn run_layer_present_tick(
    window: &Window,
    viewport_id: &str,
    layer_state: &Arc<Mutex<MacOsLayerState>>,
    frame_slot: &Arc<Mutex<ScheduledFrameSlot>>,
    telemetry: &Arc<Mutex<HostCadenceTelemetry>>,
    render_loop_pending: &Arc<AtomicBool>,
) {
    let _pending_guard = PendingFlagGuard::new(render_loop_pending.clone());
    let now_ms = now_ms_f64();
    let Ok(mut telemetry_state) = telemetry.lock() else {
        return;
    };
    telemetry_state.record_display_tick(now_ms);
    let Some(frame) = ({
        let Ok(mut frame_slot_state) = frame_slot.lock() else {
            return;
        };
        frame_slot_state.take_ready_frame(now_ms, &mut telemetry_state)
    }) else {
        return;
    };
    let XbxEngineRenderPixelData::Descriptor { handle } = &frame.pixel_data else {
        return;
    };
    let Some(descriptor) = handle
        .as_ref()
        .downcast_ref::<xbxengine::MacOsCVPixelBufferDescriptor>()
    else {
        return;
    };
    if descriptor.ptr.is_null() {
        return;
    }
    let Ok(mut layer_state) = layer_state.lock() else {
        return;
    };
    let Some(layer_ptr) = ensure_display_layer(window, &mut layer_state) else {
        return;
    };
    if !layer_state.first_present_logged {
        layer_state.first_present_logged = true;
        log::info!(
            "[native_video][macos] first layer present for viewport={} window={}",
            viewport_id,
            window.label()
        );
    }
    present_cv_pixelbuffer(layer_ptr, descriptor.ptr, frame.frame_seq);
    telemetry_state.record_present(now_ms);
}

#[cfg(target_os = "macos")]
pub(super) fn ensure_display_layer(
    window: &Window,
    state: &mut MacOsLayerState,
) -> Option<*mut objc2::runtime::AnyObject> {
    use objc2::runtime::{AnyClass, AnyObject};
    use objc2::{msg_send, rc::autoreleasepool};
    use objc2_app_kit::NSView;
    use objc2_foundation::{NSRect, NSString};
    use std::ffi::CStr;

    let ns_view_ptr = window.ns_view().ok()? as *mut AnyObject;
    let view: &NSView = unsafe { ns_view_ptr.cast::<NSView>().as_ref()? };

    if let Some(ptr) = state.display_layer_ptr {
        autoreleasepool(|_| unsafe {
            let bounds: NSRect = msg_send![view, bounds];
            let ns_window: *mut AnyObject = msg_send![view, window];
            if !ns_window.is_null() {
                let scale_factor: f64 = msg_send![ns_window, backingScaleFactor];
                let _: () = msg_send![ptr, setContentsScale: scale_factor];
            }
            let _: () = msg_send![ptr, setFrame: bounds];
            let _: () = msg_send![ptr, setNeedsLayout];
        });
        return Some(ptr);
    }

    autoreleasepool(|_| unsafe {
        let _: () = msg_send![view, setWantsLayer: true];
        let view_layer: *mut AnyObject = msg_send![view, layer];
        if view_layer.is_null() {
            return;
        }
        let class_name = match CStr::from_bytes_with_nul(b"AVSampleBufferDisplayLayer\0") {
            Ok(name) => name,
            Err(_) => return,
        };
        let layer_class = match AnyClass::get(class_name) {
            Some(class) => class,
            None => return,
        };
        let layer: *mut AnyObject = msg_send![layer_class, alloc];
        let layer: *mut AnyObject = msg_send![layer, init];
        if layer.is_null() {
            return;
        }

        let gravity = NSString::from_str("AVLayerVideoGravityResizeAspect");
        let gravity_ref: &NSString = gravity.as_ref();
        let _: () = msg_send![layer, setVideoGravity: gravity_ref];

        let ns_window: *mut AnyObject = msg_send![view, window];
        if !ns_window.is_null() {
            let scale_factor: f64 = msg_send![ns_window, backingScaleFactor];
            let _: () = msg_send![layer, setContentsScale: scale_factor];
            let _: () = msg_send![view_layer, setContentsScale: scale_factor];
        }

        let bounds: NSRect = msg_send![view, bounds];
        let _: () = msg_send![layer, setFrame: bounds];
        // 视频层强制插到底部，webview 透明后即可在其上方继续承载菜单与面板。
        let _: () = msg_send![view_layer, insertSublayer: layer, atIndex: 0usize];
        let _: () = msg_send![layer, setNeedsLayout];

        state.display_layer_ptr = Some(layer);
        log::info!("[native_video][macos] display layer created");
    });

    state.display_layer_ptr
}

#[cfg(target_os = "macos")]
pub(super) fn drop_display_layer(window: &Window, state: &mut MacOsLayerState) {
    use objc2::runtime::AnyObject;
    use objc2::{msg_send, rc::autoreleasepool};
    use objc2_app_kit::NSView;

    let Some(layer_ptr) = state.display_layer_ptr.take() else {
        return;
    };
    let ns_view_ptr = match window.ns_view() {
        Ok(ptr) => ptr as *mut AnyObject,
        Err(_) => return,
    };
    let view: &NSView = unsafe { ns_view_ptr.cast::<NSView>().as_ref().unwrap() };

    autoreleasepool(|_| unsafe {
        let _: () = msg_send![layer_ptr, removeFromSuperlayer];
        let _: () = msg_send![layer_ptr, release];
        let _: () = msg_send![view, setNeedsDisplay: true];
    });
}

#[cfg(target_os = "macos")]
pub(super) fn present_cv_pixelbuffer(
    layer_ptr: *mut objc2::runtime::AnyObject,
    buffer_ptr: *mut std::ffi::c_void,
    frame_seq: u64,
) {
    use objc2::{msg_send, rc::autoreleasepool};
    use std::ffi::c_void;
    use std::ptr;

    #[repr(C)]
    struct CMTime {
        value: i64,
        timescale: i32,
        flags: u32,
        epoch: i64,
    }

    #[repr(C)]
    struct CMSampleTimingInfo {
        duration: CMTime,
        presentation_time_stamp: CMTime,
        decode_time_stamp: CMTime,
    }

    const K_CM_TIME_INVALID: CMTime = CMTime {
        value: 0,
        timescale: 0,
        flags: 0,
        epoch: 0,
    };
    const K_CM_TIME_FLAGS_VALID: u32 = 1;
    const DEFAULT_TIMESCALE: i32 = 60;

    type OSStatus = i32;

    #[link(name = "CoreMedia", kind = "framework")]
    extern "C" {
        fn CMVideoFormatDescriptionCreateForImageBuffer(
            allocator: *const c_void,
            image_buffer: *mut c_void,
            format_description_out: *mut *mut c_void,
        ) -> OSStatus;
        fn CMSampleBufferCreateForImageBuffer(
            allocator: *const c_void,
            image_buffer: *mut c_void,
            data_ready: bool,
            make_data_ready_callback: *const c_void,
            make_data_ready_refcon: *const c_void,
            format_description: *mut c_void,
            sample_timing: *const CMSampleTimingInfo,
            sample_buffer_out: *mut *mut c_void,
        ) -> OSStatus;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFRelease(cf: *mut c_void);
    }

    #[link(name = "AVFoundation", kind = "framework")]
    extern "C" {}

    #[link(name = "QuartzCore", kind = "framework")]
    extern "C" {}

    autoreleasepool(|_| unsafe {
        let mut format_desc: *mut c_void = ptr::null_mut();
        let status =
            CMVideoFormatDescriptionCreateForImageBuffer(ptr::null(), buffer_ptr, &mut format_desc);
        if status != 0 || format_desc.is_null() {
            return;
        }

        let pts = CMTime {
            value: frame_seq as i64,
            timescale: DEFAULT_TIMESCALE,
            flags: K_CM_TIME_FLAGS_VALID,
            epoch: 0,
        };
        let duration = CMTime {
            value: 1,
            timescale: DEFAULT_TIMESCALE,
            flags: K_CM_TIME_FLAGS_VALID,
            epoch: 0,
        };
        let timing = CMSampleTimingInfo {
            duration,
            presentation_time_stamp: pts,
            decode_time_stamp: K_CM_TIME_INVALID,
        };
        let mut sample_buffer: *mut c_void = ptr::null_mut();
        let status = CMSampleBufferCreateForImageBuffer(
            ptr::null(),
            buffer_ptr,
            true,
            ptr::null(),
            ptr::null(),
            format_desc,
            &timing,
            &mut sample_buffer,
        );
        if status == 0 && !sample_buffer.is_null() {
            let _: () = msg_send![layer_ptr, enqueueSampleBuffer: sample_buffer];
        }
        if !sample_buffer.is_null() {
            CFRelease(sample_buffer);
        }
        if !format_desc.is_null() {
            CFRelease(format_desc);
        }
    });
}
