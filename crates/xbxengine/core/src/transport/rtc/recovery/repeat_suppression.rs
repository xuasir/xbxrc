use std::sync::Mutex;

use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::recovery::escalation::{RecoveryAction, VideoEscalationReason};
use crate::transport::rtc::recovery::runtime_state::{has_fresh_media_output, unix_now_ms};
use crate::{
    XbxEngineAnchorCandidateState, XbxEngineMediaRuntimeStats, XbxEngineVideoTimelineObservation,
};

const WAIT_KEYFRAME_REPEAT_SUPPRESS_MS: f64 = 260.0;
const IDLE_TIMEOUT_REPEAT_SUPPRESS_MS: f64 = 360.0;
const TRANSPORT_AWAIT_DEBT_FRESH_MS: f64 = 900.0;

pub(crate) fn resolve_recent_repeat_suppression(
    runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
    reason: &VideoEscalationReason,
) -> Option<RecoveryAction> {
    let now_ms = unix_now_ms();
    let (escalation, has_new_transport_recovery_epoch, active_transport_await_debt) =
        RuntimeStatsSink::read_shared(runtime_stats, |stats| {
            let escalation = stats.latest_video_escalation_observation.clone()?;
            let has_new_transport_recovery_epoch =
                stats.transport_recovery_epoch > stats.transport_recovery_epoch_at_last_escalation;
            let active_transport_await_debt = has_active_transport_await_debt(stats, now_ms);
            Some((
                escalation,
                has_new_transport_recovery_epoch,
                active_transport_await_debt,
            ))
        })
        .flatten()?;
    let elapsed_ms = now_ms - escalation.observed_at_ms;
    if elapsed_ms < 0.0 {
        return None;
    }

    match reason {
        VideoEscalationReason::WaitKeyframe
        | VideoEscalationReason::TransportAwaitRecoveryKeyframe => {
            let same_wait_keyframe_chain = matches!(
                escalation.reason.as_str(),
                "waitKeyframe"
                    | "ingressWaitKeyframe"
                    | "ingressFrameAbandoned"
                    | "transportAwaitRecoveryKeyframe"
            );
            let active_recovery_action =
                coalesced_action_for_existing_family(escalation.action.as_str());
            if same_wait_keyframe_chain
                && active_recovery_action.is_some()
                && active_transport_await_debt
            {
                return active_recovery_action;
            }
            if same_wait_keyframe_chain
                && active_recovery_action.is_some()
                && !has_new_transport_recovery_epoch
                && elapsed_ms <= WAIT_KEYFRAME_REPEAT_SUPPRESS_MS
            {
                return active_recovery_action;
            }
        }
        VideoEscalationReason::DisplaySupplyCritical => {
            let same_local_display_chain = escalation.reason == "displaySupplyCritical";
            let decoder_reset_inflight = matches!(
                coalesced_action_for_existing_family(escalation.action.as_str()),
                Some(RecoveryAction::CoalescedDecoderResetInFlight)
            );
            if same_local_display_chain
                && decoder_reset_inflight
                && !has_new_transport_recovery_epoch
                && elapsed_ms <= IDLE_TIMEOUT_REPEAT_SUPPRESS_MS
            {
                return Some(RecoveryAction::CoalescedDecoderResetInFlight);
            }
        }
        VideoEscalationReason::AdapterIdleTimeout => {
            let same_idle_chain = escalation.reason == "adapterIdleTimeout";
            let decoder_reset_inflight = matches!(
                coalesced_action_for_existing_family(escalation.action.as_str()),
                Some(RecoveryAction::CoalescedDecoderResetInFlight)
            );
            if same_idle_chain
                && decoder_reset_inflight
                && !has_new_transport_recovery_epoch
                && elapsed_ms <= IDLE_TIMEOUT_REPEAT_SUPPRESS_MS
            {
                return Some(RecoveryAction::CoalescedDecoderResetInFlight);
            }
        }
        _ => {}
    }

    None
}

fn coalesced_action_for_existing_family(action: &str) -> Option<RecoveryAction> {
    if matches!(
        action,
        "requestDecoderReset"
            | "requestKeyframe+decoderReset"
            | "requestKeyframe+decoderReset(startupLowQualityRetry)"
    ) {
        return Some(RecoveryAction::CoalescedDecoderResetInFlight);
    }
    if action.starts_with("requestKeyframe") {
        return Some(RecoveryAction::CoalescedKeyframeInFlight);
    }
    None
}

fn has_active_transport_await_debt(stats: &XbxEngineMediaRuntimeStats, now_ms: f64) -> bool {
    if has_objective_transport_recovery_success(stats, now_ms) {
        return false;
    }
    let latest_anchor_await =
        stats
            .latest_anchor_candidate_ledger
            .as_ref()
            .is_some_and(|candidate| {
                candidate.state == XbxEngineAnchorCandidateState::AwaitingRecovery
                    && (now_ms - candidate.observed_at_ms).max(0.0) <= TRANSPORT_AWAIT_DEBT_FRESH_MS
                    && matches!(
                        candidate.source_event.as_str(),
                        "frame-await-recovery-keyframe"
                            | "gap-repair-in-flight"
                            | "gap-reorder-pending"
                    )
            });
    let timeline_unresolved = stats
        .latest_video_timeline_observation
        .as_ref()
        .is_some_and(|timeline| {
            (now_ms - timeline.observed_at_ms).max(0.0) <= TRANSPORT_AWAIT_DEBT_FRESH_MS
                && is_unresolved_transport_await_timeline(timeline)
        });
    latest_anchor_await || timeline_unresolved
}

fn has_objective_transport_recovery_success(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> bool {
    let current_clean_anchor = stats
        .video_anchor_clean_epoch
        .is_some_and(|epoch| epoch == stats.transport_recovery_epoch)
        && stats.video_anchor_clean_source_event.as_deref()
            == Some("chain-clean-keyframe-submitted");
    let chain_healthy = stats
        .latest_video_timeline_observation
        .as_ref()
        .is_some_and(|timeline| timeline.chain.state == "healthy");
    let track_attached_with_video = stats
        .latest_video_track_status
        .as_ref()
        .is_some_and(|track| track.state == "remoteTrackAttached" && track.video_bytes_total > 0);
    let pipeline_not_stalled = !stats.video_decoder_stalled.unwrap_or(false)
        && !stats.video_renderer_stalled.unwrap_or(false);
    current_clean_anchor
        && chain_healthy
        && track_attached_with_video
        && pipeline_not_stalled
        && has_fresh_media_output(stats, now_ms)
}

fn is_unresolved_transport_await_timeline(timeline: &XbxEngineVideoTimelineObservation) -> bool {
    matches!(
        timeline.chain.state.as_str(),
        "recovering" | "broken" | "repairing" | "stalled"
    ) && (matches!(
        timeline.chain.reason.as_deref(),
        Some(
            "awaitingRecoveryKeyframe"
                | "awaitRecoveryKeyframe"
                | "referenceChainUnrecoverable"
                | "transportAwaitRecoveryKeyframe"
                | "streamThinStall"
                | "gapRepairInFlight"
        )
    ) || matches!(
        timeline.source_event.as_str(),
        "frame-await-recovery-keyframe"
            | "gap-repair-in-flight"
            | "gap-reorder-pending"
            | "nack-observation"
            | "timeout-stream-thin-stall"
    ))
}
