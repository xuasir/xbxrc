use std::sync::Mutex;

use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::recovery::escalation::{RecoveryAction, VideoEscalationReason};
use crate::transport::rtc::recovery::policy::RecoveryScenarioProfile;
use crate::transport::rtc::recovery::runtime_state::{
    decoder_backend_failure_signal_is_active, resolve_runtime_recovery_profile, unix_now_ms,
};
use crate::XbxEngineMediaRuntimeStats;

pub(crate) enum DecoderBackendFailureResolution {
    Suppress(RecoveryAction),
    Escalate(RecoveryScenarioProfile),
}

pub(crate) fn resolve_decoder_backend_failure_recovery(
    runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
    reason: &VideoEscalationReason,
) -> Option<DecoderBackendFailureResolution> {
    if !matches!(
        reason,
        VideoEscalationReason::TransportExpiredDeadline
            | VideoEscalationReason::TransportSevereDeadline
            | VideoEscalationReason::TransportSampleLoss
            | VideoEscalationReason::TransportAwaitRecoveryKeyframe
            | VideoEscalationReason::WaitKeyframe
            | VideoEscalationReason::AdapterIdleTimeout
    ) {
        return None;
    }

    let now_ms = unix_now_ms();
    let (profile, since_last_reset_ms) = RuntimeStatsSink::read_shared(runtime_stats, |stats| {
        let profile = resolve_runtime_recovery_profile(stats);
        if !decoder_backend_failure_signal_is_active(stats, profile, now_ms) {
            return None;
        }
        let since_last_reset_ms = stats
            .latest_video_decoder_reset_time_ms
            .map(|at_ms| (now_ms - at_ms).max(0.0))
            .unwrap_or(f64::INFINITY);
        Some((profile, since_last_reset_ms))
    })
    .flatten()?;

    if since_last_reset_ms < profile.decoder_backend_failure_min_reset_spacing_ms {
        return Some(DecoderBackendFailureResolution::Suppress(
            RecoveryAction::CooldownSuppressed,
        ));
    }

    Some(DecoderBackendFailureResolution::Escalate(profile))
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use xbxengine_protocol::XbxEngineTargetTypeDto;

    use super::{resolve_decoder_backend_failure_recovery, DecoderBackendFailureResolution};
    use crate::transport::rtc::recovery::escalation::VideoEscalationReason;
    use crate::transport::rtc::recovery::runtime_state::unix_now_ms;
    use crate::{XbxEngineMediaRuntimeStats, XbxEngineVideoTwccObservation};

    fn healthy_twcc_observation(observed_at_ms: f64) -> XbxEngineVideoTwccObservation {
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
            feedback_interval_ms: Some(80.0),
            arrival_span_ms: Some(70.0),
            receive_bitrate_kbps: Some(18_000.0),
            twcc_sample_valid: true,
            twcc_invalid_reason: None,
            quality: crate::XbxEngineTwccObservationQuality::Stable,
            delivery_ratio: 1.0,
            packet_loss_ratio: 0.0,
            observed_at_ms,
        }
    }

    #[test]
    fn decoder_backend_failure_uses_runtime_baseline_profile_kind() {
        let now_ms = unix_now_ms();
        let stats = XbxEngineMediaRuntimeStats {
            session_target_type: Some(XbxEngineTargetTypeDto::Cloud),
            baseline_remote_profile: Some("relayGaming".to_string()),
            transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
            latest_video_packet_arrival_time_ms: Some(now_ms - 30.0),
            latest_video_decode_ok_time_ms: Some(now_ms - 1_800.0),
            latest_video_host_present_time_ms: Some(now_ms - 1_800.0),
            video_renderer_stalled: Some(true),
            latest_video_twcc_observation: Some(healthy_twcc_observation(now_ms - 20.0)),
            video_decoder_hardware_failure_streak: 4,
            latest_video_decoder_hardware_failure_time_ms: Some(now_ms - 25.0),
            ..XbxEngineMediaRuntimeStats::default()
        };

        let resolution = resolve_decoder_backend_failure_recovery(
            &Mutex::new(stats),
            &VideoEscalationReason::TransportExpiredDeadline,
        );

        match resolution {
            Some(DecoderBackendFailureResolution::Escalate(profile)) => {
                assert_eq!(profile.kind.as_str(), "relayGaming");
            }
            _ => panic!("unexpected resolution"),
        }
    }
}
