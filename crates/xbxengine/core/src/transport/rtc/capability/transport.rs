use std::sync::{Arc, Mutex};

use crate::transport::rtc::connection::RtcConnectionService;
use crate::XbxEngineMediaRuntimeStats;

use super::feedback::{
    KeyframeRequestKind, KeyframeSendOutcome, TransportCapabilityError, VideoFeedbackState,
};

/// 窄 transport 能力面：连接、RTCP 写出、反馈目标与 RTT。
pub struct ConnectionTransportCapability {
    connection: Arc<Mutex<RtcConnectionService>>,
    runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
}

impl ConnectionTransportCapability {
    pub fn new(
        connection: Arc<Mutex<RtcConnectionService>>,
        runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) -> Self {
        Self {
            connection,
            runtime_stats,
        }
    }
}

pub trait RtcTransportCapability: Send + Sync {
    fn video_feedback_state(&self) -> VideoFeedbackState;
    fn send_nack_rtcp(&self, payload: &[u8]) -> Result<(), TransportCapabilityError>;
    fn send_keyframe(&self, kind: KeyframeRequestKind) -> KeyframeSendOutcome;
    #[allow(dead_code)]
    fn send_remb(&self, kbps: u32) -> Result<(), TransportCapabilityError>;
    #[allow(dead_code)]
    fn latest_rtt_ms(&self) -> Option<u32>;
}

impl RtcTransportCapability for ConnectionTransportCapability {
    fn video_feedback_state(&self) -> VideoFeedbackState {
        let Ok(mut connection) = self.connection.lock() else {
            return VideoFeedbackState::Unavailable;
        };
        connection.video_keyframe_feedback_state()
    }

    fn send_nack_rtcp(&self, payload: &[u8]) -> Result<(), TransportCapabilityError> {
        let mut connection =
            self.connection
                .lock()
                .map_err(|_| TransportCapabilityError::SendFailed {
                    detail: "connection lock failed".to_string(),
                })?;
        connection
            .send_video_rtcp_payload(payload)
            .map_err(|error| TransportCapabilityError::SendFailed {
                detail: error.to_string(),
            })
    }

    fn send_keyframe(&self, kind: KeyframeRequestKind) -> KeyframeSendOutcome {
        match self.video_feedback_state() {
            VideoFeedbackState::Unavailable => return KeyframeSendOutcome::TransportNotReady,
            VideoFeedbackState::Warming => return KeyframeSendOutcome::FeedbackWarming,
            VideoFeedbackState::Ready => {}
        }
        let Ok(mut connection) = self.connection.lock() else {
            return KeyframeSendOutcome::FeedbackUnavailable;
        };
        let stats = self.runtime_stats.clone();
        let control_result = if connection.control_keyframe_request_ready() {
            Some(connection.request_video_keyframe_control_direct(&stats))
        } else {
            None
        };
        let rtcp_result = match kind {
            KeyframeRequestKind::Pli => connection.request_video_pli_direct(&stats),
            KeyframeRequestKind::Fir => connection.request_video_fir_direct(&stats),
        };
        if rtcp_result.is_ok() || control_result.as_ref().is_some_and(Result::is_ok) {
            KeyframeSendOutcome::Sent
        } else {
            KeyframeSendOutcome::FeedbackUnavailable
        }
    }

    fn send_remb(&self, kbps: u32) -> Result<(), TransportCapabilityError> {
        let mut connection =
            self.connection
                .lock()
                .map_err(|_| TransportCapabilityError::SendFailed {
                    detail: "connection lock failed".to_string(),
                })?;
        connection
            .request_target_remb_kbps(kbps, &self.runtime_stats)
            .map_err(|error| TransportCapabilityError::SendFailed {
                detail: error.to_string(),
            })
    }

    fn latest_rtt_ms(&self) -> Option<u32> {
        self.runtime_stats.lock().ok().and_then(|stats| {
            stats
                .video_rtt_ms
                .or(stats.recovery_effective_rtt_ms)
                .map(|rtt| rtt as u32)
        })
    }
}
