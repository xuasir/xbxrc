mod diagnostic_export;
mod diagnostic_records;
mod diagnostics_state;
mod export;
pub mod policy;
pub mod rpc;
mod runtime_state;
mod sdp_summary;
pub mod service;
mod trace_contract;
mod types;
mod writer;

pub use policy::{
    should_record_trace_event, stats_snapshot_interval, trace_observation_tick_interval,
    RuntimeTraceProfile, TraceDimension, TraceDimensionSet, TraceImportance, TraceMetadata,
};
pub use sdp_summary::summarize_sdp;
pub use service::{
    set_trace_root_dir, RuntimeTraceCategory, RuntimeTraceRecorder, RuntimeTraceRecorderRef,
};
pub use types::{TraceFlushStatus, TraceRecordDisposition};

/// 新配置或未写入磁盘时的默认 `runtime_trace_mode`（与 `defaults::default_config_map` 对齐）。
pub fn default_stored_trace_mode() -> String {
    if cfg!(debug_assertions) {
        "dev".to_string()
    } else {
        "production".to_string()
    }
}

/// 构建确定模式；开发启动可用 XBX_TRACE_MODE=production 复验生产成本。
pub fn effective_runtime_trace_mode(_stored: &str) -> String {
    if cfg!(debug_assertions)
        && std::env::var("XBX_TRACE_MODE").is_ok_and(|mode| mode == "production")
    {
        return "production".into();
    }
    default_stored_trace_mode()
}
