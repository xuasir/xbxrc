use std::time::SystemTime;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum XbxLogLevel {
    Error = 1,
    Warn = 2,
    Info = 3,
    Debug = 4,
    Trace = 5,
}

impl XbxLogLevel {
    fn as_str(self) -> &'static str {
        match self {
            XbxLogLevel::Error => "ERROR",
            XbxLogLevel::Warn => "WARN",
            XbxLogLevel::Info => "INFO",
            XbxLogLevel::Debug => "DEBUG",
            XbxLogLevel::Trace => "TRACE",
        }
    }
}

use std::sync::atomic::{AtomicU8, Ordering};

static LOG_LEVEL: AtomicU8 = AtomicU8::new(XbxLogLevel::Warn as u8); // 默认 Warn

pub fn set_configured_level(level: Option<XbxLogLevel>) {
    let val = match level {
        Some(l) => l as u8,
        None => 0,
    };
    LOG_LEVEL.store(val, Ordering::Relaxed);
}

pub fn parse_level(raw: &str) -> Option<XbxLogLevel> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "off" | "none" | "0" => None,
        "error" => Some(XbxLogLevel::Error),
        "warn" | "warning" => Some(XbxLogLevel::Warn),
        "info" => Some(XbxLogLevel::Info),
        "debug" => Some(XbxLogLevel::Debug),
        "trace" => Some(XbxLogLevel::Trace),
        _ => Some(XbxLogLevel::Warn),
    }
}

fn configured_level() -> Option<XbxLogLevel> {
    match LOG_LEVEL.load(Ordering::Relaxed) {
        0 => None,
        1 => Some(XbxLogLevel::Error),
        2 => Some(XbxLogLevel::Warn),
        3 => Some(XbxLogLevel::Info),
        4 => Some(XbxLogLevel::Debug),
        5 => Some(XbxLogLevel::Trace),
        _ => Some(XbxLogLevel::Warn),
    }
}

pub fn xbx_log_enabled(level: XbxLogLevel) -> bool {
    configured_level().is_some_and(|configured| level <= configured)
}

pub fn xbx_log(level: XbxLogLevel, args: std::fmt::Arguments<'_>) {
    if !xbx_log_enabled(level) {
        return;
    }
    let now_ms = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_millis())
        .unwrap_or(0);
    eprintln!("[xbxengine][{}][{}] {}", level.as_str(), now_ms, args);
}

#[macro_export]
macro_rules! xbx_log_error {
    ($($arg:tt)*) => {{
        $crate::logging::xbx_log($crate::logging::XbxLogLevel::Error, format_args!($($arg)*));
    }};
}

#[macro_export]
macro_rules! xbx_log_warn {
    ($($arg:tt)*) => {{
        $crate::logging::xbx_log($crate::logging::XbxLogLevel::Warn, format_args!($($arg)*));
    }};
}

#[macro_export]
macro_rules! xbx_log_info {
    ($($arg:tt)*) => {{
        $crate::logging::xbx_log($crate::logging::XbxLogLevel::Info, format_args!($($arg)*));
    }};
}

#[macro_export]
macro_rules! xbx_log_debug {
    ($($arg:tt)*) => {{
        $crate::logging::xbx_log($crate::logging::XbxLogLevel::Debug, format_args!($($arg)*));
    }};
}

#[macro_export]
macro_rules! xbx_log_trace {
    ($($arg:tt)*) => {{
        $crate::logging::xbx_log($crate::logging::XbxLogLevel::Trace, format_args!($($arg)*));
    }};
}
