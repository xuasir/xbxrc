use std::sync::{Arc, Mutex};

use crate::api::backend::XbxEngineMediaRuntimeStats;
use crate::runtime_stats_sink::RuntimeStatsSink;

/// decode 后 latest-only / host present 低延迟控制（与 pre-decode 解耦）。
#[derive(Debug, Clone)]
pub struct PostDecodeLatencyController {
    runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
}

impl PostDecodeLatencyController {
    pub fn new(runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>) -> Self {
        Self { runtime_stats }
    }

    pub fn record_host_present_stall_throttle(&self, enabled: bool) {
        RuntimeStatsSink::new(self.runtime_stats.clone()).update(|stats| {
            stats.host_present_stall_decode_throttle = enabled;
        });
    }

    pub fn host_stall_throttle_enabled(&self) -> bool {
        RuntimeStatsSink::read_shared(self.runtime_stats.as_ref(), |stats| {
            stats.host_present_stall_decode_throttle
        })
        .unwrap_or(false)
    }
}
