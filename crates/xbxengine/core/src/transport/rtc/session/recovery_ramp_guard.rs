use std::sync::Mutex;

use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::policy::video_scheduling_owner::VideoSchedulingOwnerState;
use crate::transport::rtc::recovery::coordinator::RecoveryCoordinatorProposal;
use crate::transport::rtc::recovery::escalation::{RecoveryAction, VideoEscalationReason};
use crate::transport::rtc::recovery::runtime_state::has_fresh_media_output;
use crate::XbxEngineMediaRuntimeStats;

pub(crate) const RECOVERY_RAMP_UP_LIGHT_SIGNAL_HOLD_MS: f64 = 1_500.0;

pub(crate) struct RecoveryRampResolution {
    pub(crate) should_acknowledge_clean_anchor: bool,
    pub(crate) should_close_ramp_up: bool,
}

pub(crate) fn ramp_up_active(runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>) -> bool {
    RuntimeStatsSink::read_shared(runtime_stats, |stats| {
        stats.transport_recovery_episode_active
            && stats
                .video_anchor_clean_epoch
                .is_some_and(|epoch| epoch == stats.transport_recovery_epoch)
    })
    .unwrap_or(false)
}

pub(crate) fn should_absorb_light_recovery_signal_during_ramp_up(
    runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
    owner_state: VideoSchedulingOwnerState,
    proposal: &RecoveryCoordinatorProposal,
    observed_at_ms: f64,
    adapter_idle_render_slack_window_ms: f64,
    transport_await_diagnosis_is_short: bool,
) -> bool {
    if owner_state != VideoSchedulingOwnerState::StableServing
        || matches!(
            proposal.decision.action,
            RecoveryAction::RequestReconnectCandidate | RecoveryAction::RequestDecoderReset
        )
    {
        return false;
    }
    if !matches!(
        proposal.signal.reason,
        VideoEscalationReason::AdapterIdleTimeout
            | VideoEscalationReason::AdapterThinStream
            | VideoEscalationReason::TransportAwaitRecoveryKeyframe
    ) {
        return false;
    }
    RuntimeStatsSink::read_shared(runtime_stats, |stats| {
        let current_clean_anchor = stats
            .video_anchor_clean_epoch
            .is_some_and(|epoch| epoch == stats.transport_recovery_epoch)
            && stats.video_anchor_clean_source_event.as_deref()
                == Some("chain-clean-keyframe-submitted");
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
            && !stats.video_renderer_stalled.unwrap_or(false);
        if !pipeline_not_stalled {
            return false;
        }
        match proposal.signal.reason {
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
                let chain_healthy = stats
                    .latest_video_timeline_observation
                    .as_ref()
                    .is_some_and(|timeline| timeline.chain.state == "healthy");
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
    observed_at_ms: f64,
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
    let should_acknowledge_clean_anchor = !has_unresolved_transport_await_issue;
    let should_close_ramp_up = RuntimeStatsSink::read_shared(runtime_stats, |stats| {
        if !stats.transport_recovery_episode_active {
            return false;
        }
        let anchor_age_ms = stats
            .video_anchor_clean_observed_at_ms
            .map(|anchor_at_ms| (observed_at_ms - anchor_at_ms).max(0.0))
            .unwrap_or(f64::INFINITY);
        let pipeline_not_stalled = !stats.video_decoder_stalled.unwrap_or(false)
            && !stats.video_renderer_stalled.unwrap_or(false);
        pipeline_not_stalled
            && !has_unresolved_transport_await_issue
            && has_fresh_media_output(stats, observed_at_ms)
            && anchor_age_ms >= RECOVERY_RAMP_UP_LIGHT_SIGNAL_HOLD_MS
    })
    .unwrap_or(false);
    RecoveryRampResolution {
        should_acknowledge_clean_anchor,
        should_close_ramp_up,
    }
}
