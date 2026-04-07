#![cfg(target_os = "macos")]

use std::sync::Arc;

use ffmpeg_sys_next as ffi;

use crate::api::{
    MacOsCVPixelBufferDescriptor, MacOsVideoChromaLocation, MacOsVideoColorMatrix,
    MacOsVideoColorPrimaries, MacOsVideoColorRange, MacOsVideoTransferFunction,
};
use crate::media::video::render::renderer::XbxRenderFrame;
use crate::media::video::types::EncodedFrame;
use crate::{XbxEngineRenderPixelData, XbxEngineRuntimeError};

use super::backend_ffmpeg::{
    av_err_eagain, av_err_eof, error_from_message, ffmpeg_error, ffmpeg_init_once,
};

pub(crate) fn try_create_ffmpeg_macos_videotoolbox_backend(
) -> Result<Box<dyn super::backend::XbxVideoDecoderBackend>, XbxEngineRuntimeError> {
    Ok(Box::new(FfmpegMacOsVideoToolboxDecoder::new()?))
}

struct FfmpegMacOsVideoToolboxDecoder {
    codec_ctx: *mut ffi::AVCodecContext,
    packet: *mut ffi::AVPacket,
    frame: *mut ffi::AVFrame,
    hw_device_ctx: *mut ffi::AVBufferRef,
}

unsafe impl Send for FfmpegMacOsVideoToolboxDecoder {}

impl FfmpegMacOsVideoToolboxDecoder {
    fn new() -> Result<Self, XbxEngineRuntimeError> {
        ffmpeg_init_once();
        let codec = unsafe { ffi::avcodec_find_decoder(ffi::AVCodecID::AV_CODEC_ID_H264) };
        if codec.is_null() {
            return Err(error_from_message(
                "xbxEngineFfmpegFindH264DecoderFailed",
                "decoderNotFound",
            ));
        }

        let codec_ctx = unsafe { ffi::avcodec_alloc_context3(codec) };
        if codec_ctx.is_null() {
            return Err(error_from_message(
                "xbxEngineFfmpegAllocCodecContextFailed",
                "outOfMemory",
            ));
        }
        unsafe {
            (*codec_ctx).flags2 |= ffi::AV_CODEC_FLAG2_CHUNKS as i32;
            (*codec_ctx).thread_count = 1;
            (*codec_ctx).thread_type = 0;
        }

        let packet = unsafe { ffi::av_packet_alloc() };
        if packet.is_null() {
            unsafe {
                ffi::avcodec_free_context(&mut (codec_ctx as *mut _));
            }
            return Err(error_from_message(
                "xbxEngineFfmpegAllocPacketFailed",
                "outOfMemory",
            ));
        }

        let frame = unsafe { ffi::av_frame_alloc() };
        if frame.is_null() {
            unsafe {
                ffi::av_packet_free(&mut (packet as *mut _));
                ffi::avcodec_free_context(&mut (codec_ctx as *mut _));
            }
            return Err(error_from_message(
                "xbxEngineFfmpegAllocFrameFailed",
                "outOfMemory",
            ));
        }

        let mut hw_device_ctx: *mut ffi::AVBufferRef = std::ptr::null_mut();
        let hw_status = unsafe {
            ffi::av_hwdevice_ctx_create(
                &mut hw_device_ctx,
                ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VIDEOTOOLBOX,
                std::ptr::null(),
                std::ptr::null_mut(),
                0,
            )
        };
        if hw_status < 0 || hw_device_ctx.is_null() {
            unsafe {
                ffi::av_frame_free(&mut (frame as *mut _));
                ffi::av_packet_free(&mut (packet as *mut _));
                ffi::avcodec_free_context(&mut (codec_ctx as *mut _));
            }
            return Err(ffmpeg_error(
                "xbxEngineFfmpegCreateVideoToolboxDeviceFailed",
                hw_status,
            ));
        }

        unsafe {
            (*codec_ctx).get_format = Some(select_videotoolbox_pixel_format);
            (*codec_ctx).hw_device_ctx = ffi::av_buffer_ref(hw_device_ctx);
        }
        if unsafe { (*codec_ctx).hw_device_ctx.is_null() } {
            unsafe {
                ffi::av_buffer_unref(&mut hw_device_ctx);
                ffi::av_frame_free(&mut (frame as *mut _));
                ffi::av_packet_free(&mut (packet as *mut _));
                ffi::avcodec_free_context(&mut (codec_ctx as *mut _));
            }
            return Err(error_from_message(
                "xbxEngineFfmpegCreateVideoToolboxDeviceRefFailed",
                "av_buffer_ref returned null",
            ));
        }

        let open_status = unsafe { ffi::avcodec_open2(codec_ctx, codec, std::ptr::null_mut()) };
        if open_status < 0 {
            unsafe {
                ffi::av_buffer_unref(&mut hw_device_ctx);
                ffi::av_frame_free(&mut (frame as *mut _));
                ffi::av_packet_free(&mut (packet as *mut _));
                ffi::avcodec_free_context(&mut (codec_ctx as *mut _));
            }
            return Err(ffmpeg_error(
                "xbxEngineFfmpegOpenVideoToolboxDecoderFailed",
                open_status,
            ));
        }

        Ok(Self {
            codec_ctx,
            packet,
            frame,
            hw_device_ctx,
        })
    }

    fn queue_packet(&mut self, payload: &[u8]) -> Result<(), XbxEngineRuntimeError> {
        if payload.is_empty() {
            return Ok(());
        }
        if payload.len() > i32::MAX as usize {
            return Err(error_from_message(
                "xbxEngineFfmpegPacketTooLarge",
                format!("bytes={}", payload.len()),
            ));
        }

        unsafe {
            ffi::av_packet_unref(self.packet);
            let alloc_status = ffi::av_new_packet(self.packet, payload.len() as i32);
            if alloc_status < 0 {
                return Err(ffmpeg_error(
                    "xbxEngineFfmpegAllocInputPacketFailed",
                    alloc_status,
                ));
            }
            std::ptr::copy_nonoverlapping(payload.as_ptr(), (*self.packet).data, payload.len());
            (*self.packet).pts = ffi::AV_NOPTS_VALUE;
            (*self.packet).dts = ffi::AV_NOPTS_VALUE;
        }

        Ok(())
    }

    fn send_packet(&mut self) -> Result<(), XbxEngineRuntimeError> {
        let send_status = unsafe { ffi::avcodec_send_packet(self.codec_ctx, self.packet) };
        if send_status >= 0 {
            return Ok(());
        }
        if send_status == av_err_eagain() {
            let _ = self.receive_decoded_frame()?;
            let retry_status = unsafe { ffi::avcodec_send_packet(self.codec_ctx, self.packet) };
            if retry_status >= 0 {
                return Ok(());
            }
            return Err(ffmpeg_error(
                "xbxEngineFfmpegSendPacketRetryFailed",
                retry_status,
            ));
        }
        if send_status == av_err_eof() {
            return Ok(());
        }
        Err(ffmpeg_error("xbxEngineFfmpegSendPacketFailed", send_status))
    }

    fn receive_decoded_frame(&mut self) -> Result<Option<XbxRenderFrame>, XbxEngineRuntimeError> {
        let receive_status = unsafe { ffi::avcodec_receive_frame(self.codec_ctx, self.frame) };
        if receive_status == av_err_eagain() || receive_status == av_err_eof() {
            return Ok(None);
        }
        if receive_status < 0 {
            return Err(ffmpeg_error(
                "xbxEngineFfmpegReceiveFrameFailed",
                receive_status,
            ));
        }

        let frame = self.wrap_videotoolbox_frame()?;
        unsafe {
            ffi::av_frame_unref(self.frame);
        }
        Ok(Some(frame))
    }

    fn wrap_videotoolbox_frame(&mut self) -> Result<XbxRenderFrame, XbxEngineRuntimeError> {
        let width = unsafe { (*self.frame).width.max(0) as u32 };
        let height = unsafe { (*self.frame).height.max(0) as u32 };
        if width == 0 || height == 0 {
            return Err(error_from_message(
                "xbxEngineFfmpegInvalidFrameSize",
                format!("width={width}:height={height}"),
            ));
        }

        let frame_format =
            unsafe { std::mem::transmute::<i32, ffi::AVPixelFormat>((*self.frame).format) };
        if frame_format != ffi::AVPixelFormat::AV_PIX_FMT_VIDEOTOOLBOX {
            return Err(error_from_message(
                "xbxEngineFfmpegUnexpectedPixelFormat",
                format!("format={:?}", frame_format),
            ));
        }

        let mut pixel_buffer = unsafe { (*self.frame).data[3] as *mut std::ffi::c_void };
        if pixel_buffer.is_null() {
            pixel_buffer = unsafe { (*self.frame).data[0] as *mut std::ffi::c_void };
        }
        if pixel_buffer.is_null() {
            return Err(error_from_message(
                "xbxEngineFfmpegVideoToolboxFrameMissingPixelBuffer",
                "data[3]=null",
            ));
        }
        unsafe {
            CFRetain(pixel_buffer as CFTypeRef);
        }

        let descriptor = MacOsCVPixelBufferDescriptor {
            ptr: pixel_buffer,
            color_matrix: map_color_matrix(unsafe { (*self.frame).colorspace }),
            color_primaries: map_color_primaries(unsafe { (*self.frame).color_primaries }),
            transfer_function: map_transfer_function(unsafe { (*self.frame).color_trc }),
            color_range: map_color_range(unsafe { (*self.frame).color_range }),
            chroma_location: map_chroma_location(unsafe { (*self.frame).chroma_location }),
            drop_fn: Some(Box::new(|ptr| unsafe {
                CFRelease(ptr as CFTypeRef);
            })),
        };

        Ok(XbxRenderFrame {
            width,
            height,
            frame_seq: 0,
            rendered_at_ms: 0.0,
            rtp_timestamp: None,
            is_keyframe: false,
            frame_recovery_disposition: None,
            frame_unrecoverable_reason: None,
            pixel_data: XbxEngineRenderPixelData::Descriptor {
                handle: Arc::new(descriptor),
            },
        })
    }
}

impl super::backend::XbxVideoDecoderBackend for FfmpegMacOsVideoToolboxDecoder {
    fn backend_name(&self) -> &'static str {
        "ffmpeg-videotoolbox"
    }

    fn decode(
        &mut self,
        encoded_frame: EncodedFrame,
        _now_ms: f64,
    ) -> Result<Option<XbxRenderFrame>, XbxEngineRuntimeError> {
        if encoded_frame.payload.is_empty() {
            return Ok(None);
        }

        self.queue_packet(encoded_frame.payload.as_ref())?;
        self.send_packet()?;
        self.receive_decoded_frame()
    }
}

impl Drop for FfmpegMacOsVideoToolboxDecoder {
    fn drop(&mut self) {
        unsafe {
            if !self.hw_device_ctx.is_null() {
                ffi::av_buffer_unref(&mut self.hw_device_ctx);
            }
            if !self.frame.is_null() {
                ffi::av_frame_free(&mut self.frame);
            }
            if !self.packet.is_null() {
                ffi::av_packet_free(&mut self.packet);
            }
            if !self.codec_ctx.is_null() {
                ffi::avcodec_free_context(&mut self.codec_ctx);
            }
        }
    }
}

unsafe extern "C" fn select_videotoolbox_pixel_format(
    _ctx: *mut ffi::AVCodecContext,
    formats: *const ffi::AVPixelFormat,
) -> ffi::AVPixelFormat {
    if formats.is_null() {
        return ffi::AVPixelFormat::AV_PIX_FMT_NONE;
    }
    let mut current = formats;
    while unsafe { *current } != ffi::AVPixelFormat::AV_PIX_FMT_NONE {
        if unsafe { *current } == ffi::AVPixelFormat::AV_PIX_FMT_VIDEOTOOLBOX {
            return ffi::AVPixelFormat::AV_PIX_FMT_VIDEOTOOLBOX;
        }
        current = unsafe { current.add(1) };
    }
    unsafe { *formats }
}

fn map_color_matrix(value: ffi::AVColorSpace) -> MacOsVideoColorMatrix {
    match value {
        ffi::AVColorSpace::AVCOL_SPC_BT709 => MacOsVideoColorMatrix::Bt709,
        ffi::AVColorSpace::AVCOL_SPC_BT470BG | ffi::AVColorSpace::AVCOL_SPC_SMPTE170M => {
            MacOsVideoColorMatrix::Bt601
        }
        ffi::AVColorSpace::AVCOL_SPC_SMPTE240M => MacOsVideoColorMatrix::Smpte240M,
        ffi::AVColorSpace::AVCOL_SPC_BT2020_NCL | ffi::AVColorSpace::AVCOL_SPC_BT2020_CL => {
            MacOsVideoColorMatrix::Bt2020
        }
        _ => MacOsVideoColorMatrix::Unknown,
    }
}

fn map_color_primaries(value: ffi::AVColorPrimaries) -> MacOsVideoColorPrimaries {
    match value {
        ffi::AVColorPrimaries::AVCOL_PRI_BT709 => MacOsVideoColorPrimaries::Bt709,
        ffi::AVColorPrimaries::AVCOL_PRI_BT2020 => MacOsVideoColorPrimaries::Bt2020,
        ffi::AVColorPrimaries::AVCOL_PRI_SMPTE432 => MacOsVideoColorPrimaries::P3D65,
        _ => MacOsVideoColorPrimaries::Unknown,
    }
}

fn map_transfer_function(value: ffi::AVColorTransferCharacteristic) -> MacOsVideoTransferFunction {
    match value {
        ffi::AVColorTransferCharacteristic::AVCOL_TRC_BT709 => MacOsVideoTransferFunction::Bt709,
        ffi::AVColorTransferCharacteristic::AVCOL_TRC_IEC61966_2_1 => {
            MacOsVideoTransferFunction::Srgb
        }
        ffi::AVColorTransferCharacteristic::AVCOL_TRC_LINEAR => MacOsVideoTransferFunction::Linear,
        _ => MacOsVideoTransferFunction::Unknown,
    }
}

fn map_color_range(value: ffi::AVColorRange) -> MacOsVideoColorRange {
    match value {
        ffi::AVColorRange::AVCOL_RANGE_JPEG => MacOsVideoColorRange::Full,
        _ => MacOsVideoColorRange::Video,
    }
}

fn map_chroma_location(value: ffi::AVChromaLocation) -> MacOsVideoChromaLocation {
    match value {
        ffi::AVChromaLocation::AVCHROMA_LOC_LEFT => MacOsVideoChromaLocation::Left,
        ffi::AVChromaLocation::AVCHROMA_LOC_TOPLEFT => MacOsVideoChromaLocation::TopLeft,
        ffi::AVChromaLocation::AVCHROMA_LOC_CENTER => MacOsVideoChromaLocation::Center,
        _ => MacOsVideoChromaLocation::Unknown,
    }
}

type CFTypeRef = *const std::ffi::c_void;

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRetain(cf: CFTypeRef) -> CFTypeRef;
    fn CFRelease(cf: CFTypeRef);
}
