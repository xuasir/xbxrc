use std::any::Any;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tauri::window::WindowBuilder;
use tauri::{AppHandle, Manager, Window};
use xbxengine::{MacOsCVPixelBufferDescriptor, XbxEngineRenderFrame, XbxEngineRenderPixelData};

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
}

pub struct NativeVideoRegistry {
    app_handle: Option<AppHandle>,
    viewports: HashMap<String, NativeVideoViewportState>,
    windows: HashMap<String, Window>,
    presenters: HashMap<String, Box<dyn NativeVideoPresenter>>,
}

impl NativeVideoRegistry {
    pub fn new(app_handle: AppHandle) -> Self {
        Self {
            app_handle: Some(app_handle),
            viewports: HashMap::new(),
            windows: HashMap::new(),
            presenters: HashMap::new(),
        }
    }

    pub fn attach_viewport(&mut self, viewport_id: &str, surface_id: Option<&str>) -> bool {
        let had_window = self.windows.contains_key(viewport_id);
        let window_label = self.ensure_viewport_window(viewport_id, true);
        let window_created = !had_window && window_label.is_some();
        let presenter_missing = !self.presenters.contains_key(viewport_id);
        let surface_changed = self
            .viewports
            .get(viewport_id)
            .and_then(|viewport| viewport.surface_id.as_deref())
            != surface_id;
        let entry = self
            .viewports
            .entry(viewport_id.to_string())
            .or_insert_with(|| NativeVideoViewportState {
                viewport_id: viewport_id.to_string(),
                ..Default::default()
            });
        entry.surface_id = surface_id.map(str::to_string);
        if let Some(label) = window_label {
            entry.window_label = Some(label);
        }
        if presenter_missing {
            let presenter = self.create_presenter(viewport_id);
            self.presenters.insert(viewport_id.to_string(), presenter);
        }
        let presenter = match self.presenters.get_mut(viewport_id) {
            Some(presenter) => presenter,
            None => return false,
        };
        if presenter_missing || surface_changed {
            presenter.attach(surface_id);
        }
        presenter_missing || surface_changed || window_created
    }

    pub fn detach_viewport(&mut self, viewport_id: &str) {
        if let Some(mut presenter) = self.presenters.remove(viewport_id) {
            presenter.detach();
        }
        if let Some(window) = self.windows.remove(viewport_id) {
            let _ = window.close();
        }
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
        let window_label = if self.windows.contains_key(viewport_id) {
            self.viewports
                .get(viewport_id)
                .and_then(|viewport| viewport.window_label.clone())
        } else {
            self.ensure_viewport_window(viewport_id, false)
        };
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
        if let Some(label) = window_label {
            entry.window_label = Some(label);
        }
        entry.latest_frame_seq = Some(frame.frame_seq);
        entry.latest_frame_width = Some(frame.width);
        entry.latest_frame_height = Some(frame.height);
        entry.latest_frame_rendered_at_ms = Some(frame.rendered_at_ms);
        entry.present_count_total = entry.present_count_total.saturating_add(1);
        entry.last_present_kind = Some(resolve_present_kind(frame));

        if !self.presenters.contains_key(viewport_id) {
            let presenter = self.create_presenter(viewport_id);
            self.presenters.insert(viewport_id.to_string(), presenter);
        }
        let presenter = match self.presenters.get_mut(viewport_id) {
            Some(presenter) => presenter,
            None => return,
        };
        presenter.present(surface_id, frame);
    }

    #[allow(dead_code)]
    pub fn snapshot(&self, viewport_id: &str) -> Option<NativeVideoViewportState> {
        self.viewports.get(viewport_id).cloned()
    }

    fn ensure_viewport_window(
        &mut self,
        viewport_id: &str,
        focus_on_create: bool,
    ) -> Option<String> {
        let app_handle = self.app_handle.as_ref()?.clone();
        let label = format!("native-video-{}", viewport_id);
        if self.windows.contains_key(viewport_id) {
            return Some(label);
        }
        let window = WindowBuilder::new(&app_handle, label.as_str())
            .title("Native Video")
            .inner_size(960.0, 540.0)
            .resizable(true)
            .center()
            .build()
            .ok()?;
        let _ = window.show();
        if focus_on_create {
            let _ = window.set_focus();
        }
        self.windows.insert(viewport_id.to_string(), window);
        Some(label)
    }

    fn create_presenter(&self, viewport_id: &str) -> Box<dyn NativeVideoPresenter> {
        #[cfg(target_os = "macos")]
        {
            if let Some(app_handle) = self.app_handle.clone() {
                return Box::new(MacOsVideoPresenter::new(viewport_id, app_handle));
            }
        }
        Box::new(NoopVideoPresenter::new(viewport_id))
    }
}

impl Default for NativeVideoRegistry {
    fn default() -> Self {
        Self {
            app_handle: None,
            viewports: HashMap::new(),
            windows: HashMap::new(),
            presenters: HashMap::new(),
        }
    }
}

pub type NativeVideoRegistryRef = Arc<Mutex<NativeVideoRegistry>>;

trait NativeVideoPresenter: Send {
    fn attach(&mut self, surface_id: Option<&str>);
    fn present(&mut self, surface_id: Option<&str>, frame: &XbxEngineRenderFrame);
    fn detach(&mut self);
}

struct NoopVideoPresenter {
    #[allow(dead_code)]
    viewport_id: String,
    #[allow(dead_code)]
    surface_id: Option<String>,
}

impl NoopVideoPresenter {
    fn new(viewport_id: &str) -> Self {
        Self {
            viewport_id: viewport_id.to_string(),
            surface_id: None,
        }
    }
}

impl NativeVideoPresenter for NoopVideoPresenter {
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
struct MacOsVideoPresenter {
    viewport_id: String,
    surface_id: Option<String>,
    last_present_was_descriptor: bool,
    last_present_was_cv_pixelbuffer: bool,
    app_handle: AppHandle,
    layer_state: Arc<Mutex<MacOsLayerState>>,
}

#[cfg(target_os = "macos")]
impl MacOsVideoPresenter {
    fn new(viewport_id: &str, app_handle: AppHandle) -> Self {
        Self {
            viewport_id: viewport_id.to_string(),
            surface_id: None,
            last_present_was_descriptor: false,
            last_present_was_cv_pixelbuffer: false,
            app_handle,
            layer_state: Arc::new(Mutex::new(MacOsLayerState::default())),
        }
    }
}

#[cfg(target_os = "macos")]
impl NativeVideoPresenter for MacOsVideoPresenter {
    fn attach(&mut self, surface_id: Option<&str>) {
        self.surface_id = surface_id.map(str::to_string);
    }

    fn present(&mut self, surface_id: Option<&str>, frame: &XbxEngineRenderFrame) {
        self.surface_id = surface_id
            .map(str::to_string)
            .or_else(|| self.surface_id.clone());
        // 最小 presenter：先验证 descriptor 是否包含 CVPixelBuffer，真实 Metal 输出后续补。
        self.last_present_was_descriptor = matches!(
            frame.pixel_data,
            XbxEngineRenderPixelData::Descriptor { .. }
        );
        self.last_present_was_cv_pixelbuffer = frame_has_cv_pixelbuffer(frame);
        if !self.last_present_was_cv_pixelbuffer {
            return;
        }
        let XbxEngineRenderPixelData::Descriptor { handle } = &frame.pixel_data else {
            return;
        };
        let handle = handle.clone();
        let frame_seq = frame.frame_seq;
        let viewport_label = format!("native-video-{}", self.viewport_id);
        let app_handle = self.app_handle.clone();
        let layer_state = self.layer_state.clone();
        if let Some(window) = app_handle.get_window(&viewport_label) {
            let window_for_task = window.clone();
            let _ = window.run_on_main_thread(move || {
                let Some(descriptor) = handle
                    .as_ref()
                    .downcast_ref::<MacOsCVPixelBufferDescriptor>()
                else {
                    return;
                };
                if descriptor.ptr.is_null() {
                    return;
                }
                let mut state = match layer_state.lock() {
                    Ok(state) => state,
                    Err(_) => return,
                };
                let layer_ptr = match ensure_display_layer(&window_for_task, &mut state) {
                    Some(layer_ptr) => layer_ptr,
                    None => return,
                };
                if !state.first_present_logged {
                    state.first_present_logged = true;
                    log::info!(
                        "[native_video][macos] first present for viewport={}",
                        viewport_label
                    );
                }
                present_cv_pixelbuffer(layer_ptr, descriptor.ptr, frame_seq);
            });
        }
    }

    fn detach(&mut self) {
        self.surface_id = None;
        self.last_present_was_descriptor = false;
        self.last_present_was_cv_pixelbuffer = false;
        let viewport_label = format!("native-video-{}", self.viewport_id);
        let app_handle = self.app_handle.clone();
        let layer_state = self.layer_state.clone();
        if let Some(window) = app_handle.get_window(&viewport_label) {
            let window_for_task = window.clone();
            let _ = window.run_on_main_thread(move || {
                let mut state = match layer_state.lock() {
                    Ok(state) => state,
                    Err(_) => return,
                };
                drop_display_layer(&window_for_task, &mut state);
            });
        }
    }
}

fn resolve_present_kind(frame: &XbxEngineRenderFrame) -> String {
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

fn frame_has_cv_pixelbuffer(frame: &XbxEngineRenderFrame) -> bool {
    let XbxEngineRenderPixelData::Descriptor { handle } = &frame.pixel_data else {
        return false;
    };
    let any_ref = handle.as_ref() as &dyn Any;
    any_ref.is::<MacOsCVPixelBufferDescriptor>()
}

#[cfg(target_os = "macos")]
#[derive(Default)]
struct MacOsLayerState {
    display_layer_ptr: Option<*mut objc2::runtime::AnyObject>,
    first_present_logged: bool,
}

#[cfg(target_os = "macos")]
unsafe impl Send for MacOsLayerState {}

#[cfg(target_os = "macos")]
fn ensure_display_layer(
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
            let _: () = msg_send![ptr, setFrame: bounds];
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

        let bounds: NSRect = msg_send![view, bounds];
        let _: () = msg_send![layer, setFrame: bounds];
        let _: () = msg_send![view_layer, addSublayer: layer];

        state.display_layer_ptr = Some(layer);
        log::info!("[native_video][macos] display layer created");
    });

    state.display_layer_ptr
}

#[cfg(target_os = "macos")]
fn drop_display_layer(window: &Window, state: &mut MacOsLayerState) {
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
fn present_cv_pixelbuffer(
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
        fn CFRelease(cf: *const c_void);
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
