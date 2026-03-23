use std::sync::Mutex;

use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::recovery::escalation::{RecoveryAction, VideoEscalationReason};
use crate::transport::rtc::recovery::policy::{RecoveryScenarioProfile, ScenarioPolicyResolver};
use crate::transport::rtc::recovery::runtime_state::{
    decoder_backend_failure_signal_is_active, unix_now_ms,
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
        let profile = ScenarioPolicyResolver::resolve_recovery_profile(
            stats.session_target_type.as_ref(),
            stats.transport_path.as_deref(),
        );
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
