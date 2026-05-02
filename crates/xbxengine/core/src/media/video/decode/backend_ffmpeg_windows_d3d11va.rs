#![cfg(target_os = "windows")]

use std::ffi::CString;
use std::sync::Arc;

use ffmpeg_sys_next as ffi;
use windows::core::{Interface, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Graphics::Direct3D11::{
    ID3D11Texture2D, D3D11_BIND_DECODER, D3D11_BIND_SHADER_RESOURCE, D3D11_RESOURCE_MISC_SHARED,
    D3D11_RESOURCE_MISC_SHARED_NTHANDLE,
};
use windows::Win32::Graphics::Dxgi::{
    IDXGIResource1, DXGI_SHARED_RESOURCE_READ, DXGI_SHARED_RESOURCE_WRITE,
};

use crate::media::video::render::renderer::XbxRenderFrame;
use crate::media::video::types::EncodedFrame;
use crate::{
    MacOsVideoChromaLocation, MacOsVideoColorMatrix, MacOsVideoColorPrimaries,
    MacOsVideoColorRange, MacOsVideoTransferFunction, WindowsD3d11TextureDescriptor,
    XbxEngineRenderPixelData, XbxEngineRuntimeError,
};

use super::backend_ffmpeg::{
    av_err_eagain, av_err_eof, error_from_message, ffmpeg_error, ffmpeg_init_once,
    receive_decoded_frames_until_eagain,
};

pub(crate) fn try_create_ffmpeg_windows_d3d11va_backend(
) -> Result<Box<dyn super::backend::XbxVideoDecoderBackend>, XbxEngineRuntimeError> {
    Ok(Box::new(FfmpegWindowsD3d11vaDecoder::new()?))
}

struct FfmpegWindowsD3d11vaDecoder {
    codec_ctx: *mut ffi::AVCodecContext,
    packet: *mut ffi::AVPacket,
    hw_frame: *mut ffi::AVFrame,
    sw_frame: *mut ffi::AVFrame,
    hw_device_ctx: *mut ffi::AVBufferRef,
    sws_ctx: *mut ffi::SwsContext,
    bgra_buffer: Vec<u8>,
}

unsafe impl Send for FfmpegWindowsD3d11vaDecoder {}

#[repr(C)]
struct AVD3D11VAFramesContextCompat {
    texture: *mut std::ffi::c_void,
    bind_flags: u32,
    misc_flags: u32,
    texture_infos: *mut std::ffi::c_void,
}

impl FfmpegWindowsD3d11vaDecoder {
    fn new() -> Result<Self, XbxEngineRuntimeError> {
        ffmpeg_init_once();
        let codec = find_preferred_h264_decoder();
        if codec.is_null() {
            return Err(error_from_message(
                "xbxEngineFfmpegFindH264DecoderFailed",
                "decoderNotFound",
            ));
        }
        if !codec_supports_d3d11va(codec) {
            // 某些 FFmpeg 构建不会完整暴露 hw_config 元数据，但实际仍可通过 D3D11VA 解码。
            // 这里不提前失败，交由 get_format + receive 阶段做“硬解强约束”校验。
            log::warn!(
                "[video][decode][ffmpeg-d3d11va] codec hw_config does not explicitly advertise d3d11va, continue with runtime validation codec={}",
                codec_name(codec)
            );
        }

        let codec_ctx = unsafe { ffi::avcodec_alloc_context3(codec) };
        if codec_ctx.is_null() {
            return Err(error_from_message(
                "xbxEngineFfmpegAllocCodecContextFailed",
                "outOfMemory",
            ));
        }
        unsafe {
            // 上游传入的是完整 access unit；这里不打开 CHUNKS，避免硬解把帧边界语义放松。
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

        let hw_frame = unsafe { ffi::av_frame_alloc() };
        let sw_frame = unsafe { ffi::av_frame_alloc() };
        if hw_frame.is_null() || sw_frame.is_null() {
            unsafe {
                if !sw_frame.is_null() {
                    ffi::av_frame_free(&mut (sw_frame as *mut _));
                }
                if !hw_frame.is_null() {
                    ffi::av_frame_free(&mut (hw_frame as *mut _));
                }
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
                ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_D3D11VA,
                std::ptr::null(),
                std::ptr::null_mut(),
                0,
            )
        };
        if hw_status < 0 || hw_device_ctx.is_null() {
            unsafe {
                ffi::av_frame_free(&mut (sw_frame as *mut _));
                ffi::av_frame_free(&mut (hw_frame as *mut _));
                ffi::av_packet_free(&mut (packet as *mut _));
                ffi::avcodec_free_context(&mut (codec_ctx as *mut _));
            }
            return Err(ffmpeg_error(
                "xbxEngineFfmpegCreateD3d11vaDeviceFailed",
                hw_status,
            ));
        }

        unsafe {
            (*codec_ctx).get_format = Some(select_d3d11va_pixel_format);
            (*codec_ctx).hw_device_ctx = ffi::av_buffer_ref(hw_device_ctx);
        }
        if unsafe { (*codec_ctx).hw_device_ctx.is_null() } {
            unsafe {
                ffi::av_buffer_unref(&mut hw_device_ctx);
                ffi::av_frame_free(&mut (sw_frame as *mut _));
                ffi::av_frame_free(&mut (hw_frame as *mut _));
                ffi::av_packet_free(&mut (packet as *mut _));
                ffi::avcodec_free_context(&mut (codec_ctx as *mut _));
            }
            return Err(error_from_message(
                "xbxEngineFfmpegCreateD3d11vaDeviceRefFailed",
                "av_buffer_ref returned null",
            ));
        }

        let open_status = unsafe { ffi::avcodec_open2(codec_ctx, codec, std::ptr::null_mut()) };
        if open_status < 0 {
            unsafe {
                ffi::av_buffer_unref(&mut hw_device_ctx);
                ffi::av_frame_free(&mut (sw_frame as *mut _));
                ffi::av_frame_free(&mut (hw_frame as *mut _));
                ffi::av_packet_free(&mut (packet as *mut _));
                ffi::avcodec_free_context(&mut (codec_ctx as *mut _));
            }
            return Err(ffmpeg_error(
                "xbxEngineFfmpegOpenD3d11vaDecoderFailed",
                open_status,
            ));
        }

        Ok(Self {
            codec_ctx,
            packet,
            hw_frame,
            sw_frame,
            hw_device_ctx,
            sws_ctx: std::ptr::null_mut(),
            bgra_buffer: Vec::new(),
        })
    }

    fn queue_packet(
        &mut self,
        payload: &[u8],
        is_keyframe: bool,
    ) -> Result<(), XbxEngineRuntimeError> {
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
            (*self.packet).flags = if is_keyframe {
                ffi::AV_PKT_FLAG_KEY as i32
            } else {
                0
            };
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
        let receive_status = unsafe { ffi::avcodec_receive_frame(self.codec_ctx, self.hw_frame) };
        if receive_status == av_err_eagain() || receive_status == av_err_eof() {
            return Ok((None, receive_status));
        }
        if receive_status < 0 {
            return Err(ffmpeg_error(
                "xbxEngineFfmpegReceiveFrameFailed",
                receive_status,
            ));
        }

        let hw_format =
            unsafe { std::mem::transmute::<i32, ffi::AVPixelFormat>((*self.hw_frame).format) };
        if hw_format != ffi::AVPixelFormat::AV_PIX_FMT_D3D11VA_VLD {
            unsafe {
                ffi::av_frame_unref(self.hw_frame);
            }
            return Err(error_from_message(
                "xbxEngineFfmpegD3d11vaUnexpectedFrameFormat",
                format!("decodedFormat={hw_format:?}:expected=AV_PIX_FMT_D3D11VA_VLD"),
            ));
        }

        let frame = match self.wrap_d3d11va_frame_as_descriptor(self.hw_frame) {
            Ok(frame) => frame,
            Err(descriptor_error) => {
                log::warn!(
                    "[video][decode][ffmpeg-d3d11va] descriptor export failed, fallback to copyback: {}",
                    descriptor_error
                );
                unsafe {
                    ffi::av_frame_unref(self.sw_frame);
                }
                let transfer_status =
                    unsafe { ffi::av_hwframe_transfer_data(self.sw_frame, self.hw_frame, 0) };
                if transfer_status < 0 {
                    unsafe {
                        ffi::av_frame_unref(self.hw_frame);
                    }
                    return Err(ffmpeg_error(
                        "xbxEngineFfmpegTransferD3d11vaFrameFailed",
                        transfer_status,
                    ));
                }
                self.convert_frame_to_bgra(self.sw_frame)?
            }
        };

        unsafe {
            ffi::av_frame_unref(self.sw_frame);
            ffi::av_frame_unref(self.hw_frame);
        }
        Ok((Some(frame), receive_status))
    }

    fn convert_frame_to_bgra(
        &mut self,
        frame: *mut ffi::AVFrame,
    ) -> Result<XbxRenderFrame, XbxEngineRuntimeError> {
        let (width, height, src_format) = unsafe {
            (
                (*frame).width.max(0) as u32,
                (*frame).height.max(0) as u32,
                (*frame).format,
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
                2i32,
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
                (*frame).data.as_ptr() as *const *const u8,
                (*frame).linesize.as_ptr(),
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

        Ok(XbxRenderFrame {
            width,
            height,
            frame_seq: 0,
            rendered_at_ms: 0.0,
            rtp_timestamp: None,
            recovery_epoch_tag: None,
            recovery_owner_rtp_timestamp: None,
            is_keyframe: false,
            frame_recovery_disposition: None,
            frame_unrecoverable_reason: None,
            presentation_value_role: None,
            pixel_data: XbxEngineRenderPixelData::Bgra {
                bytes: Arc::<[u8]>::from(self.bgra_buffer.clone()),
            },
        })
    }

    fn wrap_d3d11va_frame_as_descriptor(
        &mut self,
        frame: *mut ffi::AVFrame,
    ) -> Result<XbxRenderFrame, XbxEngineRuntimeError> {
        let (width, height) =
            unsafe { ((*frame).width.max(0) as u32, (*frame).height.max(0) as u32) };
        if width == 0 || height == 0 {
            return Err(error_from_message(
                "xbxEngineFfmpegInvalidFrameSize",
                format!("width={width}:height={height}"),
            ));
        }

        let texture_ptr = unsafe { (*frame).data[0] as *mut std::ffi::c_void };
        if texture_ptr.is_null() {
            return Err(error_from_message(
                "xbxEngineFfmpegD3d11vaFrameMissingTexture",
                "data[0]=null",
            ));
        }

        let raw_texture_ptr = texture_ptr.cast();
        let texture = unsafe { ID3D11Texture2D::from_raw_borrowed(&raw_texture_ptr) }
            .ok_or_else(|| {
                error_from_message(
                    "xbxEngineFfmpegD3d11vaBorrowTextureFailed",
                    "from_raw_borrowed returned null",
                )
            })?
            .clone();

        let array_slice = unsafe { (*frame).data[1] as usize as u32 };
        let mut desc = unsafe { std::mem::zeroed() };
        unsafe {
            texture.GetDesc(&mut desc);
        }

        let resource = texture.cast::<IDXGIResource1>().map_err(|error| {
            error_from_message(
                "xbxEngineFfmpegD3d11vaCastDxgiResourceFailed",
                error.to_string(),
            )
        })?;
        let shared_access = DXGI_SHARED_RESOURCE_READ.0 | DXGI_SHARED_RESOURCE_WRITE.0;
        let shared_handle = unsafe {
            resource
                .CreateSharedHandle(None, shared_access, PCWSTR::null())
                .map_err(|error| {
                    error_from_message(
                        "xbxEngineFfmpegD3d11vaCreateSharedHandleFailed",
                        error.to_string(),
                    )
                })?
        };
        if shared_handle.is_invalid() {
            return Err(error_from_message(
                "xbxEngineFfmpegD3d11vaSharedHandleInvalid",
                "CreateSharedHandle returned invalid handle",
            ));
        }

        let descriptor = WindowsD3d11TextureDescriptor {
            texture_ptr,
            shared_handle: shared_handle.0,
            dxgi_format: desc.Format.0 as u32,
            array_slice,
            color_matrix: map_color_matrix(unsafe { (*frame).colorspace }),
            color_primaries: map_color_primaries(unsafe { (*frame).color_primaries }),
            transfer_function: map_transfer_function(unsafe { (*frame).color_trc }),
            color_range: map_color_range(unsafe { (*frame).color_range }),
            chroma_location: map_chroma_location(unsafe { (*frame).chroma_location }),
            drop_fn: Some(Box::new(move |_texture_ptr, shared_handle_ptr| unsafe {
                if !shared_handle_ptr.is_null() {
                    let _ = CloseHandle(HANDLE(shared_handle_ptr));
                }
                drop(texture);
            })),
        };

        Ok(XbxRenderFrame {
            width,
            height,
            frame_seq: 0,
            rendered_at_ms: 0.0,
            rtp_timestamp: None,
            recovery_epoch_tag: None,
            recovery_owner_rtp_timestamp: None,
            is_keyframe: false,
            frame_recovery_disposition: None,
            frame_unrecoverable_reason: None,
            presentation_value_role: None,
            pixel_data: XbxEngineRenderPixelData::Descriptor {
                handle: Arc::new(descriptor),
            },
        })
    }
}

fn find_preferred_h264_decoder() -> *const ffi::AVCodec {
    let preferred_name = CString::new("h264_d3d11va").expect("static decoder name");
    let by_name = unsafe { ffi::avcodec_find_decoder_by_name(preferred_name.as_ptr()) };
    if !by_name.is_null() {
        return by_name;
    }
    unsafe { ffi::avcodec_find_decoder(ffi::AVCodecID::AV_CODEC_ID_H264) }
}

fn codec_name(codec: *const ffi::AVCodec) -> String {
    if codec.is_null() {
        return "null".to_string();
    }
    let name_ptr = unsafe { (*codec).name };
    if name_ptr.is_null() {
        return "unknown".to_string();
    }
    unsafe { std::ffi::CStr::from_ptr(name_ptr) }
        .to_string_lossy()
        .to_string()
}

impl super::backend::XbxVideoDecoderBackend for FfmpegWindowsD3d11vaDecoder {
    fn backend_name(&self) -> &'static str {
        "ffmpeg-d3d11va"
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

        // 和软件路径保持一致，直接喂完整 Annex-B access unit。
        // 之前这里把输入重打成 AVCC，但没有同步设置 avcC/extradata 上下文，
        // 会让 D3D11VA 进入“send 成功但长期不出帧”的灰区。
        self.decode_with_payload(encoded_frame.payload.as_ref(), encoded_frame.is_keyframe)
    }
}

impl FfmpegWindowsD3d11vaDecoder {
    fn decode_with_payload(
        &mut self,
        payload: &[u8],
        is_keyframe: bool,
    ) -> Result<super::backend::XbxVideoDecoderBackendDecodeOutcome, XbxEngineRuntimeError> {
        self.queue_packet(payload, is_keyframe)?;
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

impl Drop for FfmpegWindowsD3d11vaDecoder {
    fn drop(&mut self) {
        unsafe {
            if !self.sws_ctx.is_null() {
                ffi::sws_freeContext(self.sws_ctx);
                self.sws_ctx = std::ptr::null_mut();
            }
            if !self.hw_device_ctx.is_null() {
                ffi::av_buffer_unref(&mut self.hw_device_ctx);
            }
            if !self.sw_frame.is_null() {
                ffi::av_frame_free(&mut self.sw_frame);
            }
            if !self.hw_frame.is_null() {
                ffi::av_frame_free(&mut self.hw_frame);
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

fn codec_supports_d3d11va(codec: *const ffi::AVCodec) -> bool {
    let mut index = 0;
    loop {
        let config = unsafe { ffi::avcodec_get_hw_config(codec, index) };
        if config.is_null() {
            return false;
        }
        let supports_device_ctx = unsafe {
            ((*config).methods & ffi::AV_CODEC_HW_CONFIG_METHOD_HW_DEVICE_CTX as i32) != 0
        };
        let matches_d3d11 =
            unsafe { (*config).device_type == ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_D3D11VA };
        if supports_device_ctx && matches_d3d11 {
            return true;
        }
        index += 1;
    }
}

unsafe extern "C" fn select_d3d11va_pixel_format(
    ctx: *mut ffi::AVCodecContext,
    formats: *const ffi::AVPixelFormat,
) -> ffi::AVPixelFormat {
    if formats.is_null() {
        return ffi::AVPixelFormat::AV_PIX_FMT_NONE;
    }
    let mut current = formats;
    while unsafe { *current } != ffi::AVPixelFormat::AV_PIX_FMT_NONE {
        if unsafe { *current } == ffi::AVPixelFormat::AV_PIX_FMT_D3D11VA_VLD {
            let setup_result = unsafe { configure_d3d11va_hw_frames_ctx(ctx) };
            if let Err(error) = setup_result {
                log::warn!(
                    "[video][decode][ffmpeg-d3d11va] hw_frames_ctx setup failed, skip d3d11va format: {}",
                    error
                );
                current = unsafe { current.add(1) };
                continue;
            }
            return ffi::AVPixelFormat::AV_PIX_FMT_D3D11VA_VLD;
        }
        current = unsafe { current.add(1) };
    }
    unsafe { *formats }
}

unsafe fn configure_d3d11va_hw_frames_ctx(
    ctx: *mut ffi::AVCodecContext,
) -> Result<(), XbxEngineRuntimeError> {
    if ctx.is_null() {
        return Err(error_from_message(
            "xbxEngineFfmpegD3d11vaCodecContextMissing",
            "codecContext=null",
        ));
    }
    if unsafe { !(*ctx).hw_frames_ctx.is_null() } {
        return Ok(());
    }
    if unsafe { (*ctx).hw_device_ctx.is_null() } {
        return Err(error_from_message(
            "xbxEngineFfmpegD3d11vaDeviceContextMissing",
            "hw_device_ctx=null",
        ));
    }

    let mut frames_ref: *mut ffi::AVBufferRef = std::ptr::null_mut();
    let status = unsafe {
        ffi::avcodec_get_hw_frames_parameters(
            ctx,
            (*ctx).hw_device_ctx,
            ffi::AVPixelFormat::AV_PIX_FMT_D3D11VA_VLD,
            &mut frames_ref,
        )
    };
    if status < 0 || frames_ref.is_null() {
        return Err(ffmpeg_error(
            "xbxEngineFfmpegGetD3d11vaHwFramesParametersFailed",
            status,
        ));
    }

    let frames_ctx = unsafe { (*frames_ref).data.cast::<ffi::AVHWFramesContext>() };
    if frames_ctx.is_null() {
        unsafe {
            ffi::av_buffer_unref(&mut frames_ref);
        }
        return Err(error_from_message(
            "xbxEngineFfmpegD3d11vaFramesContextMissing",
            "AVHWFramesContext=null",
        ));
    }

    let d3d11va_hwctx = unsafe { (*frames_ctx).hwctx.cast::<AVD3D11VAFramesContextCompat>() };
    if d3d11va_hwctx.is_null() {
        unsafe {
            ffi::av_buffer_unref(&mut frames_ref);
        }
        return Err(error_from_message(
            "xbxEngineFfmpegD3d11vaFramesHwctxMissing",
            "hwctx=null",
        ));
    }

    unsafe {
        (*d3d11va_hwctx).bind_flags |=
            D3D11_BIND_DECODER.0 as u32 | D3D11_BIND_SHADER_RESOURCE.0 as u32;
        (*d3d11va_hwctx).misc_flags |=
            D3D11_RESOURCE_MISC_SHARED.0 as u32 | D3D11_RESOURCE_MISC_SHARED_NTHANDLE.0 as u32;
    }

    let init_status = unsafe { ffi::av_hwframe_ctx_init(frames_ref) };
    if init_status < 0 {
        unsafe {
            ffi::av_buffer_unref(&mut frames_ref);
        }
        return Err(ffmpeg_error(
            "xbxEngineFfmpegInitD3d11vaHwFramesContextFailed",
            init_status,
        ));
    }

    unsafe {
        (*ctx).hw_frames_ctx = frames_ref;
    }
    Ok(())
}

fn map_color_matrix(colorspace: ffi::AVColorSpace) -> MacOsVideoColorMatrix {
    match colorspace {
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

fn map_color_primaries(primaries: ffi::AVColorPrimaries) -> MacOsVideoColorPrimaries {
    match primaries {
        ffi::AVColorPrimaries::AVCOL_PRI_BT709 => MacOsVideoColorPrimaries::Bt709,
        ffi::AVColorPrimaries::AVCOL_PRI_BT2020 => MacOsVideoColorPrimaries::Bt2020,
        _ => MacOsVideoColorPrimaries::Unknown,
    }
}

fn map_transfer_function(trc: ffi::AVColorTransferCharacteristic) -> MacOsVideoTransferFunction {
    match trc {
        ffi::AVColorTransferCharacteristic::AVCOL_TRC_BT709 => MacOsVideoTransferFunction::Bt709,
        ffi::AVColorTransferCharacteristic::AVCOL_TRC_IEC61966_2_1 => {
            MacOsVideoTransferFunction::Srgb
        }
        ffi::AVColorTransferCharacteristic::AVCOL_TRC_LINEAR => MacOsVideoTransferFunction::Linear,
        _ => MacOsVideoTransferFunction::Unknown,
    }
}

fn map_color_range(range: ffi::AVColorRange) -> MacOsVideoColorRange {
    match range {
        ffi::AVColorRange::AVCOL_RANGE_JPEG => MacOsVideoColorRange::Full,
        _ => MacOsVideoColorRange::Video,
    }
}

fn map_chroma_location(location: ffi::AVChromaLocation) -> MacOsVideoChromaLocation {
    match location {
        ffi::AVChromaLocation::AVCHROMA_LOC_LEFT => MacOsVideoChromaLocation::Left,
        ffi::AVChromaLocation::AVCHROMA_LOC_TOPLEFT => MacOsVideoChromaLocation::TopLeft,
        ffi::AVChromaLocation::AVCHROMA_LOC_CENTER => MacOsVideoChromaLocation::Center,
        _ => MacOsVideoChromaLocation::Unknown,
    }
}
