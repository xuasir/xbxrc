use std::sync::Arc;

use ffmpeg_sys_next as ffi;

use crate::media::video::render::renderer::XbxRenderFrame;
use crate::media::video::types::EncodedFrame;
use crate::{XbxEngineRenderPixelData, XbxEngineRuntimeError};

use super::backend_ffmpeg::{
    av_err_eagain, av_err_eof, error_from_message, ffmpeg_error, ffmpeg_init_once,
    receive_decoded_frames_until_eagain,
};

pub(crate) fn try_create_ffmpeg_software_backend(
) -> Result<Box<dyn super::backend::XbxVideoDecoderBackend>, XbxEngineRuntimeError> {
    Ok(Box::new(FfmpegSoftwareDecoder::new()?))
}

struct FfmpegSoftwareDecoder {
    codec_ctx: *mut ffi::AVCodecContext,
    packet: *mut ffi::AVPacket,
    frame: *mut ffi::AVFrame,
    sws_ctx: *mut ffi::SwsContext,
    bgra_buffer: Vec<u8>,
}

unsafe impl Send for FfmpegSoftwareDecoder {}

impl FfmpegSoftwareDecoder {
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

        let open_status = unsafe { ffi::avcodec_open2(codec_ctx, codec, std::ptr::null_mut()) };
        if open_status < 0 {
            unsafe {
                ffi::av_frame_free(&mut (frame as *mut _));
                ffi::av_packet_free(&mut (packet as *mut _));
                ffi::avcodec_free_context(&mut (codec_ctx as *mut _));
            }
            return Err(ffmpeg_error(
                "xbxEngineFfmpegOpenH264DecoderFailed",
                open_status,
            ));
        }

        Ok(Self {
            codec_ctx,
            packet,
            frame,
            sws_ctx: std::ptr::null_mut(),
            bgra_buffer: Vec::new(),
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

    fn send_packet(&mut self) -> Result<i32, XbxEngineRuntimeError> {
        let send_status = unsafe { ffi::avcodec_send_packet(self.codec_ctx, self.packet) };
        if send_status >= 0 {
            return Ok(send_status);
        }
        if send_status == av_err_eagain() {
            let _ = receive_decoded_frames_until_eagain(|| self.receive_decoded_frame())?;
            let retry_status = unsafe { ffi::avcodec_send_packet(self.codec_ctx, self.packet) };
            if retry_status >= 0 {
                return Ok(retry_status);
            }
            return Err(ffmpeg_error(
                "xbxEngineFfmpegSendPacketRetryFailed",
                retry_status,
            ));
        }
        if send_status == av_err_eof() {
            return Ok(send_status);
        }
        Err(ffmpeg_error("xbxEngineFfmpegSendPacketFailed", send_status))
    }

    fn receive_decoded_frame(
        &mut self,
    ) -> Result<(Option<XbxRenderFrame>, i32), XbxEngineRuntimeError> {
        let receive_status = unsafe { ffi::avcodec_receive_frame(self.codec_ctx, self.frame) };
        if receive_status == av_err_eagain() || receive_status == av_err_eof() {
            return Ok((None, receive_status));
        }
        if receive_status < 0 {
            return Err(ffmpeg_error(
                "xbxEngineFfmpegReceiveFrameFailed",
                receive_status,
            ));
        }

        let frame = self.convert_frame_to_bgra()?;
        unsafe {
            ffi::av_frame_unref(self.frame);
        }
        Ok((Some(frame), receive_status))
    }

    fn convert_frame_to_bgra(&mut self) -> Result<XbxRenderFrame, XbxEngineRuntimeError> {
        let (width, height, src_format) = unsafe {
            (
                (*self.frame).width.max(0) as u32,
                (*self.frame).height.max(0) as u32,
                (*self.frame).format,
            )
        };
        if width == 0 || height == 0 {
            return Err(error_from_message(
                "xbxEngineFfmpegInvalidFrameSize",
                format!("width={width}:height={height}"),
            ));
        }

        let expected_size = width as usize * height as usize * 4;
        if self.bgra_buffer.len() != expected_size {
            self.bgra_buffer.resize(expected_size, 0);
        }

        let mut dst_data = [std::ptr::null_mut(); 4];
        let mut dst_linesize = [0i32; 4];
        dst_data[0] = self.bgra_buffer.as_mut_ptr();
        dst_linesize[0] = (width * 4) as i32;

        self.sws_ctx = unsafe {
            ffi::sws_getCachedContext(
                self.sws_ctx,
                width as i32,
                height as i32,
                std::mem::transmute::<i32, ffi::AVPixelFormat>(src_format),
                width as i32,
                height as i32,
                ffi::AVPixelFormat::AV_PIX_FMT_BGRA,
                ffi::SwsFlags::SWS_BILINEAR as i32,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        };
        if self.sws_ctx.is_null() {
            return Err(error_from_message(
                "xbxEngineFfmpegCreateSwsContextFailed",
                format!("srcFormat={src_format}"),
            ));
        }

        let scale_status = unsafe {
            ffi::sws_scale(
                self.sws_ctx,
                (*self.frame).data.as_ptr() as *const *const u8,
                (*self.frame).linesize.as_ptr(),
                0,
                height as i32,
                dst_data.as_mut_ptr(),
                dst_linesize.as_mut_ptr(),
            )
        };
        if scale_status <= 0 {
            return Err(error_from_message(
                "xbxEngineFfmpegScaleToBgraFailed",
                format!("status={scale_status}"),
            ));
        }

        let pixel_data = XbxEngineRenderPixelData::Bgra {
            bytes: Arc::<[u8]>::from(self.bgra_buffer.clone()),
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
            pixel_data,
        })
    }
}

impl super::backend::XbxVideoDecoderBackend for FfmpegSoftwareDecoder {
    fn backend_name(&self) -> &'static str {
        "ffmpeg-software"
    }

    fn decode(
        &mut self,
        encoded_frame: EncodedFrame,
        _now_ms: f64,
    ) -> Result<super::backend::XbxVideoDecoderBackendDecodeOutcome, XbxEngineRuntimeError> {
        if encoded_frame.payload.is_empty() {
            return Ok(super::backend::XbxVideoDecoderBackendDecodeOutcome {
                frames: Vec::new(),
                send_packet_status: None,
                receive_frame_status: None,
            });
        }

        self.queue_packet(encoded_frame.payload.as_ref())?;
        let send_packet_status = self.send_packet()?;
        let (frames, receive_frame_status) =
            receive_decoded_frames_until_eagain(|| self.receive_decoded_frame())?;
        Ok(super::backend::XbxVideoDecoderBackendDecodeOutcome {
            frames,
            send_packet_status: Some(send_packet_status),
            receive_frame_status: Some(receive_frame_status),
        })
    }
}

impl Drop for FfmpegSoftwareDecoder {
    fn drop(&mut self) {
        unsafe {
            if !self.sws_ctx.is_null() {
                ffi::sws_freeContext(self.sws_ctx);
                self.sws_ctx = std::ptr::null_mut();
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
