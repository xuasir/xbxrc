use crate::transport::rtc::recovery::escalation::{
    VideoEscalationConfig, VideoEscalationController,
};
use crate::{XbxEngineVideoNackObservation, XbxEngineVideoTwccObservation};
pub(super) fn test_escalation_controller(
    cooldown_ms: u64,
    keyframe_burst_threshold: u8,
    decoder_reset_burst_threshold: u8,
) -> VideoEscalationController {
    VideoEscalationController::new(VideoEscalationConfig {
        cooldown_ms,
        keyframe_burst_threshold,
        decoder_reset_burst_threshold,
        keyframe_min_interval_ms: cooldown_ms,
        escalation_window_ms: cooldown_ms.saturating_mul(3),
        keyframe_upgrade_min_delay_ms: (cooldown_ms / 2).max(40),
    })
}
pub(super) fn healthy_twcc_observation(now_ms: f64) -> XbxEngineVideoTwccObservation {
    XbxEngineVideoTwccObservation {
        observation_id: 1,
        source: "local-feedback".to_string(),
        feedback_packet_count: 20,
        covered_sequence_start: 10,
        covered_sequence_end: 29,
        covered_sequence_span: 20,
        observed_packet_count: 20,
        observed_byte_count: 32_000,
        coverage_ratio: None,
        ledger_hit_ratio: None,
        feedback_interval_ms: Some(100.0),
        arrival_span_ms: Some(95.0),
        receive_bitrate_kbps: Some(18_000.0),
        twcc_sample_valid: true,

        twcc_invalid_reason: None,

        quality: crate::XbxEngineTwccObservationQuality::Stable,
        delivery_ratio: 0.99,
        packet_loss_ratio: 0.01,
        observed_at_ms: now_ms,
    }
}

pub(super) fn make_test_nack_observation(
    action: &str,
    frame_importance: &str,
    retry_count: u8,
    observed_at_ms: f64,
) -> XbxEngineVideoNackObservation {
    XbxEngineVideoNackObservation {
        observation_id: 1,
        action: action.to_string(),
        source: "sampleLoss".to_string(),
        first_sequence: 1,
        last_sequence: 2,
        packet_count: 2,
        retry_count,
        frame_rtp_timestamp: Some(1),
        frame_is_keyframe: Some(frame_importance == "keyframe"),
        frame_importance: Some(frame_importance.to_string()),
        deadline_at_ms: None,
        estimated_recovery_arrival_ms: None,
        nack_disposition: Some("attempted".to_string()),
        frame_playout_deadline_at_ms: None,
        frame_unrecoverable_reason: None,
        frame_budget: None,
        observed_at_ms,
    }
}
