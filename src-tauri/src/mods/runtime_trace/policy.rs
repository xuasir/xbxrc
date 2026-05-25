//! runtime-logs 写入策略：与设置中的 `runtime_trace_mode` 对齐。
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use xbxengine::XbxLogLevel;

static TRACE_EVENT_SAMPLE_TS_MS: OnceLock<Mutex<HashMap<String, f64>>> = OnceLock::new();

/// 按模式调整引擎日志是否进入 stderr 与 runtime trace sink（应用初始化时调用）。
pub fn apply_xbxengine_trace_logging(mode: &str) {
    xbxengine::set_stderr_enabled(false);
    match mode {
        "off" => {
            xbxengine::set_configured_level(Some(XbxLogLevel::Warn));
            xbxengine::set_log_sink_min_level(None);
        }
        "minimal" => {
            xbxengine::set_configured_level(Some(XbxLogLevel::Warn));
            // minimal 只把 Error 及以上写入 trace，避免 per-frame WARN 洪峰拖死进程。
            xbxengine::set_log_sink_min_level(Some(XbxLogLevel::Error));
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
        "minimal" => Duration::from_secs(5),
        "standard" => Duration::from_secs(2),
        _ => Duration::from_secs(1),
    }
}

/// `record_runtime_trace_observations` 最小调用间隔（与 xbxengine 16ms tick 解耦）。
pub fn trace_observation_tick_interval(mode: &str) -> Duration {
    match mode {
        "minimal" => Duration::from_millis(500),
        "standard" => Duration::from_millis(100),
        _ => Duration::from_millis(16),
    }
}

fn trace_event_sample_slot() -> &'static Mutex<HashMap<String, f64>> {
    TRACE_EVENT_SAMPLE_TS_MS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn should_emit_sampled_trace_event(now_ms: f64, key: &str, interval_ms: f64) -> bool {
    let Ok(mut slot) = trace_event_sample_slot().lock() else {
        return false;
    };
    let should_emit = slot
        .get(key)
        .map(|last| now_ms - *last >= interval_ms)
        .unwrap_or(true);
    if should_emit {
        slot.insert(key.to_string(), now_ms);
    }
    should_emit
}

/// 高频 UI / 原始事件在 low-noise 模式下降采样；关键结构化事件仍全量保留。
pub fn should_record_trace_event(trace_mode: &str, domain: &str, event: &str, now_ms: f64) -> bool {
    match trace_mode {
        "minimal" => match (domain, event) {
            ("gamepad-shell", "runtimeSnapshotTransitionObserved") => {
                should_emit_sampled_trace_event(now_ms, "gamepad-shell:runtimeSnapshot", 2_000.0)
            }
            (
                "streaming-runtime-host",
                "gamepadUiRuntimeSnapshotApplied" | "gamepadUiRuntimeSnapshotRefreshed",
            ) => should_emit_sampled_trace_event(
                now_ms,
                &format!("streaming-runtime-host:{event}"),
                2_000.0,
            ),
            ("xbxengine", "runtimeEventRaw") => {
                should_emit_sampled_trace_event(now_ms, "xbxengine:runtimeEventRaw", 1_000.0)
            }
            ("xbxengine", "channelMessageCatalog") => {
                should_emit_sampled_trace_event(now_ms, "xbxengine:channelMessageCatalog", 2_000.0)
            }
            ("native_video", "layout_updated") => {
                should_emit_sampled_trace_event(now_ms, "native_video:layout_updated", 2_000.0)
            }
            _ => true,
        },
        "standard" => match (domain, event) {
            ("gamepad-shell", "runtimeSnapshotTransitionObserved")
            | ("streaming-runtime-host", "gamepadUiRuntimeSnapshotApplied")
            | ("streaming-runtime-host", "gamepadUiRuntimeSnapshotRefreshed") => {
                should_emit_sampled_trace_event(now_ms, &format!("{domain}:{event}"), 1_000.0)
            }
            _ => true,
        },
        _ => true,
    }
}
