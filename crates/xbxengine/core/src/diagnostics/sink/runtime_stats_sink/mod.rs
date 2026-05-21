//! 媒体 `XbxEngineMediaRuntimeStats` 的统一写入入口（sink）。
//! RFC：采集面只承载事实；诊断映射在 `diagnostics` / `trace_projection`，不得反向驱动控制决策。

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use crate::diagnostics::observation_bus::ObservationBus;

mod core;
mod ingress;
mod observations;
mod picture_recovery;
mod support;
mod transport_apply;
mod transport_recovery;

#[derive(Clone)]
pub(crate) struct RuntimeStatsSink {
    observation_bus: ObservationBus,
    picture_recovery_response_trace_cache:
        Arc<Mutex<VecDeque<PictureRecoveryResponseTraceCacheEntry>>>,
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct PictureRecoveryResponseTraceCacheEntry {
    episode_id: u64,
    first_video_packet_sequence: Option<u16>,
    first_keyframe_packet_sequence: Option<u16>,
}

#[cfg(test)]
#[path = "../runtime_stats_sink.test.rs"]
mod tests;
