use crate::media::video::h264::inspection::H264AccessUnitInspection;
use crate::{
    XbxEngineH264InspectionObservation,
    XbxEngineKeyframeRequestEpisodeObservation as XbxEnginePictureRecoveryEpisodeObservation,
    XbxEngineMediaRuntimeStats, XbxEngineVideoTimelineGapSnapshot,
    XbxEngineVideoTimelineObservation,
};

/// 恢复系统的统一“事实模型”合同。
///
/// 注意：这里的枚举值是跨层（source/coordinator/session/owner/stats/trace）共享的单一事实源，
/// 不允许在其他模块并行定义同名语义。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FrameValue {
    #[allow(dead_code)]
    Disposable,
    Continuity,
    Reference,
    RecoveryAnchor,
    #[allow(dead_code)]
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
    /// 低价值 / 无活跃可修补缺口的基线。
    LowValueGap,
    /// 有缺口但尚无 reference 级证据，不得直接进入恢复主线当 ReferenceGap。
    RepairableGap,
    ReferenceGap,
    AnchorGap,
    ChainBroken,
    RecoveryBlocked,
}

impl GapSeverity {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::LowValueGap => "LowValueGap",
            Self::RepairableGap => "RepairableGap",
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
    Deferred,
    Expired,
}

/// RFC: 恢复进度统一七级语义，作为跨层事实口径。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RecoveryProgressLevel {
    WaitingResponse,
    ContinuationSeen,
    AnchorSeen,
    Decoded,
    PlaybackRecovered,
    CleanAnchorCommitted,
    DisplayStable,
}

impl RecoveryProgressLevel {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::WaitingResponse => "WaitingResponse",
            Self::ContinuationSeen => "ContinuationSeen",
            Self::AnchorSeen => "AnchorSeen",
            Self::Decoded => "Decoded",
            Self::PlaybackRecovered => "PlaybackRecovered",
            Self::CleanAnchorCommitted => "CleanAnchorCommitted",
            Self::DisplayStable => "DisplayStable",
        }
    }
}

pub(crate) fn recovery_progress_level_from_str(value: &str) -> Option<RecoveryProgressLevel> {
    match value {
        "WaitingResponse" => Some(RecoveryProgressLevel::WaitingResponse),
        "ContinuationSeen" => Some(RecoveryProgressLevel::ContinuationSeen),
        "AnchorSeen" => Some(RecoveryProgressLevel::AnchorSeen),
        "Decoded" => Some(RecoveryProgressLevel::Decoded),
        "PlaybackRecovered" => Some(RecoveryProgressLevel::PlaybackRecovered),
        "CleanAnchorCommitted" => Some(RecoveryProgressLevel::CleanAnchorCommitted),
        "DisplayStable" => Some(RecoveryProgressLevel::DisplayStable),
        _ => None,
    }
}

impl RecoveryEpisodeStage {
    #[allow(dead_code)]
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Requested => "Requested",
            Self::Sent => "Sent",
            Self::ResponseObserved => "ResponseObserved",
            Self::Decoded => "Decoded",
            Self::Deferred => "Deferred",
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

pub(crate) fn recovery_progress_level_from_episode(
    status: &str,
    response_verdict: Option<&str>,
    first_video_packet_is_keyframe: Option<bool>,
    first_keyframe_packet_at_ms: Option<f64>,
    first_keyframe_decoded_at_ms: Option<f64>,
    has_current_clean_anchor: bool,
    has_display_stable: bool,
) -> Option<RecoveryProgressLevel> {
    if has_display_stable {
        return Some(RecoveryProgressLevel::DisplayStable);
    }
    if has_current_clean_anchor || response_verdict == Some("cleanAnchorCommitted") {
        return Some(RecoveryProgressLevel::CleanAnchorCommitted);
    }
    if first_keyframe_decoded_at_ms.is_some() || status == "decoded" {
        return Some(RecoveryProgressLevel::Decoded);
    }
    if first_keyframe_packet_at_ms.is_some()
        || first_video_packet_is_keyframe == Some(true)
        || matches!(status, "packet-seen")
    {
        return Some(RecoveryProgressLevel::AnchorSeen);
    }
    if matches!(status, "response-observed")
        || (first_video_packet_is_keyframe == Some(false) && response_verdict != Some("pending"))
    {
        return Some(RecoveryProgressLevel::ContinuationSeen);
    }
    if matches!(
        status,
        "requested" | "sent" | "deferred" | "failed" | "expired-unsent" | "missed"
    ) {
        return Some(RecoveryProgressLevel::WaitingResponse);
    }
    None
}

pub(crate) fn recovery_progress_missing_anchor(progress: Option<RecoveryProgressLevel>) -> bool {
    matches!(
        progress,
        Some(RecoveryProgressLevel::WaitingResponse | RecoveryProgressLevel::ContinuationSeen)
            | None
    )
}

#[allow(dead_code)]
pub(crate) fn recovery_progress_allows_decoder_reset(
    progress: Option<RecoveryProgressLevel>,
) -> bool {
    matches!(
        progress,
        Some(
            RecoveryProgressLevel::AnchorSeen
                | RecoveryProgressLevel::Decoded
                | RecoveryProgressLevel::PlaybackRecovered
                | RecoveryProgressLevel::CleanAnchorCommitted
                | RecoveryProgressLevel::DisplayStable
        )
    )
}

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

pub(crate) fn is_ingress_waiting_keyframe(
    receiver_state: Option<&str>,
    chain_state: Option<&str>,
    chain_reason: Option<&str>,
    source_event: Option<&str>,
) -> bool {
    if is_receiver_state_receiving(receiver_state) {
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
    matches!(chain_state, Some("waiting-keyframe" | "repairing"))
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
        && clean_anchor_source_event == Some("chain-clean-anchor-submitted")
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

pub(crate) fn current_clean_anchor_observed_at_ms_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
) -> Option<f64> {
    current_clean_anchor_observed_at_ms(
        stats.video_anchor_clean_epoch,
        stats.video_anchor_clean_observed_at_ms,
        stats.video_anchor_clean_source_event.as_deref(),
        stats.transport_recovery_epoch,
    )
}

pub(crate) fn has_current_clean_anchor_from_stats(stats: &XbxEngineMediaRuntimeStats) -> bool {
    current_clean_anchor_observed_at_ms_from_stats(stats).is_some()
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
    let current_clean_anchor_observed_at_ms = current_clean_anchor_observed_at_ms_from_stats(stats);
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

/// 从 timeline 观测推导统一 `GapSeverity`（不含 episode stalled→RecoveryBlocked，见
/// [`derive_gap_severity_with_episode_stall`]）。
pub(crate) fn derive_gap_severity_from_timeline_observation(
    timeline: &XbxEngineVideoTimelineObservation,
) -> GapSeverity {
    let reason = timeline.chain.reason.as_deref();
    if matches!(reason, Some("referenceChainUnrecoverable")) {
        if chain_broken_observation_lacks_media_evidence(timeline) {
            return GapSeverity::ReferenceGap;
        }
        return GapSeverity::ChainBroken;
    }
    if matches!(
        reason,
        Some("awaitingRecoveryAnchor" | "awaitRecoveryAnchor" | "receiverWaitingKeyframe",)
    ) {
        return GapSeverity::AnchorGap;
    }
    if let Some(gap) = timeline.gap.as_ref() {
        if timeline_gap_implies_reference_gap_evidence(gap) {
            return GapSeverity::ReferenceGap;
        }
        return GapSeverity::RepairableGap;
    }
    GapSeverity::LowValueGap
}

/// 纯 transport 预算抬价 + 匿名缺洞时，不把 `chain.reason` 上的坏链语义升级成 `ChainBroken`。
fn chain_broken_observation_lacks_media_evidence(
    timeline: &XbxEngineVideoTimelineObservation,
) -> bool {
    let Some(gap) = timeline.gap.as_ref() else {
        return false;
    };
    if gap.frame_rtp_timestamp.is_some() {
        return false;
    }
    if gap.gap_dependency_confidence.as_deref() == Some("bound") {
        return false;
    }
    let evidence = gap.evidence_importance.as_deref().unwrap_or("unknown");
    if evidence != "unknown" {
        return false;
    }
    matches!(gap.budget_importance.as_deref(), Some("supply" | "anchor"))
}

/// 仅当 gap 快照携带 reference 级媒体/依赖证据时，才允许从「可修补缺口」升格为 `ReferenceGap`。
fn timeline_gap_implies_reference_gap_evidence(gap: &XbxEngineVideoTimelineGapSnapshot) -> bool {
    if gap.gap_dependency_confidence.as_deref() == Some("bound") {
        return true;
    }
    matches!(
        gap.evidence_importance.as_deref(),
        Some("reference" | "supply" | "anchor")
    ) || matches!(
        gap.frame_importance.as_deref(),
        Some("reference" | "supply" | "anchor" | "keyframe")
    )
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
                "awaitingRecoveryAnchor"
                    | "awaitRecoveryAnchor"
                    | "receiverWaitingKeyframe"
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
        GapSeverity::LowValueGap | GapSeverity::RepairableGap => Some(FrameValue::Continuity),
    }
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

#[cfg(test)]
mod derive_gap_observation_tests {
    use super::*;
    use crate::{
        XbxEngineH264InspectionObservation, XbxEngineMediaRuntimeStats,
        XbxEngineVideoTimelineChainSnapshot, XbxEngineVideoTimelineGapSnapshot,
        XbxEngineVideoTimelineObservation,
    };

    #[test]
    fn timeline_gap_without_reference_evidence_maps_to_repairable_gap() {
        let obs = XbxEngineVideoTimelineObservation {
            observation_id: 99,
            source_event: "gap-observed".into(),
            gap: Some(XbxEngineVideoTimelineGapSnapshot {
                state: "observed".into(),
                sequence: Some(10),
                frame_rtp_timestamp: Some(42),
                frame_importance: Some("delta".into()),
                budget_importance: Some("disposable".into()),
                evidence_importance: Some("unknown".into()),
                gap_dependency_confidence: Some("anonymous".into()),
                observed_at_ms: 0.0,
            }),
            frame: None,
            chain: XbxEngineVideoTimelineChainSnapshot {
                state: "receiving".into(),
                reason: None,
                chain_break_evidence: None,
                observed_at_ms: 0.0,
            },
            observed_at_ms: 0.0,
        };
        assert_eq!(
            derive_gap_severity_from_timeline_observation(&obs),
            GapSeverity::RepairableGap
        );
    }

    #[test]
    fn chain_broken_reason_with_anonymous_budget_only_gap_maps_to_reference_severity() {
        let obs = XbxEngineVideoTimelineObservation {
            observation_id: 1,
            source_event: "t".into(),
            gap: Some(XbxEngineVideoTimelineGapSnapshot {
                state: "observed".into(),
                sequence: Some(1),
                frame_rtp_timestamp: None,
                frame_importance: Some("unknown".into()),
                budget_importance: Some("supply".into()),
                evidence_importance: Some("unknown".into()),
                gap_dependency_confidence: Some("anonymous".into()),
                observed_at_ms: 0.0,
            }),
            frame: None,
            chain: XbxEngineVideoTimelineChainSnapshot {
                state: "waiting-keyframe".into(),
                reason: Some("referenceChainUnrecoverable".into()),
                chain_break_evidence: None,
                observed_at_ms: 0.0,
            },
            observed_at_ms: 0.0,
        };
        assert_eq!(
            derive_gap_severity_from_timeline_observation(&obs),
            GapSeverity::ReferenceGap
        );
    }

    #[test]
    fn fresh_invalid_bootstrap_breaks_sustaining_recovery_suppression_after_clean_anchor() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.transport_recovery_epoch = 7;
        stats.video_anchor_clean_epoch = Some(7);
        stats.video_anchor_clean_observed_at_ms = Some(100.0);
        stats.video_anchor_clean_source_event = Some("chain-clean-anchor-submitted".into());
        stats.latest_video_timeline_observation = Some(XbxEngineVideoTimelineObservation {
            observation_id: 1,
            source_event: "frame-complete-candidate-decode-feedback-blocked".into(),
            gap: Some(XbxEngineVideoTimelineGapSnapshot {
                state: "expired".into(),
                sequence: Some(1),
                frame_rtp_timestamp: None,
                frame_importance: Some("anchor".into()),
                budget_importance: Some("disposable".into()),
                evidence_importance: Some("anchor".into()),
                gap_dependency_confidence: Some("anonymous".into()),
                observed_at_ms: 180.0,
            }),
            frame: None,
            chain: XbxEngineVideoTimelineChainSnapshot {
                state: "sustaining-recovery".into(),
                reason: Some("recoverySustaining".into()),
                chain_break_evidence: None,
                observed_at_ms: 180.0,
            },
            observed_at_ms: 180.0,
        });
        stats.latest_h264_inspection_observation = Some(XbxEngineH264InspectionObservation {
            observation_id: 2,
            frame_rtp_timestamp: Some(7001),
            nal_types: vec!["SliceLayerWithoutPartitioningNonIdr".into()],
            nal_count: 1,
            vcl_nal_count: 1,
            has_inband_sps: false,
            has_inband_pps: false,
            committed_sps_present: true,
            committed_pps_present: true,
            slice_headers_valid: true,
            delta_continuation_ready: true,
            parameter_sets_changed: false,
            config_changed: false,
            is_idr: false,
            sample_width: Some(1920),
            sample_height: Some(1080),
            bootstrap_ready: false,
            bootstrap_reject_reason: Some("NonIdrVcl".into()),
            admission_accepted: true,
            observed_at_ms: 190.0,
            ..Default::default()
        });

        assert!(!has_current_transport_await_issue_from_stats(&stats));
    }

    #[test]
    fn stale_invalid_bootstrap_does_not_break_sustaining_recovery_suppression() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.transport_recovery_epoch = 7;
        stats.video_anchor_clean_epoch = Some(7);
        stats.video_anchor_clean_observed_at_ms = Some(100.0);
        stats.video_anchor_clean_source_event = Some("chain-clean-anchor-submitted".into());
        stats.latest_video_timeline_observation = Some(XbxEngineVideoTimelineObservation {
            observation_id: 1,
            source_event: "frame-complete-candidate-decode-feedback-blocked".into(),
            gap: Some(XbxEngineVideoTimelineGapSnapshot {
                state: "expired".into(),
                sequence: Some(1),
                frame_rtp_timestamp: None,
                frame_importance: Some("anchor".into()),
                budget_importance: Some("disposable".into()),
                evidence_importance: Some("anchor".into()),
                gap_dependency_confidence: Some("anonymous".into()),
                observed_at_ms: 500.0,
            }),
            frame: None,
            chain: XbxEngineVideoTimelineChainSnapshot {
                state: "sustaining-recovery".into(),
                reason: Some("recoverySustaining".into()),
                chain_break_evidence: None,
                observed_at_ms: 500.0,
            },
            observed_at_ms: 500.0,
        });
        stats.latest_h264_inspection_observation = Some(XbxEngineH264InspectionObservation {
            observation_id: 2,
            frame_rtp_timestamp: Some(7001),
            nal_types: vec!["SliceLayerWithoutPartitioningNonIdr".into()],
            nal_count: 1,
            vcl_nal_count: 1,
            has_inband_sps: false,
            has_inband_pps: false,
            committed_sps_present: true,
            committed_pps_present: true,
            slice_headers_valid: true,
            delta_continuation_ready: true,
            parameter_sets_changed: false,
            config_changed: false,
            is_idr: false,
            sample_width: Some(1920),
            sample_height: Some(1080),
            bootstrap_ready: false,
            bootstrap_reject_reason: Some("NonIdrVcl".into()),
            admission_accepted: true,
            observed_at_ms: 190.0,
            ..Default::default()
        });

        assert!(!has_current_transport_await_issue_from_stats(&stats));
    }

    #[test]
    fn recovery_progress_level_mapping_follows_rfc_order() {
        assert_eq!(
            recovery_progress_level_from_episode(
                "requested",
                Some("pending"),
                None,
                None,
                None,
                false,
                false
            ),
            Some(RecoveryProgressLevel::WaitingResponse)
        );
        assert_eq!(
            recovery_progress_level_from_episode(
                "response-observed",
                Some("on-time"),
                Some(false),
                None,
                None,
                false,
                false
            ),
            Some(RecoveryProgressLevel::ContinuationSeen)
        );
        assert_eq!(
            recovery_progress_level_from_episode(
                "packet-seen",
                Some("on-time"),
                Some(true),
                Some(10.0),
                None,
                false,
                false
            ),
            Some(RecoveryProgressLevel::AnchorSeen)
        );
        assert_eq!(
            recovery_progress_level_from_episode(
                "decoded",
                Some("on-time"),
                Some(true),
                Some(10.0),
                Some(20.0),
                false,
                false
            ),
            Some(RecoveryProgressLevel::Decoded)
        );
        assert_eq!(
            recovery_progress_level_from_episode(
                "decoded",
                Some("cleanAnchorCommitted"),
                Some(true),
                Some(10.0),
                Some(20.0),
                true,
                false
            ),
            Some(RecoveryProgressLevel::CleanAnchorCommitted)
        );
        assert_eq!(
            recovery_progress_level_from_episode(
                "decoded",
                Some("cleanAnchorCommitted"),
                Some(true),
                Some(10.0),
                Some(20.0),
                true,
                true
            ),
            Some(RecoveryProgressLevel::DisplayStable)
        );
    }

    #[test]
    fn recovery_progress_gap_helpers_match_contract() {
        assert!(recovery_progress_missing_anchor(Some(
            RecoveryProgressLevel::WaitingResponse
        )));
        assert!(recovery_progress_missing_anchor(Some(
            RecoveryProgressLevel::ContinuationSeen
        )));
        assert!(!recovery_progress_missing_anchor(Some(
            RecoveryProgressLevel::AnchorSeen
        )));
        assert_eq!(
            recovery_progress_level_from_str("ContinuationSeen"),
            Some(RecoveryProgressLevel::ContinuationSeen)
        );
        assert_eq!(recovery_progress_level_from_str("unknown"), None);
        assert!(recovery_progress_allows_decoder_reset(Some(
            RecoveryProgressLevel::Decoded
        )));
        assert!(!recovery_progress_allows_decoder_reset(Some(
            RecoveryProgressLevel::ContinuationSeen
        )));
    }
}
