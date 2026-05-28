use crate::{XbxEngineVideoTimelineGapSnapshot, XbxEngineVideoTimelineObservation};

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
