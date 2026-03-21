use std::future::Future;
use std::pin::Pin;

use crate::media::video::types::AssembledVideoFrame;

pub trait TransportFeedbackPort: Send + Sync {
    fn send_transport_layer_nack<'a>(
        &'a self,
        media_ssrc: u32,
        sequences: &'a [u16],
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransportAdmissionObservation {
    AwaitRecoveryKeyframe,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransportLossObservation {
    PacketLossDetected,
    RecoveryKeyframeRequested,
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
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<TransportObservation>> + Send + 'a>>;
}

pub struct VideoFramePipelineSources {
    pub frame_source: Box<dyn FrameSource>,
    pub transport_observation_source: Box<dyn TransportObservationSource>,
}
