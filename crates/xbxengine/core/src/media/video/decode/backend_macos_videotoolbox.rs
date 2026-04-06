#[cfg(target_os = "macos")]
use crate::api::{
    MacOsVideoChromaLocation, MacOsVideoColorMatrix, MacOsVideoColorPrimaries,
    MacOsVideoColorRange, MacOsVideoTransferFunction,
};
#[cfg(target_os = "macos")]
use crate::media::video::h264::inspection::{H264AccessUnitInspection, H264ParameterSets};
#[cfg(target_os = "macos")]
use crate::media::video::render::renderer::XbxRenderFrame;
#[cfg(target_os = "macos")]
use crate::media::video::types::EncodedFrame;
#[cfg(target_os = "macos")]
use crate::{XbxEngineRenderPixelData, XbxEngineRuntimeError};

#[cfg(target_os = "macos")]
pub(crate) fn try_create_macos_videotoolbox_backend(
) -> Result<Box<dyn super::backend::XbxVideoDecoderBackend>, XbxEngineRuntimeError> {
    Ok(Box::new(MacOsVideoToolboxDecoder::new()?))
}

#[cfg(target_os = "macos")]
struct MacOsVideoToolboxDecoder {
    format_description: CMVideoFormatDescriptionRef,
    decompression_session: VTDecompressionSessionRef,
    last_parameter_sets: Option<H264ParameterSets>,
}

#[cfg(target_os = "macos")]
unsafe impl Send for MacOsVideoToolboxDecoder {}

#[cfg(target_os = "macos")]
impl MacOsVideoToolboxDecoder {
    fn new() -> Result<Self, XbxEngineRuntimeError> {
        Ok(Self {
            format_description: std::ptr::null_mut(),
            decompression_session: std::ptr::null_mut(),
            last_parameter_sets: None,
        })
    }

    fn ensure_decoder_session(
        &mut self,
        inspection: &H264AccessUnitInspection,
    ) -> Result<bool, XbxEngineRuntimeError> {
        let Some(parameter_sets) = inspection.bootstrap_parameter_sets() else {
            return Ok(!self.decompression_session.is_null());
        };

        if self
            .last_parameter_sets
            .as_ref()
            .is_none_or(|committed| !committed.same_decoder_configuration(parameter_sets))
        {
            self.last_parameter_sets = Some(parameter_sets.clone());
            self.release_session();
        }

        if !self.decompression_session.is_null() {
            return Ok(true);
        }

        if !inspection.bootstrap_ready {
            return Ok(false);
        }

        let parameter_sets = self
            .last_parameter_sets
            .as_ref()
            .expect("parameter sets must be captured before creating a session");
        let parameter_set_pointers = [
            parameter_sets.sps.raw.as_ptr(),
            parameter_sets.pps.raw.as_ptr(),
        ];
        let parameter_set_sizes = [parameter_sets.sps.raw.len(), parameter_sets.pps.raw.len()];
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
                crate::xbx_log_error!("[xbxengine][rtc][vt] create pixel buffer attributes failed");
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

            VTSessionSetProperty(
                session as _,
                kVTDecompressionPropertyKey_RealTime as _,
                kCFBooleanTrue as _,
            );

            self.decompression_session = session;
        }

        self.format_description = format_description;
        Ok(true)
    }

    fn release_session(&mut self) {
        if !self.decompression_session.is_null() {
            unsafe {
                VTDecompressionSessionInvalidate(self.decompression_session);
                CFRelease(self.decompression_session as CFTypeRef);
            }
            self.decompression_session = std::ptr::null_mut();
        }
        if !self.format_description.is_null() {
            unsafe {
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
impl super::backend::XbxVideoDecoderBackend for MacOsVideoToolboxDecoder {
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

        let avcc_payload = encoded_frame
            .h264
            .build_avcc_payload(&encoded_frame.payload);
        if avcc_payload.is_empty() {
            return Ok(None);
        }

        let mut block_buffer: CMBlockBufferRef = std::ptr::null_mut();
        let status = unsafe {
            CMBlockBufferCreateWithMemoryBlock(
                std::ptr::null(),
                std::ptr::null_mut(),
                avcc_payload.len(),
                std::ptr::null(),
                std::ptr::null_mut(),
                0,
                avcc_payload.len(),
                0,
                &mut block_buffer,
            )
        };
        if status != NO_ERR || block_buffer.is_null() {
            return Err(XbxEngineRuntimeError::new(format!(
                "xbxEngineCreateBlockBufferFailed:status={status}"
            )));
        }

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

        let result_state = match sync_rx.recv_timeout(std::time::Duration::from_secs(1)) {
            Ok(state_ptr) => unsafe { Box::from_raw(state_ptr) },
            Err(_) => {
                crate::xbx_log_error!(
                    "[xbxengine][rtc][vt] decode session timed out or callback never fired"
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
        let frame = XbxRenderFrame {
            width,
            height,
            frame_seq: 0,
            rendered_at_ms: 0.0,
            rtp_timestamp: None,
            is_keyframe: false,
            frame_recovery_disposition: None,
            frame_unrecoverable_reason: None,
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

#[cfg(target_os = "macos")]
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
            CFRetain(image_buffer as CFTypeRef);
        }
        output.pixel_buffer = image_buffer;
    } else {
        output.pixel_buffer = std::ptr::null_mut();
    }

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
        num_samples: i32,
        num_sample_timing_entries: i32,
        sample_timing_array: *const std::ffi::c_void,
        num_sample_size_entries: i32,
        sample_size_array: *const usize,
        sample_buffer_out: *mut CMSampleBufferRef,
    ) -> OSStatus;
}

#[cfg(target_os = "macos")]
#[link(name = "CoreVideo", kind = "framework")]
unsafe extern "C" {
    fn CVPixelBufferGetWidth(pixel_buffer: CVImageBufferRef) -> usize;
    fn CVPixelBufferGetHeight(pixel_buffer: CVImageBufferRef) -> usize;
    fn CVBufferGetAttachment(
        buffer: CVImageBufferRef,
        key: CFStringRef,
        attachment_mode: *mut CVAttachmentMode,
    ) -> CFTypeRef;

    static kCVPixelBufferPixelFormatTypeKey: *const std::ffi::c_void;
    static kCVImageBufferYCbCrMatrixKey: CFStringRef;
    static kCVImageBufferYCbCrMatrix_ITU_R_709_2: CFTypeRef;
    static kCVImageBufferYCbCrMatrix_ITU_R_601_4: CFTypeRef;
    static kCVImageBufferYCbCrMatrix_SMPTE_240M_1995: CFTypeRef;
    static kCVImageBufferYCbCrMatrix_ITU_R_2020: CFTypeRef;
    static kCVImageBufferColorPrimariesKey: CFStringRef;
    static kCVImageBufferColorPrimaries_ITU_R_709_2: CFTypeRef;
    static kCVImageBufferColorPrimaries_P3_D65: CFTypeRef;
    static kCVImageBufferColorPrimaries_ITU_R_2020: CFTypeRef;
    static kCVImageBufferTransferFunctionKey: CFStringRef;
    static kCVImageBufferTransferFunction_ITU_R_709_2: CFTypeRef;
    static kCVImageBufferTransferFunction_sRGB: CFTypeRef;
    static kCVImageBufferTransferFunction_Linear: CFTypeRef;
    static kCVImageBufferChromaLocationTopFieldKey: CFStringRef;
    static kCVImageBufferChromaLocationBottomFieldKey: CFStringRef;
    static kCVImageBufferChromaLocation_Center: CFTypeRef;
    static kCVImageBufferChromaLocation_Left: CFTypeRef;
    static kCVImageBufferChromaLocation_TopLeft: CFTypeRef;
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
