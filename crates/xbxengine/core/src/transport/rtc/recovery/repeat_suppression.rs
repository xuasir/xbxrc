use std::sync::Mutex;

use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::recovery::escalation::{RecoveryAction, VideoEscalationReason};
use crate::transport::rtc::recovery::runtime_state::{has_fresh_media_output, unix_now_ms};
use crate::{
    XbxEngineAnchorCandidateState, XbxEngineMediaRuntimeStats, XbxEngineVideoTimelineObservation,
};

const WAIT_KEYFRAME_REPEAT_SUPPRESS_MS: f64 = 260.0;
const WAIT_KEYFRAME_DECODER_RESET_REPEAT_SUPPRESS_MS: f64 = 620.0;
const IDLE_TIMEOUT_REPEAT_SUPPRESS_MS: f64 = 360.0;
const TRANSPORT_AWAIT_DEBT_FRESH_MS: f64 = 900.0;

pub(crate) fn resolve_recent_repeat_suppression(
    runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
    reason: &VideoEscalationReason,
) -> Option<RecoveryAction> {
    let now_ms = unix_now_ms();
    let (
        escalation,
        has_new_transport_recovery_epoch,
        active_transport_await_debt,
        active_keyframe_inflight,
    ) = RuntimeStatsSink::read_shared(runtime_stats, |stats| {
        let escalation = stats.latest_video_escalation_observation.clone()?;
        let has_new_transport_recovery_epoch =
            stats.transport_recovery_epoch > stats.transport_recovery_epoch_at_last_escalation;
        let active_transport_await_debt = has_active_transport_await_debt(stats, now_ms);
        let active_keyframe_inflight = has_active_keyframe_inflight(stats, now_ms);
        Some((
            escalation,
            has_new_transport_recovery_epoch,
            active_transport_await_debt,
            active_keyframe_inflight,
        ))
    })
    .flatten()?;
    let elapsed_ms = now_ms - escalation.observed_at_ms;
    if elapsed_ms < 0.0 {
        return None;
    }

    let local_repair_reason = matches!(
        reason,
        VideoEscalationReason::DisplaySupplyCritical
            | VideoEscalationReason::AdapterIdleTimeout
            | VideoEscalationReason::AdapterThinStream
            | VideoEscalationReason::Reconfigure
            | VideoEscalationReason::DecoderBackendFailure
    );

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
            let active_recovery_action = coalesced_action_for_existing_family(
                escalation.action.as_str(),
                active_keyframe_inflight,
            );
            let decoder_reset_inflight = matches!(
                active_recovery_action,
                Some(RecoveryAction::CoalescedDecoderResetInFlight)
            );
            if same_wait_keyframe_chain
                && decoder_reset_inflight
                && elapsed_ms <= WAIT_KEYFRAME_DECODER_RESET_REPEAT_SUPPRESS_MS
            {
                return Some(RecoveryAction::CoalescedDecoderResetInFlight);
            }
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
                coalesced_action_for_existing_family(escalation.action.as_str(), false),
                Some(RecoveryAction::CoalescedDecoderResetInFlight)
            );
            if same_local_display_chain
                && decoder_reset_inflight
                && elapsed_ms <= IDLE_TIMEOUT_REPEAT_SUPPRESS_MS
            {
                return Some(RecoveryAction::CoalescedDecoderResetInFlight);
            }
        }
        VideoEscalationReason::AdapterIdleTimeout => {
            let same_idle_chain = escalation.reason == "adapterIdleTimeout";
            let decoder_reset_inflight = matches!(
                coalesced_action_for_existing_family(escalation.action.as_str(), false),
                Some(RecoveryAction::CoalescedDecoderResetInFlight)
            );
            if same_idle_chain
                && decoder_reset_inflight
                && elapsed_ms <= IDLE_TIMEOUT_REPEAT_SUPPRESS_MS
            {
                return Some(RecoveryAction::CoalescedDecoderResetInFlight);
            }
        }
        VideoEscalationReason::AdapterThinStream => {
            let same_thin_stream_chain = matches!(
                escalation.reason.as_str(),
                "displaySupplyDegraded" | "adapterThinStream"
            );
            let active_action = coalesced_action_for_existing_family(
                escalation.action.as_str(),
                active_keyframe_inflight,
            );
            if same_thin_stream_chain
                && active_action.is_some()
                && elapsed_ms <= IDLE_TIMEOUT_REPEAT_SUPPRESS_MS
            {
                return active_action;
            }
        }
        _ => {}
    }

    if local_repair_reason
        && matches!(
            coalesced_action_for_existing_family(
                escalation.action.as_str(),
                active_keyframe_inflight
            ),
            Some(RecoveryAction::CoalescedDecoderResetInFlight)
        )
        && elapsed_ms <= IDLE_TIMEOUT_REPEAT_SUPPRESS_MS
    {
        return Some(RecoveryAction::CoalescedDecoderResetInFlight);
    }

    None
}

fn coalesced_action_for_existing_family(
    action: &str,
    active_keyframe_inflight: bool,
) -> Option<RecoveryAction> {
    if matches!(
        action,
        "requestDecoderReset"
            | "requestKeyframe+decoderReset"
            | "requestKeyframe+decoderReset(startupLowQualityRetry)"
    ) {
        return Some(RecoveryAction::CoalescedDecoderResetInFlight);
    }
    if action.starts_with("requestKeyframe") && active_keyframe_inflight {
        return Some(RecoveryAction::CoalescedKeyframeInFlight);
    }
    None
}

fn has_active_keyframe_inflight(stats: &XbxEngineMediaRuntimeStats, now_ms: f64) -> bool {
    if has_recent_keyframe_rtcp_unavailable(stats, now_ms) {
        return false;
    }
    stats
        .latest_keyframe_request_episode
        .as_ref()
        .is_some_and(|episode| {
            matches!(episode.response_verdict.as_deref(), None | Some("pending"))
                && matches!(episode.status.as_str(), "requested" | "sent")
                && episode.sent_at_ms.is_some()
                && episode.deadline_at_ms.unwrap_or(now_ms + 1.0) >= now_ms
        })
}

fn has_recent_keyframe_rtcp_unavailable(stats: &XbxEngineMediaRuntimeStats, now_ms: f64) -> bool {
    stats
        .recent_recovery_decision_ledgers
        .iter()
        .rev()
        .any(|ledger| {
            (now_ms - ledger.observed_at_ms).max(0.0) <= TRANSPORT_AWAIT_DEBT_FRESH_MS
                && ledger.action_selected.starts_with("requestKeyframe")
                && ledger.command_result.as_deref() != Some("succeeded")
                && ledger.command_detail.as_deref().is_some_and(|detail| {
                    detail.contains("xbxEngineRtcVideoRtcpFeedbackTargetUnavailable")
                })
        })
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
                | "gapRepairInFlight"
        )
    ) || matches!(
        timeline.source_event.as_str(),
        "frame-await-recovery-keyframe"
            | "gap-repair-in-flight"
            | "gap-reorder-pending"
            | "nack-observation"
    ))
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::resolve_recent_repeat_suppression;
    use crate::transport::rtc::recovery::escalation::{RecoveryAction, VideoEscalationReason};
    use crate::transport::rtc::recovery::runtime_state::unix_now_ms;
    use crate::{XbxEngineMediaRuntimeStats, XbxEngineVideoEscalationObservation};

    #[test]
    fn local_idle_timeout_repeat_suppression_ignores_transport_epoch_rotation() {
        let now_ms = unix_now_ms();
        let stats = XbxEngineMediaRuntimeStats {
            transport_recovery_epoch: 9,
            transport_recovery_epoch_at_last_escalation: 8,
            latest_video_escalation_observation: Some(XbxEngineVideoEscalationObservation {
                observation_id: 1,
                reason: "adapterIdleTimeout".to_string(),
                action: "requestDecoderReset".to_string(),
                recovery_stage: "degraded-serving".to_string(),
                recovery_chain_value: "health".to_string(),
                recovery_failure_cost: "high".to_string(),
                recovery_window_source: "decoder-reset-window".to_string(),
                observed_at_ms: now_ms,
            }),
            ..XbxEngineMediaRuntimeStats::default()
        };

        let action = resolve_recent_repeat_suppression(
            &Mutex::new(stats),
            &VideoEscalationReason::AdapterIdleTimeout,
        );

        assert_eq!(action, Some(RecoveryAction::CoalescedDecoderResetInFlight));
    }

    #[test]
    fn local_thin_stream_repeat_suppression_coalesces_keyframe_chain_without_transport_epoch_dependency(
    ) {
        let now_ms = unix_now_ms();
        let stats = XbxEngineMediaRuntimeStats {
            transport_recovery_epoch: 21,
            transport_recovery_epoch_at_last_escalation: 20,
            latest_video_escalation_observation: Some(XbxEngineVideoEscalationObservation {
                observation_id: 2,
                reason: "displaySupplyDegraded".to_string(),
                action: "requestKeyframe".to_string(),
                recovery_stage: "degraded-serving".to_string(),
                recovery_chain_value: "health".to_string(),
                recovery_failure_cost: "medium".to_string(),
                recovery_window_source: "supply-repair-window".to_string(),
                observed_at_ms: now_ms,
            }),
            latest_keyframe_request_episode: Some(
                crate::XbxEngineKeyframeRequestEpisodeObservation {
                    episode_id: 1,
                    request_reason: Some("displaySupplyDegraded".to_string()),
                    request_kind: Some("pli".to_string()),
                    status: "sent".to_string(),
                    requested_at_ms: now_ms,
                    sent_at_ms: Some(now_ms),
                    deadline_at_ms: Some(now_ms + 120.0),
                    first_keyframe_packet_at_ms: None,
                    first_keyframe_decoded_at_ms: None,
                    response_rtp_timestamp: None,
                    response_frame_seq: None,
                    response_verdict: Some("pending".to_string()),
                },
            ),
            ..XbxEngineMediaRuntimeStats::default()
        };

        let action = resolve_recent_repeat_suppression(
            &Mutex::new(stats),
            &VideoEscalationReason::AdapterThinStream,
        );

        assert_eq!(action, Some(RecoveryAction::CoalescedKeyframeInFlight));
    }
}
