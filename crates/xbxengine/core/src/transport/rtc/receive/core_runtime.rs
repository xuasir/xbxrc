//! `RtcReceiveCore`：接收主线对外类型（RFC 四层之 receive 层入口）。

use std::sync::Arc;

use crate::media::video::types::AssembledVideoFrame;
use crate::transport::rtc::capability::RtcTransportCapability;
use crate::transport::rtc::receive::RtcVideoFrameSource;
use crate::transport::rtc::stream::adapter_types::FrameSource;

use super::core_body::ReceiveCoreBody;
use super::decode_gate::DecodeGate;
use super::engine::ReceiveEngine;
use super::receiver_state::ReceiverState;

/// RFC receive 层对外类型：decode gate + [`ReceiveCoreBody`] + ingress 编排。
pub struct RtcReceiveCore {
    decode_gate: DecodeGate,
    core_body: ReceiveCoreBody,
    ingress: RtcVideoFrameSource,
}

impl RtcReceiveCore {
    pub(crate) fn new(mut ingress: RtcVideoFrameSource) -> Self {
        let core_body = ingress.take_receive_core();
        Self {
            decode_gate: DecodeGate,
            core_body,
            ingress,
        }
    }

    pub(crate) fn ingress_mut(&mut self) -> &mut RtcVideoFrameSource {
        &mut self.ingress
    }

    pub(crate) fn receive_engine(&self) -> &ReceiveEngine {
        &self.core_body.receive_engine
    }

    pub(crate) fn receive_engine_mut(&mut self) -> &mut ReceiveEngine {
        &mut self.core_body.receive_engine
    }

    pub(crate) fn transport_capability(&self) -> Arc<dyn RtcTransportCapability> {
        self.core_body.transport_capability.clone()
    }

    pub(crate) fn receiver_state(&self) -> ReceiverState {
        self.ingress.receiver_local_state()
    }

    pub(crate) fn decode_gate(&self) -> &DecodeGate {
        &self.decode_gate
    }

    pub(crate) fn receive_core(&self) -> &ReceiveCoreBody {
        &self.core_body
    }

    pub(crate) fn receive_core_mut(&mut self) -> &mut ReceiveCoreBody {
        &mut self.core_body
    }
}

impl FrameSource for RtcReceiveCore {
    fn recv_frame<'a>(
        &'a mut self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<AssembledVideoFrame>> + Send + 'a>>
    {
        self.ingress.recv_frame()
    }
}
