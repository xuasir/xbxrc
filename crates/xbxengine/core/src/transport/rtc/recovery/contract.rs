use crate::media::video::types::FrameValue as MediaFrameValue;
use crate::{
    XbxEngineH264InspectionObservation, XbxEngineKeyframeRequestEpisodeObservation,
    XbxEngineVideoTimelineObservation,
};

/// 恢复系统的统一“事实模型”合同。
///
/// 注意：这里的枚举值是跨层（source/coordinator/session/owner/stats/trace）共享的单一事实源，
/// 不允许在其他模块并行定义同名语义。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FrameValue {
    Disposable,
    Continuity,
    Reference,
    RecoveryAnchor,
    CleanAnchor,
}

impl FrameValue {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Disposable => "Disposable",
            Self::Continuity => "Continuity",
            Self::Reference => "Reference",
            Self::RecoveryAnchor => "RecoveryAnchor",
            Self::CleanAnchor => "CleanAnchor",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GapSeverity {
    MinorGap,
    ReferenceGap,
    AnchorGap,
    ChainBroken,
    RecoveryBlocked,
}

impl GapSeverity {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::MinorGap => "MinorGap",
            Self::ReferenceGap => "ReferenceGap",
            Self::AnchorGap => "AnchorGap",
            Self::ChainBroken => "ChainBroken",
            Self::RecoveryBlocked => "RecoveryBlocked",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RecoveryEpisodeStage {
    Requested,
    Sent,
    ResponseObserved,
    Decoded,
    CleanAnchorCommitted,
    Deferred,
    Stalled,
    Expired,
}

impl RecoveryEpisodeStage {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Requested => "Requested",
            Self::Sent => "Sent",
            Self::ResponseObserved => "ResponseObserved",
            Self::Decoded => "Decoded",
            Self::CleanAnchorCommitted => "CleanAnchorCommitted",
            Self::Deferred => "Deferred",
            Self::Stalled => "Stalled",
            Self::Expired => "Expired",
        }
    }
}

pub(crate) fn recovery_episode_stage_from_status(status: &str) -> Option<RecoveryEpisodeStage> {
    match status {
        "requested" => Some(RecoveryEpisodeStage::Requested),
        "sent" => Some(RecoveryEpisodeStage::Sent),
        "response-observed" | "packet-seen" => Some(RecoveryEpisodeStage::ResponseObserved),
        "decoded" => Some(RecoveryEpisodeStage::Decoded),
        "deferred" => Some(RecoveryEpisodeStage::Deferred),
        "expired-unsent" | "missed" => Some(RecoveryEpisodeStage::Expired),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CoalescingMode {
    Merge,
    Refresh,
    Preempt,
}

impl CoalescingMode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Merge => "Merge",
            Self::Refresh => "Refresh",
            Self::Preempt => "Preempt",
        }
    }
}

const TRANSPORT_AWAIT_UNRESOLVED_REASONS: [&str; 4] = [
    "transportAwaitRecoveryKeyframe",
    "awaitingRecoveryKeyframe",
    "awaitRecoveryKeyframe",
    "referenceChainUnrecoverable",
];

pub(crate) fn is_transport_await_unresolved_reason(reason: &str) -> bool {
    TRANSPORT_AWAIT_UNRESOLVED_REASONS.contains(&reason)
}

pub(crate) fn is_transport_await_probe_source_event(source_event: Option<&str>) -> bool {
    matches!(
        source_event,
        Some(
            "frame-await-recovery-keyframe"
                | "frame-inspection-rejected-await-keyframe"
                | "frame-inspection-rejected-trigger-recovery-keyframe"
        )
    )
}

pub(crate) fn is_invalid_recovery_bootstrap_reject_reason(reason: Option<&str>) -> bool {
    matches!(
        reason,
        Some(
            "NonIdrVcl"
                | "bootstrapMissingSps"
                | "bootstrapMissingPps"
                | "inspectionRejectInvalidSliceHeader"
        )
    )
}

pub(crate) fn inspection_has_invalid_recovery_bootstrap(
    inspection: &XbxEngineH264InspectionObservation,
) -> bool {
    !inspection.bootstrap_ready
        && is_invalid_recovery_bootstrap_reject_reason(
            inspection.bootstrap_reject_reason.as_deref(),
        )
}

pub(crate) fn is_terminal_transport_await_deferred_episode(
    episode: &XbxEngineKeyframeRequestEpisodeObservation,
    inspection: Option<&XbxEngineH264InspectionObservation>,
    has_clean_anchor_evidence: bool,
    now_ms: f64,
    fresh_window_ms: f64,
) -> bool {
    if episode.request_reason.as_deref() != Some("transportAwaitRecoveryKeyframe") {
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

fn is_recovery_sustaining_observation(
    chain_state: Option<&str>,
    chain_reason: Option<&str>,
) -> bool {
    matches!(chain_state, Some("sustaining-recovery"))
        || matches!(chain_reason, Some("recoverySustaining"))
}

pub(crate) fn is_ingress_waiting_keyframe(
    chain_state: Option<&str>,
    chain_reason: Option<&str>,
    source_event: Option<&str>,
) -> bool {
    if is_recovery_sustaining_observation(chain_state, chain_reason) {
        return false;
    }
    let probe_event_waiting = is_transport_await_probe_source_event(source_event)
        && !matches!(chain_state, Some("healthy"));
    matches!(chain_state, Some("broken" | "recovering"))
        || chain_reason.is_some_and(is_transport_await_unresolved_reason)
        || probe_event_waiting
}

pub(crate) fn has_unresolved_transport_await_issue_from_observation(
    timeline: &XbxEngineVideoTimelineObservation,
) -> bool {
    if is_recovery_sustaining_observation(
        Some(timeline.chain.state.as_str()),
        timeline.chain.reason.as_deref(),
    ) {
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
        && clean_anchor_source_event == Some("chain-clean-keyframe-submitted")
    {
        clean_anchor_observed_at_ms
    } else {
        None
    }
}

pub(crate) fn has_current_transport_await_issue_from_observation(
    timeline: &XbxEngineVideoTimelineObservation,
    current_clean_anchor_observed_at_ms: Option<f64>,
) -> bool {
    has_unresolved_transport_await_issue_from_observation(timeline)
        && current_clean_anchor_observed_at_ms
            .is_none_or(|clean_anchor_at_ms| timeline.observed_at_ms > clean_anchor_at_ms)
}

/// 从 timeline 观测推导统一 `GapSeverity`（不含 episode stalled→RecoveryBlocked，见
/// [`derive_gap_severity_with_episode_stall`]）。
pub(crate) fn derive_gap_severity_from_timeline_observation(
    timeline: &XbxEngineVideoTimelineObservation,
) -> GapSeverity {
    if is_recovery_sustaining_observation(
        Some(timeline.chain.state.as_str()),
        timeline.chain.reason.as_deref(),
    ) {
        return GapSeverity::MinorGap;
    }
    let reason = timeline.chain.reason.as_deref();
    if matches!(reason, Some("referenceChainUnrecoverable")) {
        return GapSeverity::ChainBroken;
    }
    if matches!(
        reason,
        Some(
            "awaitingRecoveryKeyframe" | "awaitRecoveryKeyframe" | "transportAwaitRecoveryKeyframe",
        )
    ) {
        return GapSeverity::AnchorGap;
    }
    if timeline.gap.is_some() {
        return GapSeverity::ReferenceGap;
    }
    GapSeverity::MinorGap
}

/// 与 keyframe episode stalled（无推进边沿）叠加时，将严重度提升为 `RecoveryBlocked`。
pub(crate) fn derive_gap_severity_with_episode_stall(
    timeline: &XbxEngineVideoTimelineObservation,
    episode_stalled_no_progress: bool,
) -> GapSeverity {
    if episode_stalled_no_progress {
        let reason = timeline.chain.reason.as_deref();
        if matches!(
            reason,
            Some(
                "awaitingRecoveryKeyframe"
                    | "awaitRecoveryKeyframe"
                    | "transportAwaitRecoveryKeyframe"
                    | "referenceChainUnrecoverable",
            )
        ) {
            return GapSeverity::RecoveryBlocked;
        }
    }
    derive_gap_severity_from_timeline_observation(timeline)
}

/// ledger / NACK / owner 共用的 `FrameValue` 映射（`RecoveryBlocked` 不映射帧价值，由调用方保留基线）。
pub(crate) fn frame_value_from_gap_severity(gs: GapSeverity) -> Option<FrameValue> {
    match gs {
        GapSeverity::RecoveryBlocked => None,
        GapSeverity::ChainBroken | GapSeverity::AnchorGap => Some(FrameValue::RecoveryAnchor),
        GapSeverity::ReferenceGap => Some(FrameValue::Reference),
        GapSeverity::MinorGap => Some(FrameValue::Continuity),
    }
}

/// 将恢复语义帧价值映射到媒体层 `FrameValue`（NACK/transport repair 预算）。
pub(crate) fn media_frame_value_from_recovery_semantics(
    fv: FrameValue,
    base_payload_size: usize,
) -> MediaFrameValue {
    match fv {
        FrameValue::Disposable | FrameValue::Continuity => {
            MediaFrameValue::new(false, false, base_payload_size.max(1))
        }
        FrameValue::Reference => MediaFrameValue::new(false, true, base_payload_size.max(1)),
        FrameValue::RecoveryAnchor | FrameValue::CleanAnchor => {
            MediaFrameValue::new(true, false, base_payload_size.max(1))
        }
    }
}

/// 非 Minor 的 gap 严重度视为需要 transport 恢复侧重点加压（coordinator 等）。
pub(crate) fn gap_severity_indicates_transport_recovery_pressure(gs: GapSeverity) -> bool {
    !matches!(gs, GapSeverity::MinorGap)
}

pub(crate) fn is_media_healthy_baseline(
    connected: bool,
    chain_healthy: bool,
    track_state: Option<&str>,
    track_video_bytes_total: Option<u64>,
    decode_age_ms: Option<f64>,
    present_age_ms: Option<f64>,
    decode_fresh_limit_ms: f64,
    present_fresh_limit_ms: f64,
    decoder_stalled: bool,
    renderer_stalled: bool,
) -> bool {
    if !connected || !chain_healthy || decoder_stalled || renderer_stalled {
        return false;
    }
    let track_attached = matches!(track_state, Some("remoteTrackAttached"));
    let has_video_bytes = track_video_bytes_total.is_some_and(|bytes| bytes > 0);
    let decode_fresh = decode_age_ms.is_some_and(|age| age <= decode_fresh_limit_ms);
    let present_fresh = present_age_ms.is_some_and(|age| age <= present_fresh_limit_ms);
    track_attached && has_video_bytes && decode_fresh && present_fresh
}
