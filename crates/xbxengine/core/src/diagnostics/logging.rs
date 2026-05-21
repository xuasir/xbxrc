use std::time::SystemTime;
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU8, Ordering},
        Arc, OnceLock,
    },
    time::UNIX_EPOCH,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum XbxLogLevel {
    Error = 1,
    Warn = 2,
    Info = 3,
    Debug = 4,
    Trace = 5,
}

impl XbxLogLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            XbxLogLevel::Error => "ERROR",
            XbxLogLevel::Warn => "WARN",
            XbxLogLevel::Info => "INFO",
            XbxLogLevel::Debug => "DEBUG",
            XbxLogLevel::Trace => "TRACE",
        }
    }
}

static LOG_LEVEL: AtomicU8 = AtomicU8::new(XbxLogLevel::Warn as u8); // 默认 Warn
static STDERR_ENABLED: AtomicBool = AtomicBool::new(true);
static LOG_SINK_MIN_LEVEL: AtomicU8 = AtomicU8::new(XbxLogLevel::Warn as u8);
static LOG_SINK: OnceLock<Arc<dyn Fn(&XbxLogRecord) + Send + Sync>> = OnceLock::new();

#[derive(Clone, Debug)]
pub struct XbxLogRecord {
    pub ts_ms: u128,
    pub level: XbxLogLevel,
    pub message: String,
}

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

fn sink_level() -> Option<XbxLogLevel> {
    match LOG_SINK_MIN_LEVEL.load(Ordering::Relaxed) {
        0 => None,
        1 => Some(XbxLogLevel::Error),
        2 => Some(XbxLogLevel::Warn),
        3 => Some(XbxLogLevel::Info),
        4 => Some(XbxLogLevel::Debug),
        5 => Some(XbxLogLevel::Trace),
        _ => Some(XbxLogLevel::Warn),
    }
}

pub fn set_stderr_enabled(enabled: bool) {
    STDERR_ENABLED.store(enabled, Ordering::Relaxed);
}

pub fn set_log_sink_min_level(level: Option<XbxLogLevel>) {
    let raw = match level {
        Some(level) => level as u8,
        None => 0,
    };
    LOG_SINK_MIN_LEVEL.store(raw, Ordering::Relaxed);
}

pub fn set_log_sink(sink: Arc<dyn Fn(&XbxLogRecord) + Send + Sync>) {
    let _ = LOG_SINK.set(sink);
}

pub fn xbx_log_enabled(level: XbxLogLevel) -> bool {
    configured_level().is_some_and(|configured| level <= configured)
}

/// Per-frame decode/pacer/renderer diagnostics; default log level (Warn) keeps this off.
pub fn playback_flow_log_enabled() -> bool {
    xbx_log_enabled(XbxLogLevel::Debug)
}

pub fn xbx_log(level: XbxLogLevel, args: std::fmt::Arguments<'_>) {
    if !xbx_log_enabled(level) {
        return;
    }
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis())
        .unwrap_or(0);
    let record = XbxLogRecord {
        ts_ms: now_ms,
        level,
        message: args.to_string(),
    };
    if let Some(min_level) = sink_level() {
        if level <= min_level {
            if let Some(sink) = LOG_SINK.get() {
                sink(&record);
            }
        }
    }
    if STDERR_ENABLED.load(Ordering::Relaxed) {
        eprintln!(
            "[xbxengine][{}][{}] {}",
            record.level.as_str(),
            record.ts_ms,
            record.message
        );
    }
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

#[macro_export]
macro_rules! xbx_log_playback_flow {
    ($($arg:tt)*) => {{
        if $crate::logging::playback_flow_log_enabled() {
            $crate::logging::xbx_log(
                $crate::logging::XbxLogLevel::Debug,
                format_args!($($arg)*),
            );
        }
    }};
}
