use std::collections::VecDeque;

use crate::{
    media::video::frame_buffer::FrameReleasePolicy, media::video::render::renderer::XbxRenderFrame,
    media::video::types::EncodedFrame, XbxEngineRenderPixelData, XbxEngineRuntimeError,
};

const MAX_DECODED_FRAME_QUEUE_LEN: usize = 2;

#[derive(Debug)]
struct QueuedDecodedFrame {
    queued_at_ms: f64,
    frame: XbxRenderFrame,
}

trait XbxHardwareVideoDecoder: Send {
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
    frame_release_policy: FrameReleasePolicy,
    decoder: Box<dyn XbxHardwareVideoDecoder>,
    latest_decoded_seq: u64,
    first_video_packet_logged: bool,
    decoded_frame_queue: VecDeque<QueuedDecodedFrame>,
    last_decode_ok_time_ms: Option<f64>,
    last_encoded_frame_time_ms: Option<f64>,
    decoder_reset_count: u64,
    latest_decoder_reset_time_ms: Option<f64>,
    decoded_frame_drop_count: u64,
}

impl XbxVideoDecodeState {
    pub(crate) fn new(min_delay_ms: u64, max_delay_ms: u64) -> Result<Self, XbxEngineRuntimeError> {
        Ok(Self {
            frame_release_policy: FrameReleasePolicy::new(min_delay_ms, max_delay_ms),
            decoder: create_hardware_video_decoder(),
            latest_decoded_seq: 0,
            first_video_packet_logged: false,
            decoded_frame_queue: VecDeque::new(),
            last_decode_ok_time_ms: None,
            last_encoded_frame_time_ms: None,
            decoder_reset_count: 0,
            latest_decoder_reset_time_ms: None,
            decoded_frame_drop_count: 0,
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
        Ok(())
    }

    pub(crate) fn process_encoded_frame(&mut self, encoded_frame: EncodedFrame, now_ms: f64) {
        self.last_encoded_frame_time_ms = Some(now_ms);
        if !self.first_video_packet_logged {
            self.first_video_packet_logged = true;
            crate::xbx_log_info!(
                "[xbxengine][webrtc-rs] first encoded video frame received ts={} bytes={}",
                encoded_frame.rtp_timestamp,
                encoded_frame.payload.len()
            );
        }
        let decoded_frame = match self.decoder.decode(encoded_frame, now_ms) {
            Ok(frame) => frame,
            Err(error) => {
                crate::xbx_log_error!("[xbxengine][webrtc-rs] hardware decode failed: {error}");
                None
            }
        };
        let Some(mut decoded_frame) = decoded_frame else {
            return;
        };
        self.latest_decoded_seq = self.latest_decoded_seq.saturating_add(1);
        self.last_decode_ok_time_ms = Some(now_ms);
        decoded_frame.frame_seq = self.latest_decoded_seq;
        decoded_frame.rendered_at_ms = now_ms;
        self.enqueue_decoded_frame(QueuedDecodedFrame {
            queued_at_ms: now_ms,
            frame: decoded_frame,
        });
    }

    pub(crate) fn pop_decoded_frame(&mut self, now_ms: f64) -> Option<XbxRenderFrame> {
        let queued = self.decoded_frame_queue.front()?;
        let queue_delay_ms = (now_ms - queued.queued_at_ms).max(0.0);
        let should_release = self
            .frame_release_policy
            .should_release(queue_delay_ms, self.decoded_frame_queue.len());
        
        if !should_release {
            return None;
        }
        
        crate::xbx_log_warn!("[xbxengine][vt] pop_decoded_frame: delay={:.2}ms qlen={}", queue_delay_ms, self.decoded_frame_queue.len());
        self.decoded_frame_queue.pop_front().map(|item| item.frame)
    }



    fn enqueue_decoded_frame(&mut self, frame: QueuedDecodedFrame) {
        while self.decoded_frame_queue.len() >= MAX_DECODED_FRAME_QUEUE_LEN {
            let dropped = self.decoded_frame_queue.pop_front();
            if let Some(d) = dropped {
                crate::xbx_log_warn!("[xbxengine][vt] enqueue_decoded_frame: queue FULL, dropping old frame seq={}", d.frame.frame_seq);
            }
            self.decoded_frame_drop_count = self.decoded_frame_drop_count.saturating_add(1);
        }
        crate::xbx_log_warn!("[xbxengine][vt] enqueue_decoded_frame: seq={} qlen={}", frame.frame.frame_seq, self.decoded_frame_queue.len() + 1);
        self.decoded_frame_queue.push_back(frame);
    }
}

#[cfg(test)]
impl XbxVideoDecodeState {
    fn new_for_test(
        min_delay_ms: u64,
        max_delay_ms: u64,
        decoder: Box<dyn XbxHardwareVideoDecoder>,
    ) -> Self {
        Self {
            frame_release_policy: FrameReleasePolicy::new(min_delay_ms, max_delay_ms),
            decoder,
            latest_decoded_seq: 0,
            first_video_packet_logged: false,
            decoded_frame_queue: VecDeque::new(),
            last_decode_ok_time_ms: None,
            last_encoded_frame_time_ms: None,
            decoder_reset_count: 0,
            latest_decoder_reset_time_ms: None,
            decoded_frame_drop_count: 0,
        }
    }

    fn enqueue_decoded_frame_for_test(&mut self, frame: XbxRenderFrame) {
        self.enqueue_decoded_frame(QueuedDecodedFrame {
            queued_at_ms: 0.0,
            frame,
        });
    }
}


pub(crate) fn now_ms_f64() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0)
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

    fn ensure_decoder_session(&mut self, payload: &[u8]) -> Result<bool, XbxEngineRuntimeError> {
        let nals = split_annex_b_nals(payload);
        crate::xbx_log_warn!("[xbxengine][vt] ensure_decoder_session: found {} nals", nals.len());
        for nal in &nals {
            if nal.is_empty() {
                continue;
            }
            let nal_type = nal[0] & 0x1f;
            crate::xbx_log_warn!("[xbxengine][vt] found nal type={}", nal_type);
            match nal_type {
                7 => {
                    if self.last_sps != *nal {
                        crate::xbx_log_warn!("[xbxengine][vt] SPS changed, forcing session recreate");
                        self.last_sps = nal.to_vec();
                        self.release_session();
                    }
                }
                8 => {
                    if self.last_pps != *nal {
                        crate::xbx_log_warn!("[xbxengine][vt] PPS changed, forcing session recreate");
                        self.last_pps = nal.to_vec();
                        self.release_session();
                    }
                }
                _ => {}
            }
        }

        if self.last_sps.is_empty() || self.last_pps.is_empty() {
            return Ok(!self.decompression_session.is_null());
        }

        if !self.decompression_session.is_null() && self.format_description != std::ptr::null_mut() {
            // 已有会话且参数未变（逻辑简化：这里假设 format_description 也是基于最新的 SPS/PPS）
            // 实际上我们应该检查 SPS/PPS 是否真的改变了再重建。
            // 为了稳健性，我们在 NAL 循环里已经更新了 self.last_sps/pps。
            // 这里我们只需要检查是否需要（重新）创建。
        }

        if !self.decompression_session.is_null() {
            return Ok(true);
        }

        crate::xbx_log_warn!("[xbxengine][webrtc-rs][vt] creating decoder session with stored SPS/PPS");
        self.release_session();

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
                crate::xbx_log_error!("[xbxengine][webrtc-rs][vt] create pixel buffer attributes failed");
            }

            crate::xbx_log_warn!("[xbxengine][webrtc-rs][vt] calling VTDecompressionSessionCreate");
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
        crate::xbx_log_warn!("[xbxengine][webrtc-rs][vt] decoder session created successfully");
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
    fn decode(
        &mut self,
        encoded_frame: EncodedFrame,
        _now_ms: f64,
    ) -> Result<Option<XbxRenderFrame>, XbxEngineRuntimeError> {
        if !self.ensure_decoder_session(&encoded_frame.payload)? {
            return Ok(None);
        }

        // 将 Annex-B (00 00 01 / 00 00 00 01) 转换为 AVCC (4-byte length prefix)
        // VideoToolbox 需要 AVCC 格式。
        let nals = split_annex_b_nals(&encoded_frame.payload);
        let mut avcc_payload = Vec::with_capacity(encoded_frame.payload.len() + nals.len() * 4);
        for nal in nals {
            if nal.is_empty() { continue; }
            let nal_type = nal[0] & 0x1f;
            // AVCC 模式下，SPS/PPS/AUD 不应在 SampleData 中，它们在 FormatDescription 里。
            // 某些解码器对 in-band parameter sets 敏感，返回 -12909。
            if nal_type == 7 || nal_type == 8 || nal_type == 9 {
                continue;
            }
            let len = nal.len() as u32;
            avcc_payload.extend_from_slice(&len.to_be_bytes());
            avcc_payload.extend_from_slice(nal);
        }

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
            unsafe { CFRelease(block_buffer as CFTypeRef); }
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

        crate::xbx_log_warn!("[xbxengine][vt] calling VTDecompressionSessionDecodeFrame");
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
        crate::xbx_log_warn!("[xbxengine][vt] VTDecompressionSessionDecodeFrame status={}", status);

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
                crate::xbx_log_error!("[xbxengine][webrtc-rs][vt] decode session timed out or callback never fired");
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
        crate::xbx_log_warn!("[xbxengine][vt] callback received valid image buffer");
        unsafe {
            // SAFETY: 回调返回后仍需读取像素缓冲，先 retain 再在上层释放。
            CFRetain(image_buffer as CFTypeRef);
        }
        output.pixel_buffer = image_buffer;
    } else {
        crate::xbx_log_warn!("[xbxengine][vt] callback fired with status={} buffer_is_null={}", status, image_buffer.is_null());
        output.pixel_buffer = std::ptr::null_mut();
    }

    // 通过 sync_tx 发送自己（指针），知会上层解码完成。
    if let Some(tx) = output.sync_tx.take() {
        let _ = tx.send(output_ptr);
    }
}



#[cfg(target_os = "macos")]
fn split_annex_b_nals(data: &[u8]) -> Vec<&[u8]> {
    let mut nals = Vec::new();
    let mut i = 0usize;
    while i + 3 < data.len() {
        let start_len = if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
            3
        } else if i + 4 < data.len()
            && data[i] == 0
            && data[i + 1] == 0
            && data[i + 2] == 0
            && data[i + 3] == 1
        {
            4
        } else {
            i += 1;
            continue;
        };

        let nal_start = i + start_len;
        let mut nal_end = data.len();
        let mut j = nal_start;
        while j + 3 < data.len() {
            let has_three = data[j] == 0 && data[j + 1] == 0 && data[j + 2] == 1;
            let has_four = j + 4 < data.len()
                && data[j] == 0
                && data[j + 1] == 0
                && data[j + 2] == 0
                && data[j + 3] == 1;
            if has_three || has_four {
                nal_end = j;
                break;
            }
            j += 1;
        }
        if nal_start < nal_end {
            nals.push(&data[nal_start..nal_end]);
        }
        i = nal_end;
    }
    nals
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
    fn CVPixelBufferGetWidth(pixel_buffer: CVImageBufferRef) -> usize;
    fn CVPixelBufferGetHeight(pixel_buffer: CVImageBufferRef) -> usize;
}

#[cfg(target_os = "macos")]
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRetain(cf: CFTypeRef) -> CFTypeRef;
    fn CFRelease(cf: CFTypeRef);

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

#[cfg(test)]
mod tests {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    use super::{XbxHardwareVideoDecoder, XbxVideoDecodeState};
    use crate::{
        media::video::render::renderer::XbxRenderFrame, media::video::types::EncodedFrame,
        XbxEngineRenderPixelData,
    };

    struct SpyHardwareDecoder {
        reset_calls: Arc<AtomicUsize>,
    }

    impl XbxHardwareVideoDecoder for SpyHardwareDecoder {
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
}
