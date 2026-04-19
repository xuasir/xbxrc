pub mod policy;
pub mod rpc;
pub mod service;

pub use policy::{apply_xbxengine_trace_logging, stats_snapshot_interval};
pub use service::{RuntimeTraceRecorder, RuntimeTraceRecorderRef};

/// 新配置或未写入磁盘时的默认 `runtime_trace_mode`（与 `defaults::default_config_map` 对齐）。
pub fn default_stored_trace_mode() -> String {
    if cfg!(debug_assertions) {
        "minimal".to_string()
    } else {
        "off".to_string()
    }
}

/// 实际生效的 trace 模式：发行构建固定为 `off`（不写 runtime-logs、不挂引擎 trace sink）。
pub fn effective_runtime_trace_mode(stored: &str) -> String {
    if cfg!(debug_assertions) {
        stored.to_string()
    } else {
        "off".to_string()
    }
}
