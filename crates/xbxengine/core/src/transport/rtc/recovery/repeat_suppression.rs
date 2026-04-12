use std::sync::Mutex;

use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::recovery::contract::has_current_clean_anchor_from_stats;
use crate::transport::rtc::recovery::escalation::{RecoveryAction, VideoEscalationReason};
use crate::transport::rtc::recovery::keyframe_lifecycle::{
    derive_keyframe_lifecycle_phase, KeyframeRequestLifecyclePhase,
};
use crate::transport::rtc::recovery::runtime_state::{has_fresh_media_output, unix_now_ms};
use crate::{
    XbxEngineAnchorCandidateState, XbxEngineMediaRuntimeStats, XbxEngineVideoTimelineObservation,
};

const WAIT_KEYFRAME_REPEAT_SUPPRESS_MS: f64 = 260.0;
const WAIT_KEYFRAME_DECODER_RESET_REPEAT_SUPPRESS_MS: f64 = 620.0;
const IDLE_TIMEOUT_REPEAT_SUPPRESS_MS: f64 = 360.0;
const TRANSPORT_AWAIT_DEBT_FRESH_MS: f64 = 900.0;
const DECODER_RESET_PROGRESS_HOLD_MS: f64 = 900.0;
const INVALID_KEYFRAME_RESPONSE_GRACE_MS: f64 = 220.0;
const INVALID_KEYFRAME_RESPONSE_FRESH_MS: f64 = 1_500.0;
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
        active_decoder_reset_family,
    ) = RuntimeStatsSink::read_shared(runtime_stats, |stats| {
        let escalation = stats.latest_video_escalation_observation.clone()?;
        let has_new_transport_recovery_epoch =
            stats.transport_recovery_epoch > stats.transport_recovery_epoch_at_last_escalation;
        let active_transport_await_debt = has_active_transport_await_debt(stats, now_ms);
        let active_keyframe_inflight = has_active_keyframe_inflight(stats, now_ms);
        let active_decoder_reset_family = has_active_decoder_reset_family(stats, now_ms);
        Some((
            escalation,
            has_new_transport_recovery_epoch,
            active_transport_await_debt,
            active_keyframe_inflight,
            active_decoder_reset_family,
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
                active_decoder_reset_family,
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
            let same_local_display_chain = matches!(
                escalation.reason.as_str(),
                "displaySupplyCritical" | "hostPresentStalled"
            );
            let decoder_reset_inflight = matches!(
                coalesced_action_for_existing_family(
                    escalation.action.as_str(),
                    false,
                    active_decoder_reset_family,
                ),
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
                coalesced_action_for_existing_family(
                    escalation.action.as_str(),
                    false,
                    active_decoder_reset_family,
                ),
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
                active_decoder_reset_family,
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
                active_keyframe_inflight,
                active_decoder_reset_family,
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
    active_decoder_reset_family: bool,
) -> Option<RecoveryAction> {
    if matches!(
        action,
        "requestDecoderReset"
            | "requestKeyframe+decoderReset"
            | "requestKeyframe+decoderReset(startupLowQualityRetry)"
    ) && active_decoder_reset_family
    {
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
    let Some(episode) = stats.latest_keyframe_request_episode.as_ref() else {
        return false;
    };
    let phase = derive_keyframe_lifecycle_phase(
        stats.transport_recovery_epoch,
        stats.video_anchor_clean_epoch,
        stats.video_anchor_clean_observed_at_ms,
        episode,
    );
    match phase {
        KeyframeRequestLifecyclePhase::Success
        | KeyframeRequestLifecyclePhase::Failure
        | KeyframeRequestLifecyclePhase::Decoded
        | KeyframeRequestLifecyclePhase::Requesting => false,
        KeyframeRequestLifecyclePhase::Sent | KeyframeRequestLifecyclePhase::PacketSeen => {
            if episode.sent_at_ms.is_none() {
                return false;
            }
            if episode
                .deadline_at_ms
                .is_some_and(|deadline| deadline < now_ms)
            {
                return false;
            }
            if has_transport_await_invalid_keyframe_response(
                stats,
                episode.sent_at_ms.unwrap_or(episode.requested_at_ms),
                now_ms,
            ) {
                return false;
            }
            !matches!(
                episode.response_verdict.as_deref(),
                Some("transportDeferred" | "transportFailed" | "missed")
            )
        }
    }
}

fn has_active_decoder_reset_family(stats: &XbxEngineMediaRuntimeStats, now_ms: f64) -> bool {
    let Some(attempt_at_ms) = latest_decoder_reset_family_attempt_ms(stats) else {
        return false;
    };
    if (now_ms - attempt_at_ms).max(0.0) > DECODER_RESET_PROGRESS_HOLD_MS {
        return false;
    }
    if has_transport_await_invalid_keyframe_response(stats, attempt_at_ms, now_ms) {
        return false;
    }
    !has_post_decoder_reset_progress(stats, attempt_at_ms)
}

fn latest_decoder_reset_family_attempt_ms(stats: &XbxEngineMediaRuntimeStats) -> Option<f64> {
    let action_attempt_ms =
        stats
            .latest_video_escalation_observation
            .as_ref()
            .and_then(|observation| {
                matches!(
                    observation.action.as_str(),
                    "requestDecoderReset"
                        | "requestKeyframe+decoderReset"
                        | "requestKeyframe+decoderReset(startupLowQualityRetry)"
                )
                .then_some(observation.observed_at_ms)
            });
    match (stats.latest_video_decoder_reset_time_ms, action_attempt_ms) {
        (Some(reset_at_ms), Some(action_at_ms)) => Some(reset_at_ms.max(action_at_ms)),
        (Some(reset_at_ms), None) => Some(reset_at_ms),
        (None, Some(action_at_ms)) => Some(action_at_ms),
        (None, None) => None,
    }
}

fn has_post_decoder_reset_progress(stats: &XbxEngineMediaRuntimeStats, attempt_at_ms: f64) -> bool {
    stats
        .latest_video_decode_ok_time_ms
        .is_some_and(|at_ms| at_ms > attempt_at_ms)
        || stats
            .latest_video_host_present_time_ms
            .is_some_and(|at_ms| at_ms > attempt_at_ms)
        || stats
            .video_anchor_clean_observed_at_ms
            .is_some_and(|at_ms| at_ms > attempt_at_ms)
}

fn has_transport_await_invalid_keyframe_response(
    stats: &XbxEngineMediaRuntimeStats,
    attempt_at_ms: f64,
    now_ms: f64,
) -> bool {
    let packet_seen_without_decode =
        stats
            .latest_keyframe_request_episode
            .as_ref()
            .is_some_and(|episode| {
                episode.request_reason.as_deref() == Some("transportAwaitRecoveryKeyframe")
                    && episode.status == "packet-seen"
                    && episode.first_keyframe_decoded_at_ms.is_none()
                    && episode
                        .first_keyframe_packet_at_ms
                        .is_some_and(|packet_at_ms| {
                            packet_at_ms >= attempt_at_ms
                                && (now_ms - packet_at_ms).max(0.0)
                                    >= INVALID_KEYFRAME_RESPONSE_GRACE_MS
                        })
            });
    if packet_seen_without_decode && has_active_transport_await_debt(stats, now_ms) {
        return true;
    }
    stats
        .latest_h264_inspection_observation
        .as_ref()
        .is_some_and(|inspection| {
            inspection.observed_at_ms >= attempt_at_ms
                && (now_ms - inspection.observed_at_ms).max(0.0)
                    <= INVALID_KEYFRAME_RESPONSE_FRESH_MS
                && !inspection.bootstrap_ready
                && matches!(
                    inspection.bootstrap_reject_reason.as_deref(),
                    Some(
                        "NonIdrVcl"
                            | "bootstrapMissingSps"
                            | "bootstrapMissingPps"
                            | "inspectionRejectInvalidSliceHeader"
                    )
                )
                && has_active_transport_await_debt(stats, now_ms)
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
    let current_clean_anchor = has_current_clean_anchor_from_stats(stats);
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

    use super::{resolve_recent_repeat_suppression, DECODER_RESET_PROGRESS_HOLD_MS};
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
    fn host_present_stalled_coalesces_with_display_supply_critical_decoder_reset_suppression() {
        let now_ms = unix_now_ms();
        let stats = XbxEngineMediaRuntimeStats {
            latest_video_escalation_observation: Some(XbxEngineVideoEscalationObservation {
                observation_id: 42,
                reason: "hostPresentStalled".to_string(),
                action: "requestDecoderReset".to_string(),
                recovery_stage: "degraded-serving".to_string(),
                recovery_chain_value: "health".to_string(),
                recovery_failure_cost: "high".to_string(),
                recovery_window_source: "decoder-reset-window".to_string(),
                observed_at_ms: now_ms,
            }),
            latest_video_decoder_reset_time_ms: Some(now_ms),
            ..XbxEngineMediaRuntimeStats::default()
        };

        let action = resolve_recent_repeat_suppression(
            &Mutex::new(stats),
            &VideoEscalationReason::DisplaySupplyCritical,
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
                    status_detail: None,
                    requested_at_ms: now_ms,
                    sent_at_ms: Some(now_ms),
                    deadline_at_ms: Some(now_ms + 120.0),
                    transport_detail: None,
                    first_video_packet_at_ms: None,
                    first_video_packet_rtp_timestamp: None,
                    first_video_packet_is_keyframe: None,
                    first_keyframe_packet_at_ms: None,
                    first_keyframe_decoded_at_ms: None,
                    response_rtp_timestamp: None,
                    response_frame_seq: None,
                    response_verdict: Some("pending".to_string()),
                    lifecycle_phase: None,
                    retired_at_ms: None,
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

    #[test]
    fn local_idle_timeout_does_not_coalesce_stale_decoder_reset_family_without_progress() {
        let now_ms = unix_now_ms();
        let stats = XbxEngineMediaRuntimeStats {
            latest_video_escalation_observation: Some(XbxEngineVideoEscalationObservation {
                observation_id: 3,
                reason: "adapterIdleTimeout".to_string(),
                action: "requestDecoderReset".to_string(),
                recovery_stage: "degraded-serving".to_string(),
                recovery_chain_value: "health".to_string(),
                recovery_failure_cost: "high".to_string(),
                recovery_window_source: "decoder-reset-window".to_string(),
                observed_at_ms: now_ms - (DECODER_RESET_PROGRESS_HOLD_MS + 20.0),
            }),
            latest_video_decoder_reset_time_ms: Some(
                now_ms - (DECODER_RESET_PROGRESS_HOLD_MS + 20.0),
            ),
            ..XbxEngineMediaRuntimeStats::default()
        };

        let action = resolve_recent_repeat_suppression(
            &Mutex::new(stats),
            &VideoEscalationReason::AdapterIdleTimeout,
        );

        assert_ne!(action, Some(RecoveryAction::CoalescedDecoderResetInFlight));
    }

    #[test]
    fn invalid_transport_await_keyframe_response_does_not_keep_decoder_reset_family_inflight() {
        let now_ms = unix_now_ms();
        let stats = XbxEngineMediaRuntimeStats {
            transport_recovery_epoch: 22,
            transport_recovery_epoch_at_last_escalation: 22,
            latest_video_escalation_observation: Some(XbxEngineVideoEscalationObservation {
                observation_id: 4,
                reason: "transportAwaitRecoveryKeyframe".to_string(),
                action: "requestDecoderReset".to_string(),
                recovery_stage: "rebuilding-supply".to_string(),
                recovery_chain_value: "anchor".to_string(),
                recovery_failure_cost: "high".to_string(),
                recovery_window_source: "transport-await-window".to_string(),
                observed_at_ms: now_ms - 240.0,
            }),
            latest_video_decoder_reset_time_ms: Some(now_ms - 235.0),
            latest_video_timeline_observation: Some(crate::XbxEngineVideoTimelineObservation {
                observation_id: 5,
                source_event: "frame-await-recovery-keyframe".to_string(),
                gap: None,
                frame: None,
                chain: crate::XbxEngineVideoTimelineChainSnapshot {
                    state: "recovering".to_string(),
                    reason: Some("awaitingRecoveryKeyframe".to_string()),
                    chain_break_evidence: None,

                    observed_at_ms: now_ms - 20.0,
                },
                observed_at_ms: now_ms - 20.0,
            }),
            latest_keyframe_request_episode: Some(
                crate::XbxEngineKeyframeRequestEpisodeObservation {
                    episode_id: 4,
                    request_reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                    request_kind: Some("pli".to_string()),
                    status: "packet-seen".to_string(),
                    status_detail: None,
                    requested_at_ms: now_ms - 260.0,
                    sent_at_ms: Some(now_ms - 250.0),
                    deadline_at_ms: Some(now_ms + 200.0),
                    transport_detail: None,
                    first_video_packet_at_ms: None,
                    first_video_packet_rtp_timestamp: None,
                    first_video_packet_is_keyframe: None,
                    first_keyframe_packet_at_ms: Some(now_ms - 230.0),
                    first_keyframe_decoded_at_ms: None,
                    response_rtp_timestamp: Some(4_444),
                    response_frame_seq: None,
                    response_verdict: Some("on-time".to_string()),
                    lifecycle_phase: None,
                    retired_at_ms: None,
                },
            ),
            latest_h264_inspection_observation: Some(crate::XbxEngineH264InspectionObservation {
                observation_id: 6,
                frame_rtp_timestamp: Some(4_444),
                nal_types: vec!["SliceLayerWithoutPartitioningNonIdr".to_string()],
                nal_count: 1,
                vcl_nal_count: 1,
                has_inband_sps: false,
                has_inband_pps: false,
                committed_sps_present: true,
                committed_pps_present: true,
                slice_headers_valid: true,
                delta_continuation_ready: false,
                parameter_sets_changed: false,
                config_changed: false,
                is_idr: false,
                sample_width: None,
                sample_height: None,
                bootstrap_ready: false,
                bootstrap_reject_reason: Some("NonIdrVcl".to_string()),
                admission_accepted: true,
                observed_at_ms: now_ms - 225.0,

                ..Default::default()
            }),
            ..XbxEngineMediaRuntimeStats::default()
        };

        let action = resolve_recent_repeat_suppression(
            &Mutex::new(stats),
            &VideoEscalationReason::TransportAwaitRecoveryKeyframe,
        );

        assert_ne!(action, Some(RecoveryAction::CoalescedDecoderResetInFlight));
    }
}
