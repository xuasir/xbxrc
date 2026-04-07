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
