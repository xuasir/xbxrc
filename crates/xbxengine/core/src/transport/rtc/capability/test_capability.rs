//! 单测用 transport capability：启用 receiver-local NACK/关键帧路径，不依赖真实连接。

use super::feedback::{
    KeyframeRequestKind, KeyframeSendOutcome, TransportCapabilityError, VideoFeedbackState,
};
use super::RtcTransportCapability;

#[derive(Debug, Default)]
pub struct TestTransportCapability;

impl RtcTransportCapability for TestTransportCapability {
    fn video_feedback_state(&self) -> VideoFeedbackState {
        VideoFeedbackState::Ready
    }

    fn send_nack_rtcp(&self, _payload: &[u8]) -> Result<(), TransportCapabilityError> {
        Ok(())
    }

    fn send_keyframe(&self, _kind: KeyframeRequestKind) -> KeyframeSendOutcome {
        KeyframeSendOutcome::Sent
    }

    fn send_remb(&self, _kbps: u32) -> Result<(), TransportCapabilityError> {
        Ok(())
    }

    fn latest_rtt_ms(&self) -> Option<u32> {
        None
    }
}
