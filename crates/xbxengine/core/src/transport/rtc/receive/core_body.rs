//! `RtcReceiveCore` 拥有的 receiver-local 运行时（engine + transport capability）。

use std::sync::Arc;

use super::engine::ReceiveEngine;
use crate::transport::rtc::capability::RtcTransportCapability;

/// RFC 四层之 receive 层：packet buffer / 组帧 / bootstrap / NACK / keyframe 执行面。
pub(crate) struct ReceiveCoreBody {
    pub receive_engine: ReceiveEngine,
    pub transport_capability: Arc<dyn RtcTransportCapability>,
}

impl ReceiveCoreBody {
    pub(crate) fn new(
        receive_engine: ReceiveEngine,
        transport_capability: Arc<dyn RtcTransportCapability>,
    ) -> Self {
        Self {
            receive_engine,
            transport_capability,
        }
    }
}
