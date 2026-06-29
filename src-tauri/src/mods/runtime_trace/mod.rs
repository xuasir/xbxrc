pub mod policy;
pub mod rpc;
pub mod service;

pub use policy::{
    apply_xbxengine_trace_logging, should_record_trace_event, stats_snapshot_interval,
    trace_observation_tick_interval, RuntimeTraceProfile, TraceDimension, TraceDimensionSet,
    TraceImportance, TraceMetadata,
};
pub use service::{RuntimeTraceCategory, RuntimeTraceRecorder, RuntimeTraceRecorderRef};

/// 新配置或未写入磁盘时的默认 `runtime_trace_mode`（与 `defaults::default_config_map` 对齐）。
pub fn default_stored_trace_mode() -> String {
    if cfg!(debug_assertions) {
        "dev".to_string()
    } else {
        "production".to_string()
    }
}

/// 实际生效的 trace 模式：发行构建会把 `dev` 降级为受预算约束的 `production`。
pub fn effective_runtime_trace_mode(stored: &str) -> String {
    policy::RuntimeTraceProfile::effective_from_stored_mode(stored)
        .as_str()
        .to_string()
}
