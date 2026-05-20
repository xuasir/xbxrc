use std::sync::Mutex;

use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::policy::video_scheduling_owner::VideoSchedulingOwnerState;
use crate::transport::rtc::recovery::contract::{
    has_current_clean_anchor_from_stats, is_timeline_chain_receiving_from_stats,
};
use crate::transport::rtc::recovery::coordinator::CoordinatorProposal;
use crate::transport::rtc::recovery::escalation::{RecoveryAction, VideoEscalationReason};
use crate::transport::rtc::recovery::runtime_state::{
    has_fresh_media_output, renderer_shadow_blocks_serviceability,
};
use crate::XbxEngineMediaRuntimeStats;

pub(crate) const RECOVERY_RAMP_UP_LIGHT_SIGNAL_HOLD_MS: f64 = 300.0;

pub(crate) struct RecoveryRampResolution {
    pub(crate) should_acknowledge_clean_anchor: bool,
    pub(crate) should_close_ramp_up: bool,
}

pub(crate) fn ramp_up_active(runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>) -> bool {
    RuntimeStatsSink::read_shared(runtime_stats, |stats| {
        stats.transport_recovery_episode_active && has_current_clean_anchor_from_stats(stats)
    })
    .unwrap_or(false)
}

pub(crate) fn should_absorb_light_recovery_signal_during_ramp_up(
    runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
    owner_state: VideoSchedulingOwnerState,
    proposal: &CoordinatorProposal,
    owner_signal: &crate::transport::rtc::recovery::coordinator::RecoveryOwnerSignal,
    observed_at_ms: f64,
    adapter_idle_render_slack_window_ms: f64,
    transport_await_diagnosis_is_short: bool,
) -> bool {
    if owner_state != VideoSchedulingOwnerState::StableServing
        || matches!(
            proposal.decision.action,
            RecoveryAction::RequestReconnectCandidate
        )
    {
        return false;
    }
    if !matches!(
        owner_signal.reason,
        VideoEscalationReason::AdapterIdleTimeout
            | VideoEscalationReason::AdapterThinStream
            | VideoEscalationReason::TransportAwaitRecoveryKeyframe
    ) {
        return false;
    }
    RuntimeStatsSink::read_shared(runtime_stats, |stats| {
        let current_clean_anchor = has_current_clean_anchor_from_stats(stats);
        if !current_clean_anchor || !stats.transport_recovery_episode_active {
            return false;
        }
        let still_in_ramp_up_window =
            stats
                .video_anchor_clean_observed_at_ms
                .is_some_and(|anchor_at_ms| {
                    (observed_at_ms - anchor_at_ms).max(0.0)
                        <= RECOVERY_RAMP_UP_LIGHT_SIGNAL_HOLD_MS
                });
        if !still_in_ramp_up_window {
            return false;
        }
        let pipeline_not_stalled = !stats.video_decoder_stalled.unwrap_or(false)
            && !renderer_shadow_blocks_serviceability(stats, observed_at_ms);
        if !pipeline_not_stalled {
            return false;
        }
        match owner_signal.reason {
            VideoEscalationReason::AdapterIdleTimeout
            | VideoEscalationReason::AdapterThinStream => {
                stats
                    .latest_video_host_present_time_ms
                    .is_some_and(|at_ms| {
                        (observed_at_ms - at_ms).max(0.0) <= adapter_idle_render_slack_window_ms
                    })
                    || stats.latest_video_decode_ok_time_ms.is_some_and(|at_ms| {
                        (observed_at_ms - at_ms).max(0.0) <= adapter_idle_render_slack_window_ms
                    })
            }
            VideoEscalationReason::TransportAwaitRecoveryKeyframe => {
                let chain_healthy = is_timeline_chain_receiving_from_stats(stats);
                let track_attached_with_video = stats
                    .latest_video_track_status
                    .as_ref()
                    .is_some_and(|track| {
                        track.state == "remoteTrackAttached" && track.video_bytes_total > 0
                    });
                transport_await_diagnosis_is_short
                    && chain_healthy
                    && track_attached_with_video
                    && has_fresh_media_output(stats, observed_at_ms)
            }
            _ => false,
        }
    })
    .unwrap_or(false)
}

pub(crate) fn resolve_stable_recovery_settle(
    runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
    owner_state: VideoSchedulingOwnerState,
    clean_anchor_epoch: Option<u64>,
    recovery_epoch: u64,
    _observed_at_ms: f64,
    has_unresolved_transport_await_issue: bool,
) -> RecoveryRampResolution {
    if !matches!(
        owner_state,
        VideoSchedulingOwnerState::StableServing | VideoSchedulingOwnerState::DegradedServing
    ) || !clean_anchor_epoch.is_some_and(|epoch| epoch == recovery_epoch)
    {
        return RecoveryRampResolution {
            should_acknowledge_clean_anchor: false,
            should_close_ramp_up: false,
        };
    }
    let should_acknowledge_clean_anchor = RuntimeStatsSink::read_shared(runtime_stats, |stats| {
        has_current_clean_anchor_from_stats(stats)
            && clean_anchor_epoch.is_some_and(|epoch| epoch == stats.transport_recovery_epoch)
    })
    .unwrap_or(false);
    let should_close_ramp_up = RuntimeStatsSink::read_shared(runtime_stats, |stats| {
        stats.transport_recovery_episode_active
            && has_current_clean_anchor_from_stats(stats)
            && clean_anchor_epoch.is_some_and(|epoch| epoch == stats.transport_recovery_epoch)
            && !has_unresolved_transport_await_issue
    })
    .unwrap_or(false);
    RecoveryRampResolution {
        should_acknowledge_clean_anchor,
        should_close_ramp_up,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::resolve_stable_recovery_settle;
    use crate::transport::rtc::policy::video_scheduling_owner::VideoSchedulingOwnerState;
    use crate::XbxEngineMediaRuntimeStats;

    #[test]
    fn stable_settle_closes_immediately_after_display_stable() {
        let runtime_stats = Mutex::new(XbxEngineMediaRuntimeStats {
            transport_recovery_episode_active: true,
            transport_recovery_epoch: 3,
            video_anchor_clean_epoch: Some(3),
            video_anchor_clean_observed_at_ms: Some(2_590.0),
            video_anchor_clean_source_event: Some("chain-clean-anchor-submitted".to_string()),
            ..Default::default()
        });

        let resolution = resolve_stable_recovery_settle(
            &runtime_stats,
            VideoSchedulingOwnerState::StableServing,
            Some(3),
            3,
            2_600.0,
            false,
        );

        assert!(resolution.should_acknowledge_clean_anchor);
        assert!(resolution.should_close_ramp_up);
    }

    #[test]
    fn stable_settle_accepts_serving_state_without_extra_hold() {
        let runtime_stats = Mutex::new(XbxEngineMediaRuntimeStats {
            transport_recovery_episode_active: true,
            transport_recovery_epoch: 3,
            video_anchor_clean_epoch: Some(3),
            video_anchor_clean_observed_at_ms: Some(1_000.0),
            video_anchor_clean_source_event: Some("chain-clean-anchor-submitted".to_string()),
            ..Default::default()
        });

        let resolution = resolve_stable_recovery_settle(
            &runtime_stats,
            VideoSchedulingOwnerState::StableServing,
            Some(3),
            3,
            2_600.0,
            false,
        );

        assert!(resolution.should_acknowledge_clean_anchor);
        assert!(resolution.should_close_ramp_up);
    }

    #[test]
    fn stable_settle_rejects_unresolved_transport_await() {
        let runtime_stats = Mutex::new(XbxEngineMediaRuntimeStats {
            transport_recovery_episode_active: true,
            transport_recovery_epoch: 3,
            video_anchor_clean_epoch: Some(3),
            video_anchor_clean_observed_at_ms: Some(1_000.0),
            video_anchor_clean_source_event: Some("chain-clean-anchor-submitted".to_string()),
            ..Default::default()
        });

        let resolution = resolve_stable_recovery_settle(
            &runtime_stats,
            VideoSchedulingOwnerState::StableServing,
            Some(3),
            3,
            2_600.0,
            true,
        );

        assert!(resolution.should_acknowledge_clean_anchor);
        assert!(!resolution.should_close_ramp_up);
    }
}
