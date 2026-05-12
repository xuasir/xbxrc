use crate::media::video::ingress::budget::FrameBudgetContext;
use crate::media::video::types::AssembledVideoFrame;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NackDeadlineExpiredContext {
    pub missing_packets: u16,
    pub frame_rtp_timestamp: Option<u32>,
    pub frame_importance: &'static str,
    pub budget_context: FrameBudgetContext,
    pub frame_unrecoverable_reason: Option<&'static str>,
    /// RFC 三档价值：`low` / `medium` / `high`。
    pub value_tier: &'static str,
    /// 合同化风险分层：`none` / `repairable` / `reference` / `anchor`。
    pub risk_tier: &'static str,
    /// RFC `evidence_scope`：`anonymous` / `frame_bound` / `chain_bound` / `anchor_bound`。
    pub evidence_scope: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransportAdmissionObservation {
    AwaitRecoveryKeyframe,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransportLossObservation {
    PacketLossDetected,
    RecoveryKeyframeRequested,
    #[allow(dead_code)]
    AwaitRecoveryKeyframe,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransportObservation {
    Admission(TransportAdmissionObservation),
    Loss(TransportLossObservation),
    StreamIdleTimeout,
    StreamThinStall,
    NackDeadlineExpired(NackDeadlineExpiredContext),
    NackRecoveredLate,
}

pub trait FrameSource: Send {
    fn recv_frame<'a>(
        &'a mut self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<AssembledVideoFrame>> + Send + 'a>>;
}

pub trait TransportObservationSource: Send {
    fn recv_transport_observation<'a>(
        &'a mut self,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Option<TransportObservation>> + Send + 'a>,
    >;
}

pub struct VideoFramePipelineSources {
    pub frame_source: Box<dyn FrameSource>,
    pub transport_observation_source: Box<dyn TransportObservationSource>,
}
