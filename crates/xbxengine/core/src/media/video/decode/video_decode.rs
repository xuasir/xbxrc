use std::collections::VecDeque;

use crate::{
    api::{
        MacOsVideoChromaLocation, MacOsVideoColorMatrix, MacOsVideoColorPrimaries,
        MacOsVideoColorRange, MacOsVideoTransferFunction,
    },
    media::video::h264::inspection::H264AccessUnitInspection,
    media::video::render::renderer::XbxRenderFrame,
    media::video::types::EncodedFrame,
    XbxEngineRenderPixelData, XbxEngineRuntimeError,
};

const MAX_DECODED_FRAME_QUEUE_LEN: usize = 2;
const HARDWARE_DECODE_FAILURE_BURST_GAP_MS: f64 = 400.0;

#[derive(Debug)]
struct QueuedDecodedFrame {
    frame: XbxRenderFrame,
}

trait XbxHardwareVideoDecoder: Send {
    fn backend_name(&self) -> &'static str;
    fn decode(
        &mut self,
        encoded_frame: EncodedFrame,
        now_ms: f64,
    ) -> Result<Option<XbxRenderFrame>, XbxEngineRuntimeError>;
    fn reset(&mut self) -> Result<(), XbxEngineRuntimeError>;
}

#[derive(Default)]
struct NoopXbxHardwareVideoDecoder;

impl XbxHardwareVideoDecoder for NoopXbxHardwareVideoDecoder {
    fn backend_name(&self) -> &'static str {
        "noop"
    }

    fn decode(
        &mut self,
        _encoded_frame: EncodedFrame,
        _now_ms: f64,
    ) -> Result<Option<XbxRenderFrame>, XbxEngineRuntimeError> {
        Ok(None)
    }

    fn reset(&mut self) -> Result<(), XbxEngineRuntimeError> {
        Ok(())
    }
}

fn create_hardware_video_decoder() -> Box<dyn XbxHardwareVideoDecoder> {
    #[cfg(target_os = "macos")]
    {
        match MacOsVideoToolboxDecoder::new() {
            Ok(decoder) => return Box::new(decoder),
            Err(error) => {
                crate::xbx_log_info!(
                    "[xbxengine][webrtc-rs] create macos videotoolbox decoder failed: {error}"
                );
            }
        }
    }
    Box::<NoopXbxHardwareVideoDecoder>::default()
}

pub(crate) struct XbxVideoDecodeState {
    decoder: Box<dyn XbxHardwareVideoDecoder>,
    latest_decoded_seq: u64,
    first_video_packet_logged: bool,
    decoded_frame_queue: VecDeque<QueuedDecodedFrame>,
    last_decode_ok_time_ms: Option<f64>,
    last_encoded_frame_time_ms: Option<f64>,
    decoder_reset_count: u64,
    latest_decoder_reset_time_ms: Option<f64>,
    decoded_frame_drop_count: u64,
    hardware_decode_failure_streak: u32,
    latest_hardware_decode_failure_time_ms: Option<f64>,
    latest_hardware_decode_failure_status: Option<i32>,
    waiting_for_recovery_keyframe: bool,
}

impl XbxVideoDecodeState {
    pub(crate) fn new(min_delay_ms: u64, max_delay_ms: u64) -> Result<Self, XbxEngineRuntimeError> {
        let _ = (min_delay_ms, max_delay_ms);
        Ok(Self {
            decoder: create_hardware_video_decoder(),
            latest_decoded_seq: 0,
            first_video_packet_logged: false,
            decoded_frame_queue: VecDeque::new(),
            last_decode_ok_time_ms: None,
            last_encoded_frame_time_ms: None,
            decoder_reset_count: 0,
            latest_decoder_reset_time_ms: None,
            decoded_frame_drop_count: 0,
            hardware_decode_failure_streak: 0,
            latest_hardware_decode_failure_time_ms: None,
            latest_hardware_decode_failure_status: None,
            waiting_for_recovery_keyframe: false,
        })
    }

    /**
     * 响应恢复控制面的 decoder reset：清空待释放队列，并重置硬解会话。
     * 这里不更改外部恢复阈值，只做局部状态收敛。
     */
    pub(crate) fn request_decoder_reset(&mut self) -> Result<(), XbxEngineRuntimeError> {
        self.decoded_frame_queue.clear();
        self.last_decode_ok_time_ms = None;
        self.decoder.reset()?;
        self.decoder_reset_count = self.decoder_reset_count.saturating_add(1);
        self.latest_decoder_reset_time_ms = Some(now_ms_f64());
        self.reset_hardware_failure_streak();
        self.waiting_for_recovery_keyframe = true;
        Ok(())
    }

    pub(crate) fn process_encoded_frame(&mut self, encoded_frame: EncodedFrame, now_ms: f64) {
        self.last_encoded_frame_time_ms = Some(now_ms);
        if self.waiting_for_recovery_keyframe && !encoded_frame.h264.bootstrap_ready {
            return;
        }
        if !self.first_video_packet_logged {
            self.first_video_packet_logged = true;
            crate::xbx_log_info!(
                "[xbxengine][webrtc-rs] first encoded video frame received ts={} bytes={}",
                encoded_frame.rtp_timestamp,
                encoded_frame.payload.len()
            );
        }
        let decoded_frame = match self.decoder.decode(encoded_frame, now_ms) {
            Ok(frame) => {
                self.waiting_for_recovery_keyframe = false;
                frame
            }
            Err(error) => {
                let status = parse_decoder_status_code(&error);
                crate::xbx_log_error!("[xbxengine][webrtc-rs] hardware decode failed: {error}");
                self.record_hardware_decode_failure(now_ms, status);
                if should_force_recovery_keyframe(status) {
                    crate::xbx_log_warn!(
                        "[xbxengine][webrtc-rs] decoder entered wait-keyframe recovery after backend failure"
                    );
                    let _ = self.request_decoder_reset();
                }
                None
            }
        };
        let Some(mut decoded_frame) = decoded_frame else {
            return;
        };
        self.reset_hardware_failure_streak();
        self.latest_decoded_seq = self.latest_decoded_seq.saturating_add(1);
        self.last_decode_ok_time_ms = Some(now_ms);
        decoded_frame.frame_seq = self.latest_decoded_seq;
        decoded_frame.rendered_at_ms = now_ms;
        self.enqueue_decoded_frame(QueuedDecodedFrame {
            frame: decoded_frame,
        });
    }

    pub(crate) fn last_decode_ok_time_ms(&self) -> Option<f64> {
        self.last_decode_ok_time_ms
    }

    pub(crate) fn decoder_backend_name(&self) -> &'static str {
        self.decoder.backend_name()
    }

    pub(crate) fn decoder_reset_count(&self) -> u64 {
        self.decoder_reset_count
    }

    pub(crate) fn latest_decoder_reset_time_ms(&self) -> Option<f64> {
        self.latest_decoder_reset_time_ms
    }

    pub(crate) fn decoded_frame_drop_count(&self) -> u64 {
        self.decoded_frame_drop_count
    }

    pub(crate) fn hardware_decode_failure_streak(&self) -> u32 {
        self.hardware_decode_failure_streak
    }

    pub(crate) fn latest_hardware_decode_failure_time_ms(&self) -> Option<f64> {
        self.latest_hardware_decode_failure_time_ms
    }

    pub(crate) fn latest_hardware_decode_failure_status(&self) -> Option<i32> {
        self.latest_hardware_decode_failure_status
    }

    pub(crate) fn pop_decoded_frame(&mut self, _now_ms: f64) -> Option<XbxRenderFrame> {
        // native 路径已经有 pacer 负责 playout 节奏，decode stage 不再额外等待。
        self.decoded_frame_queue.pop_front().map(|item| item.frame)
    }

    fn enqueue_decoded_frame(&mut self, frame: QueuedDecodedFrame) {
        while self.decoded_frame_queue.len() >= MAX_DECODED_FRAME_QUEUE_LEN {
            let dropped = self.decoded_frame_queue.pop_front();
            if let Some(d) = dropped {
                crate::xbx_log_warn!(
                    "[xbxengine][vt] enqueue_decoded_frame: queue FULL, dropping old frame seq={}",
                    d.frame.frame_seq
                );
            }
            self.decoded_frame_drop_count = self.decoded_frame_drop_count.saturating_add(1);
        }
        self.decoded_frame_queue.push_back(frame);
    }

    // 连续硬解失败用于 recovery 诊断：只在短窗口内累加，避免偶发错误误触发。
    fn record_hardware_decode_failure(&mut self, now_ms: f64, status: Option<i32>) {
        let same_burst = self
            .latest_hardware_decode_failure_time_ms
            .map(|last| (now_ms - last).max(0.0) <= HARDWARE_DECODE_FAILURE_BURST_GAP_MS)
            .unwrap_or(false);
        self.hardware_decode_failure_streak = if same_burst {
            self.hardware_decode_failure_streak.saturating_add(1)
        } else {
            1
        };
        self.latest_hardware_decode_failure_time_ms = Some(now_ms);
        self.latest_hardware_decode_failure_status = status;
    }

    fn reset_hardware_failure_streak(&mut self) {
        self.hardware_decode_failure_streak = 0;
        self.latest_hardware_decode_failure_status = None;
    }
}

#[cfg(test)]
impl XbxVideoDecodeState {
    fn new_for_test(
        min_delay_ms: u64,
        max_delay_ms: u64,
        decoder: Box<dyn XbxHardwareVideoDecoder>,
    ) -> Self {
        let _ = (min_delay_ms, max_delay_ms);
        Self {
            decoder,
            latest_decoded_seq: 0,
            first_video_packet_logged: false,
            decoded_frame_queue: VecDeque::new(),
            last_decode_ok_time_ms: None,
            last_encoded_frame_time_ms: None,
            decoder_reset_count: 0,
            latest_decoder_reset_time_ms: None,
            decoded_frame_drop_count: 0,
            hardware_decode_failure_streak: 0,
            latest_hardware_decode_failure_time_ms: None,
            latest_hardware_decode_failure_status: None,
            waiting_for_recovery_keyframe: false,
        }
    }

    fn enqueue_decoded_frame_for_test(&mut self, frame: XbxRenderFrame) {
        self.enqueue_decoded_frame(QueuedDecodedFrame { frame });
    }
}

pub(crate) fn now_ms_f64() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0)
}

fn parse_decoder_status_code(error: &XbxEngineRuntimeError) -> Option<i32> {
    let message = error.to_string();
    let status = message.split("status=").nth(1)?;
    let token = status
        .split(|ch: char| !(ch == '-' || ch.is_ascii_digit()))
        .next()?;
    token.parse::<i32>().ok()
}

fn should_force_recovery_keyframe(status: Option<i32>) -> bool {
    matches!(
        status,
        Some(K_VT_VIDEO_DECODER_BAD_DATA_ERR | K_VT_VIDEO_DECODER_REFERENCE_MISSING_ERR)
    )
}

#[cfg(target_os = "macos")]
struct MacOsVideoToolboxDecoder {
    format_description: CMVideoFormatDescriptionRef,
    decompression_session: VTDecompressionSessionRef,
    last_sps: Vec<u8>,
    last_pps: Vec<u8>,
}

#[cfg(target_os = "macos")]
unsafe impl Send for MacOsVideoToolboxDecoder {}

#[cfg(target_os = "macos")]
impl MacOsVideoToolboxDecoder {
    fn new() -> Result<Self, XbxEngineRuntimeError> {
        Ok(Self {
            format_description: std::ptr::null_mut(),
            decompression_session: std::ptr::null_mut(),
            last_sps: Vec::new(),
            last_pps: Vec::new(),
        })
    }

    fn ensure_decoder_session(
        &mut self,
        inspection: &H264AccessUnitInspection,
    ) -> Result<bool, XbxEngineRuntimeError> {
        let Some(parameter_sets) = inspection.bootstrap_parameter_sets() else {
            return Ok(!self.decompression_session.is_null());
        };

        if self.last_sps != parameter_sets.sps.raw || self.last_pps != parameter_sets.pps.raw {
            self.last_sps = parameter_sets.sps.raw.clone();
            self.last_pps = parameter_sets.pps.raw.clone();
            self.release_session();
        }

        if !self.decompression_session.is_null() {
            return Ok(true);
        }

        if !inspection.bootstrap_ready {
            return Ok(false);
        }

        let parameter_set_pointers = [self.last_sps.as_ptr(), self.last_pps.as_ptr()];
        let parameter_set_sizes = [self.last_sps.len(), self.last_pps.len()];
        let mut format_description: CMVideoFormatDescriptionRef = std::ptr::null_mut();
        let status = unsafe {
            CMVideoFormatDescriptionCreateFromH264ParameterSets(
                std::ptr::null(),
                2,
                parameter_set_pointers.as_ptr(),
                parameter_set_sizes.as_ptr(),
                4,
                &mut format_description,
            )
        };
        if status != NO_ERR || format_description.is_null() {
            return Err(XbxEngineRuntimeError::new(format!(
                "xbxEngineCreateVideoFormatDescriptionFailed:status={status}"
            )));
        }

        let mut callback_record = VTDecompressionOutputCallbackRecord {
            decompression_output_callback: Some(vt_decompression_output_callback),
            decompression_output_ref_con: std::ptr::null_mut(),
        };

        // 指定输出像素缓冲区属性 (NV12)
        unsafe {
            let pixel_format = K_CVPIXEL_FORMAT_TYPE_420YPCBCR8_BIPLANAR_VIDEO_RANGE as i32;
            let pixel_format_num = CFNumberCreate(
                std::ptr::null(),
                kCFNumberSInt32Type,
                &pixel_format as *const i32 as *const std::ffi::c_void,
            );

            let keys = [kCVPixelBufferPixelFormatTypeKey];
            let values = [pixel_format_num as *const std::ffi::c_void];
            let pixel_buffer_attributes = CFDictionaryCreate(
                std::ptr::null(),
                keys.as_ptr(),
                values.as_ptr(),
                1,
                &kCFTypeDictionaryKeyCallBacks,
                &kCFTypeDictionaryValueCallBacks,
            );

            if !pixel_format_num.is_null() {
                CFRelease(pixel_format_num as _);
            }

            if pixel_buffer_attributes.is_null() {
                crate::xbx_log_error!(
                    "[xbxengine][webrtc-rs][vt] create pixel buffer attributes failed"
                );
            }

            let mut session: VTDecompressionSessionRef = std::ptr::null_mut();
            let status = VTDecompressionSessionCreate(
                std::ptr::null(),
                format_description,
                std::ptr::null(),
                pixel_buffer_attributes as _,
                &mut callback_record,
                &mut session,
            );

            if !pixel_buffer_attributes.is_null() {
                CFRelease(pixel_buffer_attributes as _);
            }

            if status != NO_ERR || session.is_null() {
                CFRelease(format_description as CFTypeRef);
                return Err(XbxEngineRuntimeError::new(format!(
                    "xbxEngineCreateVideoDecompressionSessionFailed:status={status}"
                )));
            }

            // 设置实时属性 (RealTime) 以确保低延迟输出
            let key = kVTDecompressionPropertyKey_RealTime;
            let val = kCFBooleanTrue;
            VTSessionSetProperty(session as _, key as _, val as _);

            self.decompression_session = session;
        }

        self.format_description = format_description;
        Ok(true)
    }

    fn release_session(&mut self) {
        if !self.decompression_session.is_null() {
            unsafe {
                // SAFETY: 会话由 VideoToolbox 创建，按官方顺序 invalidate + CFRelease。
                VTDecompressionSessionInvalidate(self.decompression_session);
                CFRelease(self.decompression_session as CFTypeRef);
            }
            self.decompression_session = std::ptr::null_mut();
        }
        if !self.format_description.is_null() {
            unsafe {
                // SAFETY: format description 由 CoreMedia 创建，需对称释放。
                CFRelease(self.format_description as CFTypeRef);
            }
            self.format_description = std::ptr::null_mut();
        }
    }
}

#[cfg(target_os = "macos")]
impl Drop for MacOsVideoToolboxDecoder {
    fn drop(&mut self) {
        self.release_session();
    }
}

#[cfg(target_os = "macos")]
impl XbxHardwareVideoDecoder for MacOsVideoToolboxDecoder {
    fn backend_name(&self) -> &'static str {
        "videotoolbox"
    }

    fn decode(
        &mut self,
        encoded_frame: EncodedFrame,
        _now_ms: f64,
    ) -> Result<Option<XbxRenderFrame>, XbxEngineRuntimeError> {
        if !self.ensure_decoder_session(&encoded_frame.h264)? {
            return Ok(None);
        }

        // 由 inspection 给出统一的 NAL 结果，避免这里再自行扫 Annex-B。
        let avcc_payload = encoded_frame
            .h264
            .build_avcc_payload(&encoded_frame.payload);

        if avcc_payload.is_empty() {
            return Ok(None);
        }

        let mut block_buffer: CMBlockBufferRef = std::ptr::null_mut();
        let status = unsafe {
            // 首先创建一个拥有指定长度但尚无实际数据的 BlockBuffer。
            // 使用 NULL 作为 blockSource 让系统自行管理内存，确保异步场景下的内存安全。
            CMBlockBufferCreateWithMemoryBlock(
                std::ptr::null(),
                std::ptr::null_mut(),
                avcc_payload.len(),
                std::ptr::null(),
                std::ptr::null_mut(),
                0,
                avcc_payload.len(),
                0, // kCMBlockBufferAssureMemoryNowFlag
                &mut block_buffer,
            )
        };
        if status != NO_ERR || block_buffer.is_null() {
            return Err(XbxEngineRuntimeError::new(format!(
                "xbxEngineCreateBlockBufferFailed:status={status}"
            )));
        }

        // 将 avcc_payload 内容拷贝进 BlockBuffer。此时 BlockBuffer 已拥有独立副本。
        let status = unsafe {
            CMBlockBufferReplaceDataBytes(
                avcc_payload.as_ptr() as *const std::ffi::c_void,
                block_buffer,
                0,
                avcc_payload.len(),
            )
        };
        if status != NO_ERR {
            unsafe {
                CFRelease(block_buffer as CFTypeRef);
            }
            return Err(XbxEngineRuntimeError::new(format!(
                "xbxEngineFillBlockBufferFailed:status={status}"
            )));
        }

        let sample_size = avcc_payload.len();
        let mut sample_buffer: CMSampleBufferRef = std::ptr::null_mut();
        let status = unsafe {
            CMSampleBufferCreateReady(
                std::ptr::null(),
                block_buffer,
                self.format_description,
                1,
                0,
                std::ptr::null(),
                1,
                &sample_size,
                &mut sample_buffer,
            )
        };
        unsafe {
            CFRelease(block_buffer as CFTypeRef);
        }
        if status != NO_ERR || sample_buffer.is_null() {
            return Err(XbxEngineRuntimeError::new(format!(
                "xbxEngineCreateSampleBufferFailed:status={status}"
            )));
        }

        // 使用堆分配的同步状态，确保回调中的 source_frame_ref_con 绝对有效直到本函数返回。
        let mut output_state = Box::new(VideoToolboxOutputState::default());
        let (sync_tx, sync_rx) = std::sync::mpsc::sync_channel(1);
        output_state.sync_tx = Some(sync_tx);

        let mut decode_info_flags = 0u32;
        let status = unsafe {
            VTDecompressionSessionDecodeFrame(
                self.decompression_session,
                sample_buffer,
                0,
                Box::into_raw(output_state) as *mut std::ffi::c_void,
                &mut decode_info_flags,
            )
        };
        unsafe {
            CFRelease(sample_buffer as CFTypeRef);
        }

        if status != NO_ERR {
            return Err(XbxEngineRuntimeError::new(format!(
                "xbxEngineVideoToolboxDecodeFailed:status={status}"
            )));
        }

        // 等待同步回答。
        let result_state = match sync_rx.recv_timeout(std::time::Duration::from_secs(1)) {
            Ok(state_ptr) => unsafe { Box::from_raw(state_ptr) },
            Err(_) => {
                crate::xbx_log_error!(
                    "[xbxengine][webrtc-rs][vt] decode session timed out or callback never fired"
                );
                return Err(XbxEngineRuntimeError::new(
                    "xbxEngineVideoToolboxDecodeTimeout".to_string(),
                ));
            }
        };

        if result_state.status != NO_ERR {
            return Err(XbxEngineRuntimeError::new(format!(
                "xbxEngineVideoToolboxOutputCallbackFailed:status={}",
                result_state.status
            )));
        }

        let pixel_buffer = result_state.pixel_buffer;
        if pixel_buffer.is_null() {
            return Ok(None);
        }

        let width = unsafe { CVPixelBufferGetWidth(pixel_buffer) as u32 };
        let height = unsafe { CVPixelBufferGetHeight(pixel_buffer) as u32 };

        // 已经由回调函数 retain 过了，此处直接接管所有权。
        let frame = XbxRenderFrame {
            width,
            height,
            frame_seq: 0,
            rendered_at_ms: 0.0,
            pixel_data: XbxEngineRenderPixelData::Descriptor {
                handle: std::sync::Arc::new(crate::api::backend::MacOsCVPixelBufferDescriptor {
                    ptr: pixel_buffer as *mut _,
                    color_matrix: pixel_buffer_color_matrix(pixel_buffer),
                    color_primaries: pixel_buffer_color_primaries(pixel_buffer),
                    transfer_function: pixel_buffer_transfer_function(pixel_buffer),
                    color_range: pixel_buffer_color_range(pixel_buffer),
                    chroma_location: pixel_buffer_chroma_location(pixel_buffer),
                    drop_fn: Some(Box::new(|ptr| unsafe {
                        CFRelease(ptr as CFTypeRef);
                    })),
                }),
            },
        };

        Ok(Some(frame))
    }

    fn reset(&mut self) -> Result<(), XbxEngineRuntimeError> {
        self.release_session();
        Ok(())
    }
}

#[cfg(target_os = "macos")]
struct VideoToolboxOutputState {
    status: OSStatus,
    pixel_buffer: CVImageBufferRef,
    sync_tx: Option<std::sync::mpsc::SyncSender<*mut VideoToolboxOutputState>>,
}

impl Default for VideoToolboxOutputState {
    fn default() -> Self {
        Self {
            status: NO_ERR,
            pixel_buffer: std::ptr::null_mut(),
            sync_tx: None,
        }
    }
}

#[cfg(target_os = "macos")]
extern "C" fn vt_decompression_output_callback(
    _decompression_output_ref_con: *mut std::ffi::c_void,
    source_frame_ref_con: *mut std::ffi::c_void,
    status: OSStatus,
    _info_flags: VTDecodeInfoFlags,
    image_buffer: CVImageBufferRef,
    _presentation_time_stamp: CMTime,
    _presentation_duration: CMTime,
) {
    if source_frame_ref_con.is_null() {
        return;
    }
    let output_ptr = source_frame_ref_con as *mut VideoToolboxOutputState;
    let output = unsafe { &mut *output_ptr };
    output.status = status;
    if status == NO_ERR && !image_buffer.is_null() {
        unsafe {
            // SAFETY: 回调返回后仍需读取像素缓冲，先 retain 再在上层释放。
            CFRetain(image_buffer as CFTypeRef);
        }
        output.pixel_buffer = image_buffer;
    } else {
        output.pixel_buffer = std::ptr::null_mut();
    }

    // 通过 sync_tx 发送自己（指针），知会上层解码完成。
    if let Some(tx) = output.sync_tx.take() {
        let _ = tx.send(output_ptr);
    }
}

#[cfg(target_os = "macos")]
type OSStatus = i32;
#[cfg(target_os = "macos")]
type CFTypeRef = *const std::ffi::c_void;
#[cfg(target_os = "macos")]
type CFAllocatorRef = *const std::ffi::c_void;
#[cfg(target_os = "macos")]
type CMVideoFormatDescriptionRef = *mut std::ffi::c_void;
#[cfg(target_os = "macos")]
type CMBlockBufferRef = *mut std::ffi::c_void;
#[cfg(target_os = "macos")]
type CMSampleBufferRef = *mut std::ffi::c_void;
#[cfg(target_os = "macos")]
type VTDecompressionSessionRef = *mut std::ffi::c_void;
#[cfg(target_os = "macos")]
type CVImageBufferRef = *mut std::ffi::c_void;
#[cfg(target_os = "macos")]
type VTDecodeInfoFlags = u32;
#[cfg(target_os = "macos")]
type CFNumberRef = *const std::ffi::c_void;
#[cfg(target_os = "macos")]
type CFStringRef = *const std::ffi::c_void;
#[cfg(target_os = "macos")]
type CVAttachmentMode = u32;
#[allow(non_upper_case_globals)]
#[cfg(target_os = "macos")]
const kCFNumberSInt32Type: i32 = 3;

#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CMTime {
    value: i64,
    timescale: i32,
    flags: u32,
    epoch: i64,
}

#[cfg(target_os = "macos")]
#[repr(C)]
struct VTDecompressionOutputCallbackRecord {
    decompression_output_callback: Option<
        extern "C" fn(
            *mut std::ffi::c_void,
            *mut std::ffi::c_void,
            OSStatus,
            VTDecodeInfoFlags,
            CVImageBufferRef,
            CMTime,
            CMTime,
        ),
    >,
    decompression_output_ref_con: *mut std::ffi::c_void,
}

#[cfg(target_os = "macos")]
const NO_ERR: OSStatus = 0;
#[cfg(target_os = "macos")]
const K_CVPIXEL_FORMAT_TYPE_420YPCBCR8_BIPLANAR_VIDEO_RANGE: u32 = 0x3432_3076;
const K_VT_VIDEO_DECODER_BAD_DATA_ERR: i32 = -12909;
const K_VT_VIDEO_DECODER_REFERENCE_MISSING_ERR: i32 = -17694;

#[cfg(target_os = "macos")]
#[link(name = "VideoToolbox", kind = "framework")]
unsafe extern "C" {
    fn VTDecompressionSessionCreate(
        allocator: CFAllocatorRef,
        video_format_description: CMVideoFormatDescriptionRef,
        video_decoder_specification: *const std::ffi::c_void,
        destination_image_buffer_attributes: *const std::ffi::c_void,
        output_callback: *const VTDecompressionOutputCallbackRecord,
        decompression_session_out: *mut VTDecompressionSessionRef,
    ) -> OSStatus;
    fn VTDecompressionSessionDecodeFrame(
        session: VTDecompressionSessionRef,
        sample_buffer: CMSampleBufferRef,
        decode_flags: u32,
        source_frame_ref_con: *mut std::ffi::c_void,
        info_flags_out: *mut VTDecodeInfoFlags,
    ) -> OSStatus;
    fn VTDecompressionSessionInvalidate(session: VTDecompressionSessionRef);

    fn VTSessionSetProperty(
        session: *mut std::ffi::c_void,
        property_key: CFTypeRef,
        property_value: CFTypeRef,
    ) -> OSStatus;

    static kVTDecompressionPropertyKey_RealTime: CFTypeRef;
}

#[cfg(target_os = "macos")]
#[link(name = "CoreMedia", kind = "framework")]
unsafe extern "C" {
    fn CMVideoFormatDescriptionCreateFromH264ParameterSets(
        allocator: CFAllocatorRef,
        parameter_set_count: usize,
        parameter_set_pointers: *const *const u8,
        parameter_set_sizes: *const usize,
        nal_unit_header_length: i32,
        format_description_out: *mut CMVideoFormatDescriptionRef,
    ) -> OSStatus;
    fn CMBlockBufferCreateWithMemoryBlock(
        structure_allocator: CFAllocatorRef,
        memory_block: *mut std::ffi::c_void,
        block_length: usize,
        block_allocator: CFAllocatorRef,
        custom_block_source: *mut std::ffi::c_void,
        offset_to_data: usize,
        data_length: usize,
        flags: u32,
        new_block_buffer_out: *mut CMBlockBufferRef,
    ) -> OSStatus;
    fn CMBlockBufferReplaceDataBytes(
        source_bytes: *const std::ffi::c_void,
        destination_block_buffer: CMBlockBufferRef,
        offset_into_destination: usize,
        data_length: usize,
    ) -> OSStatus;
    fn CMSampleBufferCreateReady(
        allocator: CFAllocatorRef,
        data_buffer: CMBlockBufferRef,
        format_description: CMVideoFormatDescriptionRef,
        num_samples: i64,
        num_sample_timing_entries: i64,
        sample_timing_array: *const std::ffi::c_void,
        num_sample_size_entries: i64,
        sample_size_array: *const usize,
        sample_buffer_out: *mut CMSampleBufferRef,
    ) -> OSStatus;
}

#[cfg(target_os = "macos")]
#[link(name = "CoreVideo", kind = "framework")]
unsafe extern "C" {
    static kCVPixelBufferPixelFormatTypeKey: *const std::ffi::c_void;
    static kCVImageBufferYCbCrMatrixKey: CFStringRef;
    static kCVImageBufferYCbCrMatrix_ITU_R_709_2: CFStringRef;
    static kCVImageBufferYCbCrMatrix_ITU_R_601_4: CFStringRef;
    static kCVImageBufferYCbCrMatrix_SMPTE_240M_1995: CFStringRef;
    static kCVImageBufferYCbCrMatrix_ITU_R_2020: CFStringRef;
    static kCVImageBufferColorPrimariesKey: CFStringRef;
    static kCVImageBufferColorPrimaries_ITU_R_709_2: CFStringRef;
    static kCVImageBufferColorPrimaries_P3_D65: CFStringRef;
    static kCVImageBufferColorPrimaries_ITU_R_2020: CFStringRef;
    static kCVImageBufferTransferFunctionKey: CFStringRef;
    static kCVImageBufferTransferFunction_ITU_R_709_2: CFStringRef;
    static kCVImageBufferTransferFunction_sRGB: CFStringRef;
    static kCVImageBufferTransferFunction_Linear: CFStringRef;
    static kCVImageBufferChromaLocationTopFieldKey: CFStringRef;
    static kCVImageBufferChromaLocationBottomFieldKey: CFStringRef;
    static kCVImageBufferChromaLocation_Center: CFStringRef;
    static kCVImageBufferChromaLocation_Left: CFStringRef;
    static kCVImageBufferChromaLocation_TopLeft: CFStringRef;
    fn CVPixelBufferGetWidth(pixel_buffer: CVImageBufferRef) -> usize;
    fn CVPixelBufferGetHeight(pixel_buffer: CVImageBufferRef) -> usize;
    fn CVBufferGetAttachment(
        buffer: CVImageBufferRef,
        key: CFStringRef,
        attachment_mode: *mut CVAttachmentMode,
    ) -> CFTypeRef;
}

#[cfg(target_os = "macos")]
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRetain(cf: CFTypeRef) -> CFTypeRef;
    fn CFRelease(cf: CFTypeRef);
    fn CFEqual(cf1: CFTypeRef, cf2: CFTypeRef) -> bool;

    static kCFTypeDictionaryKeyCallBacks: std::ffi::c_void;
    static kCFTypeDictionaryValueCallBacks: std::ffi::c_void;

    fn CFDictionaryCreate(
        allocator: CFAllocatorRef,
        keys: *const *const std::ffi::c_void,
        values: *const *const std::ffi::c_void,
        num_values: isize,
        key_callbacks: *const std::ffi::c_void,
        value_callbacks: *const std::ffi::c_void,
    ) -> crate::api::backend::CFDictionaryRef;

    fn CFNumberCreate(
        allocator: CFAllocatorRef,
        the_type: i32,
        value_ptr: *const std::ffi::c_void,
    ) -> CFNumberRef;

    static kCFBooleanTrue: CFTypeRef;
}

#[cfg(target_os = "macos")]
fn pixel_buffer_color_matrix(pixel_buffer: CVImageBufferRef) -> MacOsVideoColorMatrix {
    let attachment = cv_attachment(pixel_buffer, unsafe { kCVImageBufferYCbCrMatrixKey });
    match attachment {
        Some(value) if cf_equals(value, unsafe { kCVImageBufferYCbCrMatrix_ITU_R_709_2 }) => {
            MacOsVideoColorMatrix::Bt709
        }
        Some(value) if cf_equals(value, unsafe { kCVImageBufferYCbCrMatrix_ITU_R_601_4 }) => {
            MacOsVideoColorMatrix::Bt601
        }
        Some(value) if cf_equals(value, unsafe { kCVImageBufferYCbCrMatrix_SMPTE_240M_1995 }) => {
            MacOsVideoColorMatrix::Smpte240M
        }
        Some(value) if cf_equals(value, unsafe { kCVImageBufferYCbCrMatrix_ITU_R_2020 }) => {
            MacOsVideoColorMatrix::Bt2020
        }
        Some(_) => MacOsVideoColorMatrix::Unknown,
        None => MacOsVideoColorMatrix::Bt709,
    }
}

#[cfg(target_os = "macos")]
fn pixel_buffer_color_primaries(pixel_buffer: CVImageBufferRef) -> MacOsVideoColorPrimaries {
    let attachment = cv_attachment(pixel_buffer, unsafe { kCVImageBufferColorPrimariesKey });
    match attachment {
        Some(value) if cf_equals(value, unsafe { kCVImageBufferColorPrimaries_ITU_R_709_2 }) => {
            MacOsVideoColorPrimaries::Bt709
        }
        Some(value) if cf_equals(value, unsafe { kCVImageBufferColorPrimaries_P3_D65 }) => {
            MacOsVideoColorPrimaries::P3D65
        }
        Some(value) if cf_equals(value, unsafe { kCVImageBufferColorPrimaries_ITU_R_2020 }) => {
            MacOsVideoColorPrimaries::Bt2020
        }
        Some(_) => MacOsVideoColorPrimaries::Unknown,
        None => MacOsVideoColorPrimaries::Bt709,
    }
}

#[cfg(target_os = "macos")]
fn pixel_buffer_transfer_function(pixel_buffer: CVImageBufferRef) -> MacOsVideoTransferFunction {
    let attachment = cv_attachment(pixel_buffer, unsafe { kCVImageBufferTransferFunctionKey });
    match attachment {
        Some(value) if cf_equals(value, unsafe { kCVImageBufferTransferFunction_ITU_R_709_2 }) => {
            MacOsVideoTransferFunction::Bt709
        }
        Some(value) if cf_equals(value, unsafe { kCVImageBufferTransferFunction_sRGB }) => {
            MacOsVideoTransferFunction::Srgb
        }
        Some(value) if cf_equals(value, unsafe { kCVImageBufferTransferFunction_Linear }) => {
            MacOsVideoTransferFunction::Linear
        }
        Some(_) => MacOsVideoTransferFunction::Unknown,
        None => MacOsVideoTransferFunction::Bt709,
    }
}

#[cfg(target_os = "macos")]
fn pixel_buffer_color_range(_pixel_buffer: CVImageBufferRef) -> MacOsVideoColorRange {
    // 当前 VideoToolbox 会话固定请求 video-range NV12，先显式带入 descriptor。
    MacOsVideoColorRange::Video
}

#[cfg(target_os = "macos")]
fn pixel_buffer_chroma_location(pixel_buffer: CVImageBufferRef) -> MacOsVideoChromaLocation {
    let attachment = cv_attachment(pixel_buffer, unsafe {
        kCVImageBufferChromaLocationTopFieldKey
    })
    .or_else(|| {
        cv_attachment(pixel_buffer, unsafe {
            kCVImageBufferChromaLocationBottomFieldKey
        })
    });
    match attachment {
        Some(value) if cf_equals(value, unsafe { kCVImageBufferChromaLocation_Center }) => {
            MacOsVideoChromaLocation::Center
        }
        Some(value) if cf_equals(value, unsafe { kCVImageBufferChromaLocation_Left }) => {
            MacOsVideoChromaLocation::Left
        }
        Some(value) if cf_equals(value, unsafe { kCVImageBufferChromaLocation_TopLeft }) => {
            MacOsVideoChromaLocation::TopLeft
        }
        Some(_) => MacOsVideoChromaLocation::Unknown,
        None => MacOsVideoChromaLocation::Center,
    }
}

#[cfg(target_os = "macos")]
fn cv_attachment(pixel_buffer: CVImageBufferRef, key: CFStringRef) -> Option<CFTypeRef> {
    if pixel_buffer.is_null() || key.is_null() {
        return None;
    }
    let value = unsafe { CVBufferGetAttachment(pixel_buffer, key, std::ptr::null_mut()) };
    if value.is_null() {
        None
    } else {
        Some(value)
    }
}

#[cfg(target_os = "macos")]
fn cf_equals(lhs: CFTypeRef, rhs: CFTypeRef) -> bool {
    if lhs.is_null() || rhs.is_null() {
        return false;
    }
    unsafe { CFEqual(lhs, rhs) }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use std::time::{Duration, Instant};

    use super::{XbxHardwareVideoDecoder, XbxVideoDecodeState};
    use crate::media::video::h264::inspection::{
        H264AccessUnitInspection, H264AccessUnitInspector, H264BootstrapRejectReason,
    };
    use crate::{
        media::video::render::renderer::XbxRenderFrame, media::video::types::EncodedFrame,
        XbxEngineRenderPixelData,
    };
    use bytes::Bytes;

    struct SpyHardwareDecoder {
        reset_calls: Arc<AtomicUsize>,
    }

    impl XbxHardwareVideoDecoder for SpyHardwareDecoder {
        fn backend_name(&self) -> &'static str {
            "spy"
        }

        fn decode(
            &mut self,
            _encoded_frame: EncodedFrame,
            _now_ms: f64,
        ) -> Result<Option<XbxRenderFrame>, crate::XbxEngineRuntimeError> {
            Ok(None)
        }

        fn reset(&mut self) -> Result<(), crate::XbxEngineRuntimeError> {
            self.reset_calls.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }

    #[test]
    fn request_decoder_reset_calls_hardware_decoder_reset() {
        let reset_calls = Arc::new(AtomicUsize::new(0));
        let decoder = SpyHardwareDecoder {
            reset_calls: reset_calls.clone(),
        };
        let mut state = XbxVideoDecodeState::new_for_test(20, 30, Box::new(decoder));

        state
            .request_decoder_reset()
            .expect("decoder reset should succeed");

        assert_eq!(reset_calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn decoded_queue_keeps_latest_two_frames_under_pressure() {
        let decoder = SpyHardwareDecoder {
            reset_calls: Arc::new(AtomicUsize::new(0)),
        };
        let mut state = XbxVideoDecodeState::new_for_test(20, 30, Box::new(decoder));

        for seq in 1..=3 {
            state.enqueue_decoded_frame_for_test(XbxRenderFrame {
                width: 2,
                height: 2,
                frame_seq: seq,
                rendered_at_ms: seq as f64,
                pixel_data: XbxEngineRenderPixelData::Rgba {
                    bytes: Arc::<[u8]>::from([0u8; 16]),
                },
            });
        }

        assert_eq!(state.decoded_frame_queue.len(), 2);
        assert_eq!(
            state
                .decoded_frame_queue
                .front()
                .map(|frame| frame.frame.frame_seq),
            Some(2)
        );
    }

    struct ScriptedHardwareDecoder {
        decode_calls: Arc<AtomicUsize>,
        reset_calls: Arc<AtomicUsize>,
        scripted_results: VecDeque<Result<Option<XbxRenderFrame>, crate::XbxEngineRuntimeError>>,
    }

    impl XbxHardwareVideoDecoder for ScriptedHardwareDecoder {
        fn backend_name(&self) -> &'static str {
            "scripted"
        }

        fn decode(
            &mut self,
            _encoded_frame: EncodedFrame,
            _now_ms: f64,
        ) -> Result<Option<XbxRenderFrame>, crate::XbxEngineRuntimeError> {
            self.decode_calls.fetch_add(1, Ordering::Relaxed);
            self.scripted_results.pop_front().unwrap_or(Ok(None))
        }

        fn reset(&mut self) -> Result<(), crate::XbxEngineRuntimeError> {
            self.reset_calls.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }

    fn make_encoded_frame(is_keyframe: bool) -> EncodedFrame {
        let now = Instant::now();
        EncodedFrame {
            codec: crate::media::video::types::VideoCodec::H264,
            is_keyframe,
            config_changed: false,
            value: crate::media::video::types::FrameValue::new(is_keyframe, false, 1024),
            width: 2560,
            height: 1440,
            rtp_timestamp: if is_keyframe { 1 } else { 2 },
            assembled_at: now,
            target_playout_time: now + Duration::from_millis(16),
            h264: make_h264_inspection(is_keyframe),
            payload: Bytes::from_static(b"\x00\x00\x00\x01\x65"),
        }
    }

    fn make_h264_inspection(bootstrap_ready: bool) -> H264AccessUnitInspection {
        H264AccessUnitInspection {
            nals: Vec::new(),
            parameter_sets: None,
            width: Some(2560),
            height: Some(1440),
            is_idr: bootstrap_ready,
            has_vcl: true,
            has_inband_sps: bootstrap_ready,
            has_inband_pps: bootstrap_ready,
            has_aud: false,
            slice_headers_valid: bootstrap_ready,
            parameter_sets_changed: false,
            config_changed: false,
            bootstrap_ready,
            bootstrap_reject_reason: if bootstrap_ready {
                None
            } else {
                Some(H264BootstrapRejectReason::MissingSps)
            },
            commit_state: H264AccessUnitInspector::test_commit_state(),
        }
    }

    #[test]
    fn bad_data_failure_waits_for_next_keyframe_before_decoding_again() {
        let decode_calls = Arc::new(AtomicUsize::new(0));
        let reset_calls = Arc::new(AtomicUsize::new(0));
        let decoder = ScriptedHardwareDecoder {
            decode_calls: decode_calls.clone(),
            reset_calls: reset_calls.clone(),
            scripted_results: VecDeque::from([
                Err(crate::XbxEngineRuntimeError::new(
                    "xbxEngineVideoToolboxOutputCallbackFailed:status=-12909",
                )),
                Ok(None),
            ]),
        };
        let mut state = XbxVideoDecodeState::new_for_test(20, 30, Box::new(decoder));

        state.process_encoded_frame(make_encoded_frame(true), 1_000.0);
        assert_eq!(decode_calls.load(Ordering::Relaxed), 1);
        assert_eq!(reset_calls.load(Ordering::Relaxed), 1);
        assert_eq!(state.decoder_reset_count(), 1);

        state.process_encoded_frame(make_encoded_frame(false), 1_016.0);
        assert_eq!(decode_calls.load(Ordering::Relaxed), 1);

        state.process_encoded_frame(make_encoded_frame(true), 1_032.0);
        assert_eq!(decode_calls.load(Ordering::Relaxed), 2);
    }
}
