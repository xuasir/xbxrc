use std::sync::Once;

use ffmpeg_sys_next as ffi;

use crate::XbxEngineRuntimeError;

static FFMPEG_INIT: Once = Once::new();

pub(crate) fn ffmpeg_init_once() {
    FFMPEG_INIT.call_once(|| unsafe {
        // Modern FFmpeg versions don't require explicit global registration.
        // Keep this hook for one-time process-level initialization.
        ffi::av_log_set_level(ffi::AV_LOG_ERROR as i32);
    });
}

pub(crate) fn ffmpeg_error(operation: &'static str, code: i32) -> XbxEngineRuntimeError {
    let mut errbuf = [0i8; 256];
    unsafe {
        ffi::av_strerror(code, errbuf.as_mut_ptr(), errbuf.len());
    }
    let message = unsafe { std::ffi::CStr::from_ptr(errbuf.as_ptr()) }
        .to_string_lossy()
        .into_owned();
    XbxEngineRuntimeError::new(format!("{operation}:status={code}:detail={message}"))
}

pub(crate) fn error_from_message(
    operation: &'static str,
    message: impl Into<String>,
) -> XbxEngineRuntimeError {
    XbxEngineRuntimeError::new(format!("{operation}:{}", message.into()))
}

pub(crate) fn av_err_eagain() -> i32 {
    ffi::AVERROR(ffi::EAGAIN)
}

pub(crate) fn av_err_eof() -> i32 {
    ffi::AVERROR_EOF
}

pub(crate) fn av_err_invaliddata() -> i32 {
    ffi::AVERROR_INVALIDDATA
}

/// `avcodec_send_packet` 之后应循环 `avcodec_receive_frame` 直至返回 EAGAIN/EOF。
/// 同样在 send 返回 EAGAIN 时，需先排空已解码输出再重试 send。
pub(crate) fn receive_decoded_frames_until_eagain<F, T, E>(
    mut receive_one: F,
) -> Result<(Vec<T>, i32), E>
where
    F: FnMut() -> Result<(Option<T>, i32), E>,
{
    let mut frames = Vec::new();
    loop {
        let (frame, status) = receive_one()?;
        match frame {
            Some(frame) => frames.push(frame),
            None => return Ok((frames, status)),
        }
    }
}

pub(crate) fn runtime_error_status_code(error: &XbxEngineRuntimeError) -> Option<i32> {
    let message = error.to_string();
    let status = message.split("status=").nth(1)?;
    let token = status
        .split(|ch: char| !(ch == '-' || ch.is_ascii_digit()))
        .next()?;
    token.parse::<i32>().ok()
}
