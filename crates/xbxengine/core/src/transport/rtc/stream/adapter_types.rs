use crate::media::video::types::AssembledVideoFrame;

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
    NackDeadlineExpired { missing_packets: u16 },
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
