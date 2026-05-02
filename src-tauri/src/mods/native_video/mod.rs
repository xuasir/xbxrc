use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::mods::runtime_trace::RuntimeTraceRecorderRef;
use serde_json::json;
use tauri::{AppHandle, Manager, Window};
use xbxengine::{
    MacOsCVPixelBufferDescriptor, MacOsVideoChromaLocation, MacOsVideoColorMatrix,
    MacOsVideoColorPrimaries, MacOsVideoColorRange, MacOsVideoTransferFunction,
    XbxEngineRenderFrame, XbxEngineRenderPixelData,
};

mod effects;
mod native_video_policy;
mod presenters;
mod scheduling;
mod types;
#[cfg(any(target_os = "macos", target_os = "windows"))]
mod wgpu_renderer;

use self::effects::{NoopVideoEffectPipeline, VideoEffectPipeline, WgpuVideoEffectPipeline};
use self::native_video_policy::{resolve_initial_video_pipeline_plan, resolve_video_pipeline_plan};
#[cfg(target_os = "windows")]
use self::presenters::WindowsWgpuPresenter;
use self::presenters::{
    resolve_present_kind, NativeVideoPresenter, NativeVideoPresenterKind, NoopVideoPresenter,
};
#[cfg(target_os = "macos")]
use self::presenters::{MacOsVideoPresenter, MacOsWgpuPresenter};
use self::scheduling::{HostCadenceTelemetry, ScheduledFrameSlot, ScheduledFrameTakeOutcome};
use self::types::{
    DecodedVideoSurface, VideoEffectPipelineKind, VideoPlatformCapabilities, VideoPresenterMode,
};

const MAIN_WINDOW_LABEL: &str = "main";
const STREAM_VIEWPORT_ID: &str = "stream-page-video";
const HOST_TIMING_QUEUE_WARN_MS: f64 = 24.0;
const HOST_TIMING_TICK_WARN_MS: f64 = 24.0;
const HOST_TIMING_SAMPLED_STAGE_INTERVAL_MS: f64 = 1_000.0;

static RUNTIME_TRACE: OnceLock<Mutex<Option<RuntimeTraceRecorderRef>>> = OnceLock::new();
static HOST_TIMING_STAGE_SAMPLE_TS_MS: OnceLock<Mutex<HashMap<String, f64>>> = OnceLock::new();

fn runtime_trace_slot() -> &'static Mutex<Option<RuntimeTraceRecorderRef>> {
    RUNTIME_TRACE.get_or_init(|| Mutex::new(None))
}

fn host_timing_stage_sample_slot() -> &'static Mutex<HashMap<String, f64>> {
    HOST_TIMING_STAGE_SAMPLE_TS_MS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(crate) fn set_runtime_trace_recorder(runtime_trace: RuntimeTraceRecorderRef) {
    if let Ok(mut slot) = runtime_trace_slot().lock() {
        *slot = Some(runtime_trace);
    }
}

pub(super) fn record_native_video_trace(event: &str, payload: serde_json::Value) {
    let runtime_trace = runtime_trace_slot()
        .lock()
        .ok()
        .and_then(|slot| slot.as_ref().cloned());
    if let Some(runtime_trace) = runtime_trace {
        runtime_trace.record_log("native_video", event, None, payload);
    }
}

fn record_native_video_timing_event(
    runtime_trace: Option<&RuntimeTraceRecorderRef>,
    pipeline: &str,
    stage: &str,
    viewport_id: &str,
    window_label: &str,
    payload: serde_json::Value,
) {
    record_native_video_timing_event_lazy(
        runtime_trace,
        pipeline,
        stage,
        viewport_id,
        window_label,
        move || payload,
    );
}

pub(super) fn record_native_video_timing_event_lazy<F>(
    runtime_trace: Option<&RuntimeTraceRecorderRef>,
    pipeline: &str,
    stage: &str,
    viewport_id: &str,
    window_label: &str,
    payload_builder: F,
) where
    F: FnOnce() -> serde_json::Value,
{
    let Some(runtime_trace) = runtime_trace else {
        return;
    };
    let now_ms = now_ms_f64();
    if !should_emit_host_timing_event(pipeline, stage, viewport_id, window_label, now_ms) {
        return;
    }
    runtime_trace.record_event(
        "native_video",
        "hostTiming",
        None,
        serde_json::json!({
            "pipeline": pipeline,
            "stage": stage,
            "viewportId": viewport_id,
            "windowLabel": window_label,
            "tsMs": now_ms,
            "details": payload_builder(),
        }),
    );
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HostTimingRecordPolicy {
    Always,
    Sampled,
}

fn resolve_host_timing_record_policy(stage: &str) -> HostTimingRecordPolicy {
    match stage {
        // 高频阶段在 present/pre-present 主链上会逐帧触发，按窗口采样降级。
        "hostMailboxIdle"
        | "run_on_main_thread_delay"
        | "tick_total"
        | "hostMailboxSubmitGap"
        | "hostMailboxUpdateFailed"
        | "present_tick_failed"
        | "present_tick_blocked" => HostTimingRecordPolicy::Sampled,
        _ => HostTimingRecordPolicy::Always,
    }
}

fn should_emit_sampled_host_timing(last_emit_ts_ms: Option<f64>, now_ms: f64) -> bool {
    last_emit_ts_ms.map_or(true, |last_ts_ms| {
        now_ms - last_ts_ms >= HOST_TIMING_SAMPLED_STAGE_INTERVAL_MS
    })
}

fn should_emit_host_timing_event(
    pipeline: &str,
    stage: &str,
    viewport_id: &str,
    window_label: &str,
    now_ms: f64,
) -> bool {
    if resolve_host_timing_record_policy(stage) == HostTimingRecordPolicy::Always {
        return true;
    }
    let Ok(mut sampled_stage_ts_ms) = host_timing_stage_sample_slot().lock() else {
        return false;
    };
    let key = format!("{pipeline}:{stage}:{viewport_id}:{window_label}");
    let should_emit =
        should_emit_sampled_host_timing(sampled_stage_ts_ms.get(&key).copied(), now_ms);
    if should_emit {
        sampled_stage_ts_ms.insert(key, now_ms);
    }
    should_emit
}

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
    pub latest_renderer_frame_time_ms: Option<f64>,
    pub present_count_total: u64,
    pub last_present_kind: Option<String>,
    pub latest_host_present_time_ms: Option<f64>,
    pub latest_host_submit_time_ms: Option<f64>,
    pub latest_host_submit_rtp_timestamp: Option<u32>,
    pub host_present_fps: f64,
    pub host_mailbox_submit_epoch: u64,
    pub host_mailbox_enqueue_count_total: u64,
    pub host_mailbox_drop_count_total: u64,
    pub host_mailbox_overwrite_count_total: u64,
    pub host_no_pending_take_count_total: u64,
    pub host_no_pending_streak: u32,
    pub host_no_pending_max_streak: u32,
    pub host_display_tick_epoch: u64,
    pub host_frame_present_epoch: u64,
    pub host_cadence_phase: Option<String>,
    pub last_displayed_frame_seq: Option<u64>,
    pub last_displayed_frame_rtp_timestamp: Option<u32>,
    pub last_displayed_at_ms: Option<f64>,
    pub host_view_generation: u64,
    pub latest_host_view_created_at_ms: Option<f64>,
    pub host_display_interval_ms: Option<f64>,
    pub host_frame_age_budget_ms: Option<f64>,
    pub host_descriptor_upload_mode: Option<String>,
    pub host_descriptor_metal_import_count_total: u64,
    pub host_descriptor_cpu_upload_count_total: u64,
}

pub struct NativeVideoRegistry {
    app_handle: Option<AppHandle>,
    runtime_trace: Option<RuntimeTraceRecorderRef>,
    platform_capabilities: VideoPlatformCapabilities,
    viewports: HashMap<String, NativeVideoViewportState>,
    presenters: HashMap<String, Box<dyn NativeVideoPresenter>>,
    effect_pipelines: HashMap<String, Box<dyn VideoEffectPipeline>>,
}

impl NativeVideoRegistry {
    pub fn new(app_handle: AppHandle, runtime_trace: Option<RuntimeTraceRecorderRef>) -> Self {
        Self {
            app_handle: Some(app_handle),
            runtime_trace,
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
        let attach_changed =
            should_reattach_viewport(presenter_missing, presenter_kind_changed, surface_changed);
        {
            let entry = self
                .viewports
                .entry(viewport_id.to_string())
                .or_insert_with(|| NativeVideoViewportState {
                    viewport_id: viewport_id.to_string(),
                    ..Default::default()
                });
            entry.window_label = Some(target.window_label().to_string());
            if attach_changed {
                // 只有真正重新 attach 时才清 per-epoch 呈现态，避免 no-op attach 把显示链重置掉。
                entry.surface_id = surface_id.map(str::to_string);
                entry.latest_frame_seq = None;
                entry.latest_frame_width = None;
                entry.latest_frame_height = None;
                entry.latest_renderer_frame_time_ms = None;
                entry.last_present_kind = None;
                entry.present_count_total = 0;
            }
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
        if attach_changed {
            presenter.attach(surface_id);
        }
        attach_changed
    }

    pub fn detach_viewport(&mut self, viewport_id: &str) {
        if let Some(mut presenter) = self.presenters.remove(viewport_id) {
            presenter.detach();
        }
        self.effect_pipelines.remove(viewport_id);
        self.viewports.remove(viewport_id);
    }

    /// Host present 停滞自愈：仅拆掉 presenter，保留 viewport 元数据，下一帧 `present_frame` 会重建。
    pub fn reset_presenter_for_host_stall_recovery(&mut self, viewport_id: &str) {
        if let Some(mut presenter) = self.presenters.remove(viewport_id) {
            presenter.detach();
        }
        record_native_video_trace(
            "presenterResetForHostStall",
            serde_json::json!({ "viewportId": viewport_id }),
        );
    }

    /// 显示域本地自愈：重置 presenter + effect pipeline + viewport 呈现态，下一帧 `present_frame` 会重建。
    pub fn reset_presenter_for_display_recovery(&mut self, viewport_id: &str, reason: &str) {
        if let Some(mut presenter) = self.presenters.remove(viewport_id) {
            presenter.detach();
        }
        self.effect_pipelines.remove(viewport_id);
        if let Some(viewport) = self.viewports.get_mut(viewport_id) {
            reset_viewport_present_runtime_state(viewport);
        }
        record_native_video_trace(
            "presenterResetForDisplayRecovery",
            serde_json::json!({ "viewportId": viewport_id, "reason": reason }),
        );
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
        entry.latest_renderer_frame_time_ms = Some(frame.rendered_at_ms);
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

    pub fn take_pending_host_frame_drops(
        &mut self,
        viewport_id: &str,
    ) -> Vec<xbxengine::XbxEngineHostVideoFrameDropEvent> {
        self.presenters
            .get_mut(viewport_id)
            .map(|presenter| presenter.take_pending_frame_drops())
            .unwrap_or_default()
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
                        self.runtime_trace.clone(),
                    ));
                }
                return Box::new(MacOsVideoPresenter::new(
                    viewport_id,
                    target.window_label(),
                    app_handle,
                    self.runtime_trace.clone(),
                ));
            }
        }
        #[cfg(target_os = "windows")]
        {
            if let Some(app_handle) = self.app_handle.clone() {
                if kind == NativeVideoPresenterKind::Wgpu {
                    return Box::new(WindowsWgpuPresenter::new(
                        viewport_id,
                        target.window_label(),
                        app_handle,
                        self.runtime_trace.clone(),
                    ));
                }
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
            runtime_trace: None,
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

fn should_reattach_viewport(
    presenter_missing: bool,
    presenter_kind_changed: bool,
    surface_changed: bool,
) -> bool {
    presenter_missing || presenter_kind_changed || surface_changed
}

fn reset_viewport_present_runtime_state(viewport: &mut NativeVideoViewportState) {
    viewport.latest_frame_seq = None;
    viewport.latest_frame_width = None;
    viewport.latest_frame_height = None;
    viewport.latest_renderer_frame_time_ms = None;
    viewport.present_count_total = 0;
    viewport.last_present_kind = None;
    viewport.latest_host_present_time_ms = None;
    viewport.host_present_fps = 0.0;
    viewport.latest_host_submit_time_ms = None;
    viewport.latest_host_submit_rtp_timestamp = None;
    viewport.host_mailbox_submit_epoch = 0;
    viewport.host_mailbox_enqueue_count_total = 0;
    viewport.host_mailbox_drop_count_total = 0;
    viewport.host_mailbox_overwrite_count_total = 0;
    viewport.host_no_pending_take_count_total = 0;
    viewport.host_no_pending_streak = 0;
    viewport.host_no_pending_max_streak = 0;
    viewport.host_display_tick_epoch = 0;
    viewport.host_frame_present_epoch = 0;
    viewport.host_cadence_phase = None;
    viewport.last_displayed_frame_seq = None;
    viewport.last_displayed_frame_rtp_timestamp = None;
    viewport.last_displayed_at_ms = None;
    viewport.host_view_generation = 0;
    viewport.latest_host_view_created_at_ms = None;
    viewport.host_display_interval_ms = None;
    viewport.host_frame_age_budget_ms = None;
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
        record_native_video_trace(
            "configure_window_video_host_failed",
            json!({
                "label": window_label,
                "error": error.to_string(),
            }),
        );
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
    last_presented_frame_seq: Option<u64>,
    last_presented_view_generation: u64,
    last_surface_size: Option<(u32, u32)>,
    host_view_ptr: Option<*mut objc2::runtime::AnyObject>,
    host_view_managed: bool,
    host_view_generation: u64,
    latest_host_view_created_at_ms: Option<f64>,
    descriptor_upload_mode: Option<String>,
    descriptor_metal_import_count_total: u64,
    descriptor_cpu_upload_count_total: u64,
    render_loop_started: bool,
    init_failed_logged: bool,
}

#[cfg(test)]
pub(super) type MacOsWgpuTelemetry = HostCadenceTelemetry;

#[cfg(target_os = "macos")]
fn request_host_present_tick_dispatch(
    render_loop_pending: &Arc<AtomicBool>,
    rerun_requested: &Arc<AtomicBool>,
) -> bool {
    if render_loop_pending.swap(true, Ordering::Relaxed) {
        rerun_requested.store(true, Ordering::Relaxed);
        return false;
    }
    true
}

#[cfg(target_os = "macos")]
fn finish_host_present_tick_dispatch(
    render_loop_pending: &Arc<AtomicBool>,
    rerun_requested: &Arc<AtomicBool>,
) -> bool {
    if rerun_requested.swap(false, Ordering::Relaxed) {
        return true;
    }
    render_loop_pending.store(false, Ordering::Relaxed);
    false
}

#[cfg(target_os = "macos")]
fn clear_host_present_tick_dispatch(
    render_loop_pending: &Arc<AtomicBool>,
    rerun_requested: &Arc<AtomicBool>,
) {
    rerun_requested.store(false, Ordering::Relaxed);
    render_loop_pending.store(false, Ordering::Relaxed);
}

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
#[derive(Default)]
pub(super) struct MacOsLayerState {
    display_layer_ptr: Option<*mut objc2::runtime::AnyObject>,
    first_present_logged: bool,
    cached_format_desc: Option<CachedFormatDescription>,
    last_layer_bounds: Option<[f64; 4]>,
    last_layer_scale: Option<f64>,
    display_layer_generation: u64,
    latest_display_layer_created_at_ms: Option<f64>,
}

#[cfg(target_os = "macos")]
unsafe impl Send for MacOsWgpuState {}

#[cfg(target_os = "macos")]
pub(super) fn run_wgpu_render_tick(
    window: &Window,
    app_handle: &AppHandle,
    viewport_id: &str,
    renderer_state: &Arc<Mutex<MacOsWgpuState>>,
    frame_slot: &Arc<Mutex<ScheduledFrameSlot>>,
    telemetry: &Arc<Mutex<HostCadenceTelemetry>>,
    render_loop_pending: &Arc<AtomicBool>,
    rerun_requested: &Arc<AtomicBool>,
    dispatch_requested_at_ms: Option<f64>,
    runtime_trace: Option<RuntimeTraceRecorderRef>,
) {
    let tick_started_at_ms = now_ms_f64();
    let run_tick = || {
        if let Some(dispatch_ms) = dispatch_requested_at_ms {
            let queue_delay_ms = (tick_started_at_ms - dispatch_ms).max(0.0);
            if queue_delay_ms >= HOST_TIMING_QUEUE_WARN_MS {
                record_native_video_timing_event(
                    runtime_trace.as_ref(),
                    "wgpu",
                    "run_on_main_thread_delay",
                    viewport_id,
                    window.label(),
                    serde_json::json!({
                        "queueDelayMs": queue_delay_ms,
                    }),
                );
            }
        }
        let Ok(mut state) = renderer_state.lock() else {
            return;
        };
        let host_view_generation_before_tick = state.host_view_generation;
        let Some(host_view_ptr) = ensure_wgpu_host_view(app_handle, window, &mut state) else {
            return;
        };
        let host_view_generation_changed =
            state.host_view_generation != host_view_generation_before_tick;
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
        let take_outcome = {
            let Ok(mut telemetry) = telemetry.lock() else {
                return;
            };
            telemetry.record_display_tick(now_ms);
            let Ok(mut frame_slot) = frame_slot.lock() else {
                return;
            };
            if host_view_generation_changed {
                frame_slot.begin_view_epoch();
            }
            frame_slot.take_ready_frame(now_ms, &mut telemetry)
        };
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
                        "[native_video][wgpu] render failed for viewport={} window={} error={}",
                        viewport_id,
                        window.label(),
                        error
                    );
                } else {
                    let descriptor_upload = renderer.descriptor_upload_telemetry();
                    state.latest_frame = Some(frame);
                    state.last_presented_view_generation = state.host_view_generation;
                    state.descriptor_upload_mode = descriptor_upload.last_mode;
                    state.descriptor_metal_import_count_total =
                        descriptor_upload.metal_import_count_total;
                    state.descriptor_cpu_upload_count_total =
                        descriptor_upload.cpu_upload_count_total;
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
            record_native_video_timing_event(
                runtime_trace.as_ref(),
                "wgpu",
                "tick_total",
                viewport_id,
                window.label(),
                serde_json::json!({
                    "totalMs": tick_total_ms,
                }),
            );
        }
    };

    run_tick();

    if finish_host_present_tick_dispatch(render_loop_pending, rerun_requested) {
        if let Err(error) = dispatch_wgpu_render_tick_on_main_thread(
            window,
            app_handle,
            viewport_id,
            renderer_state,
            frame_slot,
            telemetry,
            render_loop_pending,
            rerun_requested,
            runtime_trace.clone(),
        ) {
            record_native_video_timing_event(
                runtime_trace.as_ref(),
                "wgpu",
                "run_on_main_thread_enqueue_failed",
                viewport_id,
                window.label(),
                serde_json::json!({
                    "error": error.to_string(),
                    "source": "followupTick",
                }),
            );
            clear_host_present_tick_dispatch(render_loop_pending, rerun_requested);
        }
    }
}

#[cfg(target_os = "macos")]
pub(super) fn dispatch_wgpu_render_tick_on_main_thread(
    window: &Window,
    app_handle: &AppHandle,
    viewport_id: &str,
    renderer_state: &Arc<Mutex<MacOsWgpuState>>,
    frame_slot: &Arc<Mutex<ScheduledFrameSlot>>,
    telemetry: &Arc<Mutex<HostCadenceTelemetry>>,
    render_loop_pending: &Arc<AtomicBool>,
    rerun_requested: &Arc<AtomicBool>,
    runtime_trace: Option<RuntimeTraceRecorderRef>,
) -> tauri::Result<()> {
    let renderer_state = renderer_state.clone();
    let frame_slot = frame_slot.clone();
    let telemetry = telemetry.clone();
    let render_loop_pending = render_loop_pending.clone();
    let rerun_requested = rerun_requested.clone();
    let viewport_id = viewport_id.to_string();
    let app_handle = app_handle.clone();
    let window_for_task = window.clone();
    let dispatch_requested_at_ms = now_ms_f64();
    window.run_on_main_thread(move || {
        run_wgpu_render_tick(
            &window_for_task,
            &app_handle,
            &viewport_id,
            &renderer_state,
            &frame_slot,
            &telemetry,
            &render_loop_pending,
            &rerun_requested,
            Some(dispatch_requested_at_ms),
            runtime_trace,
        );
    })
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
        state.host_view_generation = state.host_view_generation.saturating_add(1);
        state.latest_host_view_created_at_ms = Some(now_ms_f64());
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
        state.host_view_generation = state.host_view_generation.saturating_add(1);
        state.latest_host_view_created_at_ms = Some(now_ms_f64());
        record_native_video_trace(
            "host_view_created",
            json!({
                "windowLabel": window_label,
                "hostViewGeneration": state.host_view_generation,
            }),
        );
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
    frame_slot: Arc<Mutex<ScheduledFrameSlot>>,
    telemetry: Arc<Mutex<HostCadenceTelemetry>>,
    render_loop_pending: Arc<AtomicBool>,
    rerun_requested: Arc<AtomicBool>,
    runtime_trace: Option<RuntimeTraceRecorderRef>,
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
        frame_slot: Arc<Mutex<ScheduledFrameSlot>>,
        telemetry: Arc<Mutex<HostCadenceTelemetry>>,
        render_loop_pending: Arc<AtomicBool>,
        rerun_requested: Arc<AtomicBool>,
        runtime_trace: Option<RuntimeTraceRecorderRef>,
    ) -> Result<Self, String> {
        let context = Box::new(MacOsDisplayLinkContext {
            viewport_id,
            window_label,
            app_handle,
            renderer_state,
            frame_slot,
            telemetry,
            render_loop_pending,
            rerun_requested,
            runtime_trace,
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
    layer_state: Arc<Mutex<MacOsLayerState>>,
    frame_slot: Arc<Mutex<ScheduledFrameSlot>>,
    telemetry: Arc<Mutex<HostCadenceTelemetry>>,
    render_loop_pending: Arc<AtomicBool>,
    runtime_trace: Option<RuntimeTraceRecorderRef>,
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
        layer_state: Arc<Mutex<MacOsLayerState>>,
        frame_slot: Arc<Mutex<ScheduledFrameSlot>>,
        telemetry: Arc<Mutex<HostCadenceTelemetry>>,
        render_loop_pending: Arc<AtomicBool>,
        runtime_trace: Option<RuntimeTraceRecorderRef>,
    ) -> Result<Self, String> {
        let context = Box::new(MacOsLayerDisplayLinkContext {
            viewport_id,
            window_label,
            layer_state,
            frame_slot,
            telemetry,
            render_loop_pending,
            runtime_trace,
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
    if !request_host_present_tick_dispatch(&context.render_loop_pending, &context.rerun_requested) {
        return 0;
    }
    let Some(window) = context.app_handle.get_window(&context.window_label) else {
        clear_host_present_tick_dispatch(&context.render_loop_pending, &context.rerun_requested);
        return 0;
    };
    if let Err(error) = dispatch_wgpu_render_tick_on_main_thread(
        &window,
        &context.app_handle,
        &context.viewport_id,
        &context.renderer_state,
        &context.frame_slot,
        &context.telemetry,
        &context.render_loop_pending,
        &context.rerun_requested,
        context.runtime_trace.clone(),
    ) {
        record_native_video_timing_event(
            context.runtime_trace.as_ref(),
            "wgpu",
            "run_on_main_thread_enqueue_failed",
            &context.viewport_id,
            &context.window_label,
            serde_json::json!({
                "error": error.to_string(),
                "source": "displayLink",
            }),
        );
        clear_host_present_tick_dispatch(&context.render_loop_pending, &context.rerun_requested);
    }
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
    run_layer_present_tick(
        &context.viewport_id,
        &context.window_label,
        &context.layer_state,
        &context.frame_slot,
        &context.telemetry,
        &context.render_loop_pending,
        context.runtime_trace.clone(),
    );
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

#[cfg(target_os = "macos")]
pub(super) fn run_layer_present_tick(
    viewport_id: &str,
    window_label: &str,
    layer_state: &Arc<Mutex<MacOsLayerState>>,
    frame_slot: &Arc<Mutex<ScheduledFrameSlot>>,
    telemetry: &Arc<Mutex<HostCadenceTelemetry>>,
    render_loop_pending: &Arc<AtomicBool>,
    runtime_trace: Option<RuntimeTraceRecorderRef>,
) {
    let tick_started_at_ms = now_ms_f64();
    let _pending_guard = PendingFlagGuard::new(render_loop_pending.clone());
    let prepare_outcome = prepare_layer_sample_for_present(
        layer_state,
        frame_slot,
        telemetry,
        viewport_id,
        window_label,
        runtime_trace.as_ref(),
    );
    let prepared_sample = match prepare_outcome {
        LayerSamplePrepareOutcome::Prepared { sample } => sample,
        LayerSamplePrepareOutcome::RetainedDisplayedFrame
        | LayerSamplePrepareOutcome::SkippedNoReadyFrame
        | LayerSamplePrepareOutcome::Failed => {
            return;
        }
    };
    let (layer_ptr, first_present) = {
        let Ok(mut layer_state_guard) = layer_state.lock() else {
            record_native_video_timing_event_lazy(
                runtime_trace.as_ref(),
                "layer",
                "present_tick_failed",
                viewport_id,
                window_label,
                || {
                    serde_json::json!({
                        "reason": "layerStateLockFailed",
                    })
                },
            );
            return;
        };
        let Some(layer_ptr) = layer_state_guard.display_layer_ptr else {
            record_native_video_timing_event_lazy(
                runtime_trace.as_ref(),
                "layer",
                "present_tick_blocked",
                viewport_id,
                window_label,
                || {
                    serde_json::json!({
                        "reason": "displayLayerUnavailable",
                    })
                },
            );
            return;
        };
        let first_present = !layer_state_guard.first_present_logged;
        if first_present {
            layer_state_guard.first_present_logged = true;
        }
        (layer_ptr, first_present)
    };
    if first_present {
        record_native_video_timing_event_lazy(
            runtime_trace.as_ref(),
            "layer",
            "first_present",
            viewport_id,
            window_label,
            || serde_json::json!({}),
        );
    }
    let sample_frame_seq = prepared_sample.frame_seq;
    let sample_width = prepared_sample.width;
    let sample_height = prepared_sample.height;
    let sample_rendered_at_ms = prepared_sample.rendered_at_ms;
    let sample_frame_recovery_disposition = prepared_sample.frame_recovery_disposition.clone();
    let sample_frame_unrecoverable_reason = prepared_sample.frame_unrecoverable_reason.clone();
    present_cv_pixelbuffer(layer_ptr, prepared_sample);
    let now_ms = now_ms_f64();
    if let Ok(mut telemetry_state) = telemetry.lock() {
        telemetry_state.record_present(now_ms);
    }
    let telemetry_diag = telemetry
        .lock()
        .ok()
        .map(|telemetry_state| telemetry_state.diagnostics_snapshot());
    let frame_slot_diag = frame_slot
        .lock()
        .ok()
        .map(|frame_slot_state| frame_slot_state.diagnostics_snapshot());
    record_native_video_timing_event_lazy(
        runtime_trace.as_ref(),
        "layer",
        "hostFramePresented",
        viewport_id,
        window_label,
        || {
            let host_display_tick_epoch =
                telemetry_diag.as_ref().map(|diag| diag.display_tick_epoch);
            let host_frame_present_epoch = telemetry_diag.as_ref().map(|diag| diag.present_epoch);
            let host_cadence_phase = telemetry_diag
                .as_ref()
                .map(|diag| diag.cadence_phase.as_str().to_string());
            let displayed_frame_seq = frame_slot_diag
                .as_ref()
                .and_then(|diag| diag.displayed_frame_seq);
            let pending_frame_seqs = frame_slot_diag
                .as_ref()
                .map(|diag| diag.pending_frame_seqs.clone())
                .unwrap_or_default();
            let last_presented_frame_seq = frame_slot_diag
                .as_ref()
                .and_then(|diag| diag.last_presented_frame_seq);
            let queue_depth = frame_slot_diag
                .as_ref()
                .map(|diag| diag.queue_depth)
                .unwrap_or(0);
            let pending_queue_depth = frame_slot_diag
                .as_ref()
                .map(|diag| diag.pending_queue_depth)
                .unwrap_or(0);
            serde_json::json!({
                "frameSeq": sample_frame_seq,
                "width": sample_width,
                "height": sample_height,
                "frameAgeMs": (now_ms - sample_rendered_at_ms).max(0.0),
                "frameRecoveryDisposition": sample_frame_recovery_disposition,
                "frameUnrecoverableReason": sample_frame_unrecoverable_reason,
                "displayedFrameSeq": displayed_frame_seq,
                "pendingFrameSeqs": pending_frame_seqs,
                "lastPresentedFrameSeq": last_presented_frame_seq,
                "queueDepth": queue_depth,
                "pendingQueueDepth": pending_queue_depth,
                "hostDisplayTickEpoch": host_display_tick_epoch,
                "hostFramePresentEpoch": host_frame_present_epoch,
                "hostCadencePhase": host_cadence_phase,
            })
        },
    );
    let tick_total_ms = (now_ms_f64() - tick_started_at_ms).max(0.0);
    if tick_total_ms >= HOST_TIMING_TICK_WARN_MS {
        record_native_video_timing_event_lazy(
            runtime_trace.as_ref(),
            "layer",
            "tick_total",
            viewport_id,
            window_label,
            || {
                serde_json::json!({
                    "totalMs": tick_total_ms,
                })
            },
        );
    }
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
            let bounds_key = ns_rect_to_key(bounds);
            let bounds_changed = state.last_layer_bounds != Some(bounds_key);
            let ns_window: *mut AnyObject = msg_send![view, window];
            let mut scale_changed = false;
            if !ns_window.is_null() {
                let scale_factor: f64 = msg_send![ns_window, backingScaleFactor];
                scale_changed = should_update_scale(state.last_layer_scale, Some(scale_factor));
                if scale_changed {
                    let _: () = msg_send![ptr, setContentsScale: scale_factor];
                    state.last_layer_scale = Some(scale_factor);
                }
            }
            if bounds_changed {
                let _: () = msg_send![ptr, setFrame: bounds];
                state.last_layer_bounds = Some(bounds_key);
            }
            if scale_changed || bounds_changed {
                let _: () = msg_send![ptr, setNeedsLayout];
                record_native_video_trace(
                    "layout_updated",
                    json!({
                        "scaleChanged": scale_changed,
                        "boundsChanged": bounds_changed,
                        "windowLabel": window.label(),
                    }),
                );
            }
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
            state.last_layer_scale = Some(scale_factor);
        }

        let bounds: NSRect = msg_send![view, bounds];
        let _: () = msg_send![layer, setFrame: bounds];
        // 视频层强制插到底部，webview 透明后即可在其上方继续承载菜单与面板。
        let _: () = msg_send![view_layer, insertSublayer: layer, atIndex: 0usize];
        let _: () = msg_send![layer, setNeedsLayout];

        state.display_layer_ptr = Some(layer);
        state.last_layer_bounds = Some(ns_rect_to_key(bounds));
        state.display_layer_generation = state.display_layer_generation.saturating_add(1);
        state.latest_display_layer_created_at_ms = Some(now_ms_f64());
        record_native_video_trace(
            "display_layer_created",
            json!({
                "windowLabel": window.label(),
                "displayLayerGeneration": state.display_layer_generation,
            }),
        );
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
    state.cached_format_desc = None;
    state.last_layer_bounds = None;
    state.last_layer_scale = None;
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
pub(super) fn prepare_layer_sample_for_present(
    layer_state: &Arc<Mutex<MacOsLayerState>>,
    frame_slot: &Arc<Mutex<ScheduledFrameSlot>>,
    telemetry: &Arc<Mutex<HostCadenceTelemetry>>,
    viewport_id: &str,
    window_label: &str,
    runtime_trace: Option<&RuntimeTraceRecorderRef>,
) -> LayerSamplePrepareOutcome {
    let now_ms = now_ms_f64();
    let mut telemetry_state = match telemetry.lock() {
        Ok(state) => state,
        Err(_) => {
            record_native_video_timing_event_lazy(
                runtime_trace,
                "layer",
                "prepare_sample_failed",
                viewport_id,
                window_label,
                || {
                    json!({
                        "reason": "telemetryLockFailed",
                    })
                },
            );
            return LayerSamplePrepareOutcome::Failed;
        }
    };
    telemetry_state.record_display_tick(now_ms);
    let telemetry_diag = telemetry_state.diagnostics_snapshot();
    let (frame_take_outcome, frame_slot_diag) = {
        let Ok(mut frame_slot_state) = frame_slot.lock() else {
            record_native_video_timing_event_lazy(
                runtime_trace,
                "layer",
                "prepare_sample_failed",
                viewport_id,
                window_label,
                || {
                    json!({
                        "reason": "frameSlotLockFailed",
                    })
                },
            );
            return LayerSamplePrepareOutcome::Failed;
        };
        let outcome = frame_slot_state.take_ready_frame(now_ms, &mut telemetry_state);
        let diagnostics = frame_slot_state.diagnostics_snapshot();
        (outcome, diagnostics)
    };
    let frame = match frame_take_outcome {
        ScheduledFrameTakeOutcome::Ready(frame) => frame,
        ScheduledFrameTakeOutcome::RetainedDisplayedFrame => {
            record_native_video_timing_event_lazy(
                runtime_trace,
                "layer",
                "hostMailboxPendingProtected",
                viewport_id,
                window_label,
                || {
                    let displayed_frame_age_ms = frame_slot_diag
                        .displayed_frame_rendered_at_ms
                        .map(|rendered_at_ms| (now_ms - rendered_at_ms).max(0.0));
                    json!({
                        "displayedFrameSeq": frame_slot_diag.displayed_frame_seq,
                        "displayedFrameRtpTimestamp": frame_slot_diag.displayed_frame_rtp_timestamp,
                        "displayedFrameRecoveryDisposition": frame_slot_diag.displayed_frame_recovery_disposition,
                        "displayedFrameAgeMs": displayed_frame_age_ms,
                        "pendingFrameSeqs": frame_slot_diag.pending_frame_seqs,
                        "lastPresentedFrameSeq": frame_slot_diag.last_presented_frame_seq,
                        "queueDepth": frame_slot_diag.queue_depth,
                        "pendingQueueDepth": frame_slot_diag.pending_queue_depth,
                        "hostDisplayTickEpoch": telemetry_diag.display_tick_epoch,
                        "hostFramePresentEpoch": telemetry_diag.present_epoch,
                        "hostCadencePhase": telemetry_diag.cadence_phase.as_str(),
                        "noPendingStreak": telemetry_diag.no_pending_streak,
                    })
                },
            );
            return LayerSamplePrepareOutcome::RetainedDisplayedFrame;
        }
        ScheduledFrameTakeOutcome::NoPendingFrame => {
            record_native_video_timing_event_lazy(
                runtime_trace,
                "layer",
                "hostMailboxIdle",
                viewport_id,
                window_label,
                || {
                    let displayed_frame_age_ms = frame_slot_diag
                        .displayed_frame_rendered_at_ms
                        .map(|rendered_at_ms| (now_ms - rendered_at_ms).max(0.0));
                    json!({
                        "reason": "noPendingFrame",
                        "displayedFrameSeq": frame_slot_diag.displayed_frame_seq,
                        "displayedFrameRtpTimestamp": frame_slot_diag.displayed_frame_rtp_timestamp,
                        "displayedFrameRecoveryDisposition": frame_slot_diag.displayed_frame_recovery_disposition,
                        "displayedFrameAgeMs": displayed_frame_age_ms,
                        "pendingFrameSeqs": frame_slot_diag.pending_frame_seqs,
                        "lastPresentedFrameSeq": frame_slot_diag.last_presented_frame_seq,
                        "queueDepth": frame_slot_diag.queue_depth,
                        "pendingQueueDepth": frame_slot_diag.pending_queue_depth,
                        "hostDisplayTickEpoch": telemetry_diag.display_tick_epoch,
                        "hostFramePresentEpoch": telemetry_diag.present_epoch,
                        "hostCadencePhase": telemetry_diag.cadence_phase.as_str(),
                        "noPendingStreak": telemetry_diag.no_pending_streak,
                        "noPendingTakeCountTotal": telemetry_diag.no_pending_take_count_total,
                    })
                },
            );
            return LayerSamplePrepareOutcome::SkippedNoReadyFrame;
        }
        ScheduledFrameTakeOutcome::DroppedStale {
            frame,
            frame_age_ms,
            frame_age_budget_ms,
        } => {
            record_native_video_timing_event_lazy(
                runtime_trace,
                "layer",
                "hostMailboxRejected",
                viewport_id,
                window_label,
                || {
                    json!({
                        "reason": "scheduledFrameStale",
                        "frameSeq": frame.frame_seq,
                        "frameAgeMs": frame_age_ms,
                        "frameAgeBudgetMs": frame_age_budget_ms,
                        "displayedFrameSeq": frame_slot_diag.displayed_frame_seq,
                        "pendingFrameSeqs": frame_slot_diag.pending_frame_seqs,
                        "lastPresentedFrameSeq": frame_slot_diag.last_presented_frame_seq,
                        "queueDepth": frame_slot_diag.queue_depth,
                        "pendingQueueDepth": frame_slot_diag.pending_queue_depth,
                        "hostDisplayTickEpoch": telemetry_diag.display_tick_epoch,
                        "hostFramePresentEpoch": telemetry_diag.present_epoch,
                        "hostCadencePhase": telemetry_diag.cadence_phase.as_str(),
                        "noPendingStreak": telemetry_diag.no_pending_streak,
                    })
                },
            );
            return LayerSamplePrepareOutcome::Failed;
        }
    };
    let frame_age_budget_ms = telemetry_state.frame_age_budget_ms();
    drop(telemetry_state);

    let XbxEngineRenderPixelData::Descriptor { handle } = &frame.pixel_data else {
        record_native_video_timing_event_lazy(
            runtime_trace,
            "layer",
            "prepare_sample_failed",
            viewport_id,
            window_label,
            || {
                json!({
                    "reason": "nonDescriptorPixelData",
                    "frameSeq": frame.frame_seq,
                })
            },
        );
        return LayerSamplePrepareOutcome::Failed;
    };
    let Some(descriptor) = handle
        .as_ref()
        .downcast_ref::<MacOsCVPixelBufferDescriptor>()
    else {
        record_native_video_timing_event_lazy(
            runtime_trace,
            "layer",
            "prepare_sample_failed",
            viewport_id,
            window_label,
            || {
                json!({
                    "reason": "nonCvPixelBufferDescriptor",
                    "frameSeq": frame.frame_seq,
                })
            },
        );
        return LayerSamplePrepareOutcome::Failed;
    };
    if descriptor.ptr.is_null() {
        record_native_video_timing_event_lazy(
            runtime_trace,
            "layer",
            "prepare_sample_failed",
            viewport_id,
            window_label,
            || {
                json!({
                    "reason": "descriptorPointerNull",
                    "frameSeq": frame.frame_seq,
                })
            },
        );
        return LayerSamplePrepareOutcome::Failed;
    }
    let Ok(mut layer_state_guard) = layer_state.lock() else {
        record_native_video_timing_event_lazy(
            runtime_trace,
            "layer",
            "prepare_sample_failed",
            viewport_id,
            window_label,
            || {
                json!({
                    "reason": "layerStateLockFailed",
                    "frameSeq": frame.frame_seq,
                })
            },
        );
        return LayerSamplePrepareOutcome::Failed;
    };
    let sample_prepare_outcome =
        prepare_cv_pixelbuffer_sample(&mut layer_state_guard, descriptor, &frame);
    let (sample, used_cached_format_description) = match sample_prepare_outcome {
        PreparedLayerSampleOutcome::Prepared {
            sample,
            used_cached_format_description,
        } => (sample, used_cached_format_description),
        PreparedLayerSampleOutcome::Failed { reason, status } => {
            record_native_video_timing_event_lazy(
                runtime_trace,
                "layer",
                "prepare_sample_failed",
                viewport_id,
                window_label,
                || {
                    json!({
                        "reason": reason,
                        "frameSeq": frame.frame_seq,
                        "status": status,
                    })
                },
            );
            return LayerSamplePrepareOutcome::Failed;
        }
    };
    let frame_age_ms = (now_ms - frame.rendered_at_ms).max(0.0);
    record_native_video_timing_event_lazy(
        runtime_trace,
        "layer",
        "prepare_sample_ready",
        viewport_id,
        window_label,
        || {
            json!({
                "frameSeq": frame.frame_seq,
                "width": frame.width,
                "height": frame.height,
                "frameAgeMs": frame_age_ms,
                "frameAgeBudgetMs": frame_age_budget_ms,
                "usedCachedFormatDescription": used_cached_format_description,
                "displayedFrameSeq": frame_slot_diag.displayed_frame_seq,
                "pendingFrameSeqs": frame_slot_diag.pending_frame_seqs,
                "lastPresentedFrameSeq": frame_slot_diag.last_presented_frame_seq,
                "queueDepth": frame_slot_diag.queue_depth,
                "pendingQueueDepth": frame_slot_diag.pending_queue_depth,
                "hostDisplayTickEpoch": telemetry_diag.display_tick_epoch,
                "hostFramePresentEpoch": telemetry_diag.present_epoch,
                "hostCadencePhase": telemetry_diag.cadence_phase.as_str(),
                "noPendingStreak": telemetry_diag.no_pending_streak,
            })
        },
    );
    LayerSamplePrepareOutcome::Prepared { sample }
}

#[cfg(target_os = "macos")]
fn prepare_cv_pixelbuffer_sample(
    layer_state: &mut MacOsLayerState,
    descriptor: &MacOsCVPixelBufferDescriptor,
    frame: &XbxEngineRenderFrame,
) -> PreparedLayerSampleOutcome {
    use objc2::rc::autoreleasepool;
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

    type OSStatus = i32;
    const DEFAULT_TIMESCALE: i32 = 60;
    const K_CM_TIME_FLAGS_VALID: u32 = 1;
    const K_CM_TIME_INVALID: CMTime = CMTime {
        value: 0,
        timescale: 0,
        flags: 0,
        epoch: 0,
    };

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

    #[link(name = "CoreVideo", kind = "framework")]
    extern "C" {
        fn CVPixelBufferGetPixelFormatType(pixel_buffer: *mut c_void) -> u32;
    }

    let key = FormatDescriptionCacheKey {
        width: frame.width,
        height: frame.height,
        pixel_format: unsafe { CVPixelBufferGetPixelFormatType(descriptor.ptr) },
        color_matrix: descriptor.color_matrix,
        color_primaries: descriptor.color_primaries,
        transfer_function: descriptor.transfer_function,
        color_range: descriptor.color_range,
        chroma_location: descriptor.chroma_location,
    };

    autoreleasepool(|_| unsafe {
        let mut used_cached_format_description = false;
        let format_desc_ptr = if layer_state
            .cached_format_desc
            .as_ref()
            .is_some_and(|cached| cached.key == key)
        {
            used_cached_format_description = true;
            layer_state
                .cached_format_desc
                .as_ref()
                .map(|cached| cached.format_description.as_ptr())
                .unwrap_or(ptr::null_mut())
        } else {
            let mut format_desc: *mut c_void = ptr::null_mut();
            let status = CMVideoFormatDescriptionCreateForImageBuffer(
                ptr::null(),
                descriptor.ptr,
                &mut format_desc,
            );
            if status != 0 || format_desc.is_null() {
                return PreparedLayerSampleOutcome::Failed {
                    reason: "formatDescriptionCreateFailed",
                    status: Some(status),
                };
            }
            let Some(cached_desc) = CoreFoundationOwnedRef::new(format_desc) else {
                return PreparedLayerSampleOutcome::Failed {
                    reason: "formatDescriptionCacheRefInvalid",
                    status: None,
                };
            };
            layer_state.cached_format_desc = Some(CachedFormatDescription {
                key,
                format_description: cached_desc,
            });
            layer_state
                .cached_format_desc
                .as_ref()
                .map(|cached| cached.format_description.as_ptr())
                .unwrap_or(ptr::null_mut())
        };
        if format_desc_ptr.is_null() {
            return PreparedLayerSampleOutcome::Failed {
                reason: "formatDescriptionUnavailable",
                status: None,
            };
        }

        let timing = CMSampleTimingInfo {
            duration: CMTime {
                value: 1,
                timescale: DEFAULT_TIMESCALE,
                flags: K_CM_TIME_FLAGS_VALID,
                epoch: 0,
            },
            presentation_time_stamp: CMTime {
                value: frame.frame_seq as i64,
                timescale: DEFAULT_TIMESCALE,
                flags: K_CM_TIME_FLAGS_VALID,
                epoch: 0,
            },
            decode_time_stamp: K_CM_TIME_INVALID,
        };
        let mut sample_buffer: *mut c_void = ptr::null_mut();
        let status = CMSampleBufferCreateForImageBuffer(
            ptr::null(),
            descriptor.ptr,
            true,
            ptr::null(),
            ptr::null(),
            format_desc_ptr,
            &timing,
            &mut sample_buffer,
        );
        if status != 0 || sample_buffer.is_null() {
            return PreparedLayerSampleOutcome::Failed {
                reason: "sampleBufferCreateFailed",
                status: Some(status),
            };
        }
        let Some(sample_ref) = CoreFoundationOwnedRef::new(sample_buffer) else {
            return PreparedLayerSampleOutcome::Failed {
                reason: "sampleBufferRefInvalid",
                status: None,
            };
        };
        PreparedLayerSampleOutcome::Prepared {
            sample: PreparedLayerSample {
                frame_seq: frame.frame_seq,
                width: frame.width,
                height: frame.height,
                rendered_at_ms: frame.rendered_at_ms,
                frame_recovery_disposition: frame.frame_recovery_disposition.clone(),
                frame_unrecoverable_reason: frame.frame_unrecoverable_reason.clone(),
                sample_buffer: sample_ref,
            },
            used_cached_format_description,
        }
    })
}

#[cfg(target_os = "macos")]
pub(super) fn present_cv_pixelbuffer(
    layer_ptr: *mut objc2::runtime::AnyObject,
    prepared_sample: PreparedLayerSample,
) {
    use objc2::{msg_send, rc::autoreleasepool};

    autoreleasepool(|_| unsafe {
        let _: () =
            msg_send![layer_ptr, enqueueSampleBuffer: prepared_sample.sample_buffer.as_ptr()];
    });
}

#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FormatDescriptionCacheKey {
    width: u32,
    height: u32,
    pixel_format: u32,
    color_matrix: MacOsVideoColorMatrix,
    color_primaries: MacOsVideoColorPrimaries,
    transfer_function: MacOsVideoTransferFunction,
    color_range: MacOsVideoColorRange,
    chroma_location: MacOsVideoChromaLocation,
}

#[cfg(target_os = "macos")]
#[derive(Debug)]
struct CachedFormatDescription {
    key: FormatDescriptionCacheKey,
    format_description: CoreFoundationOwnedRef,
}

#[cfg(target_os = "macos")]
#[derive(Debug)]
pub(super) struct PreparedLayerSample {
    frame_seq: u64,
    width: u32,
    height: u32,
    rendered_at_ms: f64,
    frame_recovery_disposition: Option<String>,
    frame_unrecoverable_reason: Option<String>,
    sample_buffer: CoreFoundationOwnedRef,
}

#[cfg(target_os = "macos")]
#[derive(Debug)]
pub(super) enum PreparedLayerSampleOutcome {
    Prepared {
        sample: PreparedLayerSample,
        used_cached_format_description: bool,
    },
    Failed {
        reason: &'static str,
        status: Option<i32>,
    },
}

#[cfg(target_os = "macos")]
#[derive(Debug)]
pub(super) enum LayerSamplePrepareOutcome {
    Prepared { sample: PreparedLayerSample },
    RetainedDisplayedFrame,
    SkippedNoReadyFrame,
    Failed,
}

#[cfg(target_os = "macos")]
#[derive(Debug)]
struct CoreFoundationOwnedRef {
    ptr: *mut std::ffi::c_void,
}

#[cfg(target_os = "macos")]
impl CoreFoundationOwnedRef {
    fn new(ptr: *mut std::ffi::c_void) -> Option<Self> {
        if ptr.is_null() {
            return None;
        }
        Some(Self { ptr })
    }

    fn as_ptr(&self) -> *mut std::ffi::c_void {
        self.ptr
    }
}

#[cfg(target_os = "macos")]
impl Drop for CoreFoundationOwnedRef {
    fn drop(&mut self) {
        unsafe {
            CFRelease(self.ptr);
        }
    }
}

#[cfg(target_os = "macos")]
unsafe impl Send for CoreFoundationOwnedRef {}

#[cfg(target_os = "macos")]
fn ns_rect_to_key(rect: objc2_foundation::NSRect) -> [f64; 4] {
    [
        rect.origin.x as f64,
        rect.origin.y as f64,
        rect.size.width as f64,
        rect.size.height as f64,
    ]
}

#[cfg(target_os = "macos")]
fn should_update_scale(previous: Option<f64>, current: Option<f64>) -> bool {
    match (previous, current) {
        (None, Some(_)) => true,
        (Some(_), None) => true,
        (Some(prev), Some(cur)) => (prev - cur).abs() > 0.001,
        (None, None) => false,
    }
}

#[cfg(all(test, target_os = "macos"))]
#[path = "mod.test.rs"]
mod tests;
