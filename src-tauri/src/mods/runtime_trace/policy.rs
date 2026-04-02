//! runtime-logs 写入策略：与设置中的 `runtime_trace_mode` 对齐。
use std::time::Duration;

use xbxengine::XbxLogLevel;

/// 按模式调整引擎日志是否进入 stderr 与 runtime trace sink（应用初始化时调用）。
pub fn apply_xbxengine_trace_logging(mode: &str) {
    xbxengine::set_stderr_enabled(false);
    match mode {
        "off" => {
            xbxengine::set_configured_level(Some(XbxLogLevel::Warn));
            xbxengine::set_log_sink_min_level(None);
        }
        "minimal" => {
            xbxengine::set_configured_level(Some(XbxLogLevel::Debug));
            xbxengine::set_log_sink_min_level(Some(XbxLogLevel::Warn));
        }
        "standard" => {
            xbxengine::set_configured_level(Some(XbxLogLevel::Debug));
            xbxengine::set_log_sink_min_level(Some(XbxLogLevel::Info));
        }
        "verbose" => {
            xbxengine::set_configured_level(Some(XbxLogLevel::Debug));
            xbxengine::set_log_sink_min_level(Some(XbxLogLevel::Debug));
        }
        "trace" => {
            xbxengine::set_configured_level(Some(XbxLogLevel::Trace));
            xbxengine::set_log_sink_min_level(Some(XbxLogLevel::Trace));
        }
        _ => {
            xbxengine::set_configured_level(Some(XbxLogLevel::Debug));
            xbxengine::set_log_sink_min_level(Some(XbxLogLevel::Warn));
        }
    }
}

/// `statsSnapshot` / `observabilitySnapshot` 写入间隔：模式越低噪音，间隔越长。
pub fn stats_snapshot_interval(mode: &str) -> Duration {
    match mode {
        "minimal" => Duration::from_secs(3),
        "standard" => Duration::from_secs(2),
        _ => Duration::from_secs(1),
    }
}
