use crate::media::video::h264::inspection::H264AccessUnitInspection;
use crate::transport::rtc::session::facts::recovery_episode::{
    recovery_episode_stage_from_status, RecoveryEpisodeStage,
};
use crate::{
    XbxEngineH264InspectionObservation,
    XbxEngineKeyframeRequestEpisodeObservation as XbxEnginePictureRecoveryEpisodeObservation,
    XbxEngineMediaRuntimeStats, XbxEngineVideoTimelineObservation,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CoalescingMode {
    Merge,
    Refresh,
}

impl CoalescingMode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Merge => "Merge",
            Self::Refresh => "Refresh",
        }
    }
}

const TRANSPORT_AWAIT_UNRESOLVED_REASONS: [&str; 4] = [
    "receiverWaitingKeyframe",
    "awaitingRecoveryAnchor",
    "awaitRecoveryAnchor",
    "referenceChainUnrecoverable",
];

pub(crate) fn is_transport_await_unresolved_reason(reason: &str) -> bool {
    TRANSPORT_AWAIT_UNRESOLVED_REASONS.contains(&reason)
}

pub(crate) fn is_transport_await_probe_source_event(source_event: Option<&str>) -> bool {
    matches!(
        source_event,
        Some(
            "frame-await-recovery-anchor"
                | "frame-inspection-rejected-await-anchor"
                | "frame-inspection-rejected-trigger-recovery-anchor"
        )
    )
}

pub(crate) fn is_invalid_recovery_bootstrap_reject_reason(reason: Option<&str>) -> bool {
    matches!(
        reason,
        Some("bootstrapMissingSps" | "bootstrapMissingPps" | "inspectionRejectInvalidSliceHeader")
    )
}

pub(crate) fn is_recovery_delta_continuation_ready(inspection: &H264AccessUnitInspection) -> bool {
    inspection.slice_headers_valid
        && inspection.delta_continuation_ready()
        && inspection.committed_sps_present()
        && inspection.committed_pps_present()
}

pub(crate) fn inspection_has_invalid_recovery_bootstrap(
    inspection: &XbxEngineH264InspectionObservation,
) -> bool {
    !inspection.bootstrap_ready
        && is_invalid_recovery_bootstrap_reject_reason(
            inspection.bootstrap_reject_reason.as_deref(),
        )
}

const CURRENT_TRANSPORT_AWAIT_INVALID_BOOTSTRAP_FRESH_MS: f64 = 220.0;

pub(crate) fn is_terminal_transport_await_deferred_episode(
    episode: &XbxEnginePictureRecoveryEpisodeObservation,
    inspection: Option<&XbxEngineH264InspectionObservation>,
    has_clean_anchor_evidence: bool,
    now_ms: f64,
    fresh_window_ms: f64,
) -> bool {
    if !matches!(
        episode.request_reason.as_deref(),
        Some("receiverWaitingKeyframe")
    ) {
        return false;
    }
    let stage = recovery_episode_stage_from_status(episode.status.as_str());
    if !matches!(
        stage,
        Some(RecoveryEpisodeStage::Deferred | RecoveryEpisodeStage::Expired)
    ) {
        return false;
    }
    if episode.response_verdict.as_deref() != Some("transportDeferred") {
        return false;
    }
    if episode.sent_at_ms.is_some() || !has_clean_anchor_evidence {
        return false;
    }
    let Some(inspection) = inspection else {
        return false;
    };
    if (now_ms - inspection.observed_at_ms).max(0.0) > fresh_window_ms {
        return false;
    }
    if !inspection_has_invalid_recovery_bootstrap(inspection) {
        return false;
    }
    match (
        episode.response_rtp_timestamp,
        inspection.frame_rtp_timestamp,
    ) {
        (Some(response_ts), Some(frame_ts)) => frame_ts == response_ts,
        _ => inspection.observed_at_ms >= episode.requested_at_ms,
    }
}

pub(crate) fn is_receiver_state_receiving(receiver_state: Option<&str>) -> bool {
    matches!(receiver_state, Some("receiving"))
}

/// 与 RFC 四态一致：优先 `receiver_observation`，其次 timeline `chain.state`。
pub(crate) fn is_timeline_chain_receiving_from_stats(stats: &XbxEngineMediaRuntimeStats) -> bool {
    if stats
        .latest_video_receiver_observation
        .as_ref()
        .is_some_and(|obs| is_receiver_state_receiving(Some(obs.receiver_state.as_str())))
    {
        return true;
    }
    stats
        .latest_video_timeline_observation
        .as_ref()
        .is_some_and(|timeline| matches!(timeline.chain.state.as_str(), "receiving"))
}

pub(crate) fn is_receiver_state_waiting_keyframe(receiver_state: Option<&str>) -> bool {
    matches!(receiver_state, Some("waiting-keyframe"))
}

pub(crate) fn is_receiver_state_repairing(receiver_state: Option<&str>) -> bool {
    matches!(receiver_state, Some("repairing"))
}

pub(crate) fn is_ingress_waiting_keyframe(
    receiver_state: Option<&str>,
    chain_state: Option<&str>,
    chain_reason: Option<&str>,
    source_event: Option<&str>,
) -> bool {
    if is_receiver_state_receiving(receiver_state) || is_receiver_state_repairing(receiver_state) {
        return false;
    }
    if is_receiver_state_waiting_keyframe(receiver_state) {
        return true;
    }
    if matches!(chain_state, Some("receiving" | "priming")) {
        return false;
    }
    let probe_event_waiting = is_transport_await_probe_source_event(source_event)
        && !matches!(chain_state, Some("receiving" | "priming"));
    matches!(chain_state, Some("waiting-keyframe"))
        || chain_reason.is_some_and(is_transport_await_unresolved_reason)
        || probe_event_waiting
}

pub(crate) fn has_unresolved_transport_await_issue_from_observation(
    timeline: &XbxEngineVideoTimelineObservation,
) -> bool {
    if matches!(timeline.chain.state.as_str(), "receiving" | "priming") {
        return false;
    }
    if timeline
        .chain
        .reason
        .as_deref()
        .is_some_and(is_transport_await_unresolved_reason)
    {
        return true;
    }
    if timeline
        .frame
        .as_ref()
        .and_then(|frame| frame.close_reason.as_deref())
        .is_some_and(is_transport_await_unresolved_reason)
    {
        return true;
    }
    timeline.gap.as_ref().is_some_and(|gap| {
        !matches!(gap.state.as_str(), "resolved" | "expired")
            && timeline
                .chain
                .reason
                .as_deref()
                .is_some_and(is_transport_await_unresolved_reason)
    })
}

pub(crate) fn current_clean_anchor_observed_at_ms(
    clean_anchor_epoch: Option<u64>,
    clean_anchor_observed_at_ms: Option<f64>,
    clean_anchor_source_event: Option<&str>,
    recovery_epoch: u64,
) -> Option<f64> {
    if clean_anchor_epoch == Some(recovery_epoch)
        && clean_anchor_source_event == Some("displayed-idr")
    {
        clean_anchor_observed_at_ms
    } else {
        None
    }
}

pub(crate) fn current_clean_anchor_bridge_observed_at_ms(
    bridge_epoch: Option<u64>,
    bridge_observed_at_ms: Option<f64>,
    bridge_source_event: Option<&str>,
    recovery_epoch: u64,
) -> Option<f64> {
    if bridge_epoch == Some(recovery_epoch)
        && bridge_source_event == Some("hostVisibleAnchorPending")
    {
        bridge_observed_at_ms
    } else {
        None
    }
}

const TRANSPORT_AWAIT_HARD_BOOTSTRAP_FRESH_MS: f64 = 1_500.0;

/// transport-await 硬证据：仅 receiver/inspection/display 事实，不读 keyframe episode terminal。
pub(crate) fn transport_await_has_hard_bootstrap_evidence_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> bool {
    let Some(inspection) = stats
        .latest_h264_inspection_observation
        .as_ref()
        .filter(|inspection| {
            (now_ms - inspection.observed_at_ms).max(0.0) <= TRANSPORT_AWAIT_HARD_BOOTSTRAP_FRESH_MS
        })
    else {
        return false;
    };
    if inspection_has_invalid_recovery_bootstrap(inspection) {
        return true;
    }
    if !inspection.bootstrap_ready {
        return match inspection.bootstrap_reject_reason.as_deref() {
            Some("bootstrapMissingSps" | "bootstrapMissingPps" | "bootstrapInvalidSliceHeader") => {
                true
            }
            Some("bootstrapMissingIdr" | "NonIdrVcl") => {
                !(stats.recovery_displayed_idr_at_ms.is_some()
                    || (stats.recovery_pending_displayed_idr_rtp.is_some()
                        && stats.host_frame_present_epoch > 0))
            }
            _ => false,
        };
    }
    false
}

pub(crate) fn has_current_transport_await_issue_from_observation(
    timeline: &XbxEngineVideoTimelineObservation,
    current_clean_anchor_observed_at_ms: Option<f64>,
) -> bool {
    has_unresolved_transport_await_issue_from_observation(timeline)
        && current_clean_anchor_observed_at_ms
            .is_none_or(|clean_anchor_at_ms| timeline.observed_at_ms > clean_anchor_at_ms)
}

pub(crate) fn has_current_transport_await_issue_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
) -> bool {
    let current_clean_anchor_observed_at_ms = current_clean_anchor_observed_at_ms(
        stats.video_anchor_clean_epoch,
        stats.video_anchor_clean_observed_at_ms,
        stats.video_anchor_clean_source_event.as_deref(),
        stats.transport_recovery_epoch,
    );
    let Some(timeline) = stats.latest_video_timeline_observation.as_ref() else {
        return false;
    };
    if has_current_transport_await_issue_from_observation(
        timeline,
        current_clean_anchor_observed_at_ms,
    ) {
        return true;
    }
    let receiver_receiving = stats
        .latest_video_receiver_observation
        .as_ref()
        .is_some_and(|obs| is_receiver_state_receiving(Some(obs.receiver_state.as_str())));
    if !receiver_receiving {
        return false;
    }
    let Some(clean_anchor_at_ms) = current_clean_anchor_observed_at_ms else {
        return false;
    };
    stats
        .latest_h264_inspection_observation
        .as_ref()
        .is_some_and(|inspection| {
            inspection.observed_at_ms >= clean_anchor_at_ms
                && (timeline.observed_at_ms - inspection.observed_at_ms).max(0.0)
                    <= CURRENT_TRANSPORT_AWAIT_INVALID_BOOTSTRAP_FRESH_MS
                && inspection_has_invalid_recovery_bootstrap(inspection)
        })
}
