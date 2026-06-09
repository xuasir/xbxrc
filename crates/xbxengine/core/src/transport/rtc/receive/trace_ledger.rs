use std::collections::BTreeMap;

use crate::media::video::ingress::budget::FrameBudgetContext;
use crate::media::video::types::FrameRecoveryDisposition;
use crate::transport::rtc::receive::recovery_ledger::ReceiveRecoveryLedger;
use crate::transport::rtc::recovery::contract::{
    reference_chain_diagnostic_facts_from_stats, ReferenceChainObservation, ReferenceChainState,
};
use crate::{
    XbxEngineAnchorCandidateFailureReason, XbxEngineAnchorCandidateLedger,
    XbxEngineAnchorCandidateState, XbxEngineMediaRuntimeStats, XbxEngineVideoTimelineChainSnapshot,
    XbxEngineVideoTimelineFrameSnapshot, XbxEngineVideoTimelineGapSnapshot,
    XbxEngineVideoTimelineObservation,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GapState {
    Observed,
    ReorderPending,
    NackCandidate,
    RepairInFlight,
    Resolved,
}

impl GapState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Observed => "observed",
            Self::ReorderPending => "reorder-pending",
            Self::NackCandidate => "nack-candidate",
            Self::RepairInFlight => "repair-in-flight",
            Self::Resolved => "resolved",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FrameReceiveState {
    Open,
    GapPresent,
    Repairing,
    CompleteCandidate,
    Closed,
}

impl FrameReceiveState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::GapPresent => "gap-present",
            Self::Repairing => "repairing",
            Self::CompleteCandidate => "complete-candidate",
            Self::Closed => "closed",
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct FrameRecoveryLedgerEntry {
    pub(crate) frame_playout_deadline_at_ms: Option<f64>,
    pub(crate) frame_recovery_disposition: FrameRecoveryDisposition,
    pub(crate) frame_unrecoverable_reason: Option<String>,
    pub(crate) budget_context: FrameBudgetContext,
}

#[derive(Clone, Debug)]
struct GapEntry {
    state: GapState,
    frame_rtp_timestamp: Option<u32>,
    /// NACK / `FrameBudgetContext` 调度 importance，不等价于媒体断链证据。
    budget_importance: &'static str,
    /// 媒体因果侧（绑定帧、IDR、参数集变更等）importance。
    evidence_importance: &'static str,
    provenance: GapProvenance,
    severity: TimelineGapHardness,
    #[allow(dead_code)]
    first_observed_at_ms: f64,
    last_updated_at_ms: f64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GapProvenance {
    NetworkOrUnknown,
    LocalLowValueDrop,
    Repair,
}

/// 时间线内部的缺洞“硬度”，与 `recovery::contract::GapSeverity`（Minor/Reference/…）正交；
/// 统一语义帧价值由 contract 从 `XbxEngineVideoTimelineObservation` 推导。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TimelineGapHardness {
    Soft,
    Hard,
}

#[derive(Clone, Debug)]
struct FrameEntry {
    state: FrameReceiveState,
    #[allow(dead_code)]
    first_observed_at_ms: f64,
    last_updated_at_ms: f64,
    is_keyframe: Option<bool>,
    budget_importance: &'static str,
    evidence_importance: &'static str,
    close_reason: Option<&'static str>,
}

#[derive(Clone, Debug)]
pub(super) struct TimelineSnapshot {
    pub(super) gap: Option<XbxEngineVideoTimelineGapSnapshot>,
    pub(super) frame: Option<XbxEngineVideoTimelineFrameSnapshot>,
    pub(super) chain: XbxEngineVideoTimelineChainSnapshot,
}

#[derive(Clone, Debug)]
struct AnchorCandidateEntry {
    recovery_epoch: u64,
    frame_rtp_timestamp: Option<u32>,
    state: XbxEngineAnchorCandidateState,
    source_event: String,
    failure_reason: Option<XbxEngineAnchorCandidateFailureReason>,
    observed_at_ms: f64,
}

/// trace / frame-ledger 投影（**不**参与 pre-decode 裁决；裁决见 `ReceiverState` + `PacketBuffer`）。
pub(crate) struct ReceiverTraceLedger {
    gaps: BTreeMap<u16, GapEntry>,
    frames: BTreeMap<u32, FrameEntry>,
    frame_recovery_ledger: BTreeMap<u32, FrameRecoveryLedgerEntry>,
    latest_anchor_candidate: Option<AnchorCandidateEntry>,
    timeout_reason: Option<&'static str>,
    /// 最近一次断链归因（供 trace / contract 门控）。
    last_chain_break_evidence: Option<String>,
    /// receive-local picture recovery 主事实。
    pub(crate) recovery: ReceiveRecoveryLedger,
}

impl ReceiverTraceLedger {
    pub(crate) fn new() -> Self {
        Self {
            gaps: BTreeMap::new(),
            frames: BTreeMap::new(),
            frame_recovery_ledger: BTreeMap::new(),
            latest_anchor_candidate: None,
            timeout_reason: None,
            last_chain_break_evidence: None,
            recovery: ReceiveRecoveryLedger::default(),
        }
    }

    pub(crate) fn recovery_ledger(&self) -> &ReceiveRecoveryLedger {
        &self.recovery
    }

    pub(crate) fn recovery_ledger_mut(&mut self) -> &mut ReceiveRecoveryLedger {
        &mut self.recovery
    }

    pub(crate) fn note_clean_anchor_committed(&mut self, rtp_timestamp: Option<u32>) {
        self.recovery.note_clean_anchor_committed(rtp_timestamp);
        self.gaps.clear();
        self.frame_recovery_ledger.clear();
        self.latest_anchor_candidate = None;
        self.last_chain_break_evidence = None;
    }

    #[cfg(test)]
    pub(crate) fn has_hard_recovery_gap_risk(&self) -> bool {
        if self.gaps.values().any(|entry| {
            entry.severity == TimelineGapHardness::Hard
                && !matches!(entry.state, GapState::Resolved)
        }) {
            return true;
        }
        self.frame_recovery_ledger.values().any(|entry| {
            matches!(
                entry.frame_recovery_disposition,
                FrameRecoveryDisposition::UnrecoverableReferenceChain
            )
        })
    }

    pub(crate) fn reference_chain_observation(
        &self,
        stats: &XbxEngineMediaRuntimeStats,
        now_ms: f64,
        effective_rtt_ms: f64,
    ) -> ReferenceChainObservation {
        let _ = effective_rtt_ms;
        let has_unresolved_hard_gap = self.has_unresolved_hard_gap();
        let mut diagnostic_facts = reference_chain_diagnostic_facts_from_stats(stats, now_ms);
        diagnostic_facts.has_active_gap =
            diagnostic_facts.has_active_gap || has_unresolved_hard_gap;
        diagnostic_facts.nack_exhausted = diagnostic_facts.nack_exhausted
            || self.recovery.nack_state
                == crate::transport::rtc::receive::recovery_ledger::RecoveryNackState::Exhausted;
        if let Some(unrecoverable) =
            self.reference_chain_observation_from_unrecoverable(&diagnostic_facts)
        {
            return unrecoverable;
        }
        let effective_hard_gap =
            has_unresolved_hard_gap && !self.recovery.current_media_anchor_absorbs_repair_refresh();
        self.recovery.project_reference_chain(
            effective_hard_gap,
            diagnostic_facts.nack_exhausted,
            &diagnostic_facts,
        )
    }

    fn reference_chain_observation_from_unrecoverable(
        &self,
        stats_observation: &ReferenceChainObservation,
    ) -> Option<ReferenceChainObservation> {
        self.reference_chain_observation_from_receive_ledger(stats_observation)
    }

    fn reference_chain_observation_from_receive_ledger(
        &self,
        stats_observation: &ReferenceChainObservation,
    ) -> Option<ReferenceChainObservation> {
        if self.frame_recovery_ledger.values().any(|entry| {
            matches!(
                entry.frame_recovery_disposition,
                FrameRecoveryDisposition::UnrecoverableReferenceChain
            )
        }) {
            return Some(reference_observation_with_state(
                stats_observation,
                ReferenceChainState::NeedKeyframe,
                "receive-ledger-unrecoverable-reference-chain",
            ));
        }
        if self.latest_anchor_candidate.as_ref().is_some_and(|candidate| {
            matches!(candidate.state, XbxEngineAnchorCandidateState::Rejected)
                && matches!(
                    candidate.failure_reason,
                    Some(
                        XbxEngineAnchorCandidateFailureReason::AwaitingRecoveryKeyframe
                            | XbxEngineAnchorCandidateFailureReason::ChainBrokenReferenceUnrecoverable
                            | XbxEngineAnchorCandidateFailureReason::GapExpiredDeadline
                    )
                )
        }) {
            return Some(reference_observation_with_state(
                stats_observation,
                ReferenceChainState::NeedKeyframe,
                "receive-ledger-anchor-rejected",
            ));
        }
        if self.has_unresolved_hard_gap()
            && !self.recovery.current_media_anchor_absorbs_repair_refresh()
        {
            return Some(reference_observation_with_state(
                stats_observation,
                if stats_observation.nack_exhausted {
                    ReferenceChainState::NeedKeyframe
                } else {
                    ReferenceChainState::Repairing
                },
                if stats_observation.nack_exhausted {
                    "receive-ledger-hard-gap-nack-exhausted"
                } else {
                    "receive-ledger-hard-gap-repairing"
                },
            ));
        }
        None
    }

    pub(crate) fn apply_media_evidence_to_gaps_for_frame(
        &mut self,
        frame_rtp_timestamp: u32,
        evidence_importance: &'static str,
        now_ms: f64,
    ) {
        if evidence_importance == "unknown" {
            return;
        }
        for entry in self.gaps.values_mut() {
            if entry.frame_rtp_timestamp != Some(frame_rtp_timestamp) {
                continue;
            }
            entry.evidence_importance = evidence_importance;
            entry.last_updated_at_ms = now_ms;
            let merged_ts = entry.frame_rtp_timestamp;
            let media = effective_media_importance_for_gap(
                entry.budget_importance,
                entry.evidence_importance,
                merged_ts,
            );
            let (provenance, severity) = classify_gap(entry.state, media, merged_ts, None);
            entry.provenance = provenance;
            entry.severity = severity;
        }
    }

    pub(crate) fn on_timeout_detected(&mut self) {}

    pub(crate) fn observe_gap(
        &mut self,
        sequences: &[u16],
        now_ms: f64,
        frame_rtp_timestamp: Option<u32>,
        budget_importance: &'static str,
        evidence_importance: &'static str,
    ) {
        for sequence in sequences {
            self.update_gap(
                *sequence,
                GapState::Observed,
                now_ms,
                frame_rtp_timestamp,
                budget_importance,
                evidence_importance,
                None,
            );
        }
        if let Some(frame_rtp_timestamp) = frame_rtp_timestamp {
            self.update_frame(
                frame_rtp_timestamp,
                FrameReceiveState::GapPresent,
                now_ms,
                None,
                budget_importance,
                evidence_importance,
                None,
            );
        }
    }

    pub(crate) fn mark_gap_reorder_pending(
        &mut self,
        sequences: &[u16],
        now_ms: f64,
        frame_rtp_timestamp: Option<u32>,
        budget_importance: &'static str,
        evidence_importance: &'static str,
    ) {
        for sequence in sequences {
            self.update_gap(
                *sequence,
                GapState::ReorderPending,
                now_ms,
                frame_rtp_timestamp,
                budget_importance,
                evidence_importance,
                None,
            );
        }
    }

    pub(crate) fn mark_gap_nack_candidate(
        &mut self,
        sequences: &[u16],
        now_ms: f64,
        frame_rtp_timestamp: Option<u32>,
        budget_importance: &'static str,
        evidence_importance: &'static str,
    ) {
        for sequence in sequences {
            self.update_gap(
                *sequence,
                GapState::NackCandidate,
                now_ms,
                frame_rtp_timestamp,
                budget_importance,
                evidence_importance,
                None,
            );
        }
        if let Some(frame_rtp_timestamp) = frame_rtp_timestamp {
            self.update_frame(
                frame_rtp_timestamp,
                FrameReceiveState::Repairing,
                now_ms,
                None,
                budget_importance,
                evidence_importance,
                None,
            );
        }
    }

    pub(crate) fn mark_gap_repair_in_flight(
        &mut self,
        sequences: &[u16],
        now_ms: f64,
        frame_rtp_timestamp: Option<u32>,
        budget_importance: &'static str,
        evidence_importance: &'static str,
    ) {
        for sequence in sequences {
            self.update_gap(
                *sequence,
                GapState::RepairInFlight,
                now_ms,
                frame_rtp_timestamp,
                budget_importance,
                evidence_importance,
                None,
            );
        }
        if let Some(frame_rtp_timestamp) = frame_rtp_timestamp {
            self.update_frame(
                frame_rtp_timestamp,
                FrameReceiveState::Repairing,
                now_ms,
                None,
                budget_importance,
                evidence_importance,
                None,
            );
        }
    }

    pub(crate) fn mark_gap_resolved(
        &mut self,
        sequence: u16,
        now_ms: f64,
        frame_rtp_timestamp: Option<u32>,
        budget_importance: &'static str,
        evidence_importance: &'static str,
    ) {
        self.update_gap(
            sequence,
            GapState::Resolved,
            now_ms,
            frame_rtp_timestamp,
            budget_importance,
            evidence_importance,
            None,
        );
        if let Some(frame_rtp_timestamp) = frame_rtp_timestamp {
            let has_pending_gap = self.gaps.values().any(|entry| {
                entry.frame_rtp_timestamp == Some(frame_rtp_timestamp)
                    && !matches!(entry.state, GapState::Resolved)
            });
            self.update_frame(
                frame_rtp_timestamp,
                if has_pending_gap {
                    FrameReceiveState::Repairing
                } else {
                    FrameReceiveState::Open
                },
                now_ms,
                None,
                budget_importance,
                evidence_importance,
                None,
            );
        }
        self.note_packet_recovery_progress();
    }

    pub(crate) fn observe_frame(
        &mut self,
        frame_rtp_timestamp: u32,
        now_ms: f64,
        is_keyframe: Option<bool>,
        frame_importance: &'static str,
    ) {
        self.apply_media_evidence_to_gaps_for_frame(frame_rtp_timestamp, frame_importance, now_ms);
        self.timeout_reason = None;
        self.update_frame(
            frame_rtp_timestamp,
            FrameReceiveState::Open,
            now_ms,
            is_keyframe,
            frame_importance,
            frame_importance,
            None,
        );
    }

    pub(crate) fn record_timeout_reason(&mut self, reason: &'static str) {
        self.timeout_reason = Some(reason);
    }

    pub(crate) fn observe_anchor_candidate(
        &mut self,
        recovery_epoch: u64,
        frame_rtp_timestamp: Option<u32>,
        source_event: &str,
        state: XbxEngineAnchorCandidateState,
        failure_reason: Option<XbxEngineAnchorCandidateFailureReason>,
        observed_at_ms: f64,
    ) {
        let resolved_frame_rtp_timestamp = frame_rtp_timestamp.or_else(|| {
            self.latest_anchor_candidate
                .as_ref()
                .and_then(|candidate| {
                    (candidate.recovery_epoch == recovery_epoch
                        && should_inherit_anonymous_anchor_frame(source_event, candidate))
                    .then_some(candidate.frame_rtp_timestamp)
                })
                .flatten()
        });
        self.latest_anchor_candidate = Some(AnchorCandidateEntry {
            recovery_epoch,
            frame_rtp_timestamp: resolved_frame_rtp_timestamp,
            state,
            source_event: source_event.to_string(),
            failure_reason,
            observed_at_ms,
        });
    }

    pub(crate) fn latest_anchor_candidate_ledger(&self) -> Option<XbxEngineAnchorCandidateLedger> {
        self.latest_anchor_candidate
            .as_ref()
            .map(|candidate| XbxEngineAnchorCandidateLedger {
                recovery_epoch: candidate.recovery_epoch,
                frame_rtp_timestamp: candidate.frame_rtp_timestamp,
                state: candidate.state,
                source_event: candidate.source_event.clone(),
                failure_reason: candidate.failure_reason,
                observed_at_ms: candidate.observed_at_ms,
            })
    }

    pub(crate) fn mark_frame_closed(
        &mut self,
        frame_rtp_timestamp: u32,
        now_ms: f64,
        is_keyframe: Option<bool>,
        frame_importance: &'static str,
        close_reason: Option<&'static str>,
    ) {
        self.update_frame(
            frame_rtp_timestamp,
            FrameReceiveState::Closed,
            now_ms,
            is_keyframe,
            frame_importance,
            frame_importance,
            close_reason,
        );
    }

    pub(crate) fn mark_frame_complete_candidate(
        &mut self,
        frame_rtp_timestamp: u32,
        now_ms: f64,
        is_keyframe: Option<bool>,
        frame_importance: &'static str,
    ) {
        self.update_frame(
            frame_rtp_timestamp,
            FrameReceiveState::CompleteCandidate,
            now_ms,
            is_keyframe,
            frame_importance,
            frame_importance,
            None,
        );
        self.note_packet_recovery_progress();
    }

    pub(crate) fn note_packet_recovery_progress(&mut self) {
        let has_unresolved_hard_gap = self.has_unresolved_hard_gap();
        self.recovery
            .note_packet_gap_repaired(has_unresolved_hard_gap);
    }

    pub(crate) fn record_frame_recovery(
        &mut self,
        frame_rtp_timestamp: u32,
        frame_playout_deadline_at_ms: Option<f64>,
        frame_recovery_disposition: FrameRecoveryDisposition,
        frame_unrecoverable_reason: Option<&str>,
        budget_context: FrameBudgetContext,
    ) {
        let next_entry = FrameRecoveryLedgerEntry {
            frame_playout_deadline_at_ms,
            frame_recovery_disposition,
            frame_unrecoverable_reason: frame_unrecoverable_reason.map(str::to_string),
            budget_context,
        };
        if let Some(entry) = self.frame_recovery_ledger.get_mut(&frame_rtp_timestamp) {
            entry.frame_playout_deadline_at_ms = entry
                .frame_playout_deadline_at_ms
                .or(next_entry.frame_playout_deadline_at_ms);
            if matches!(
                next_entry.frame_recovery_disposition,
                FrameRecoveryDisposition::UnrecoverableReferenceChain
            ) || !matches!(
                entry.frame_recovery_disposition,
                FrameRecoveryDisposition::UnrecoverableReferenceChain
            ) {
                entry.frame_recovery_disposition = next_entry.frame_recovery_disposition;
            }
            if next_entry.frame_unrecoverable_reason.is_some() {
                entry.frame_unrecoverable_reason = next_entry.frame_unrecoverable_reason;
            }
            entry.budget_context = next_entry.budget_context;
        } else {
            self.frame_recovery_ledger
                .insert(frame_rtp_timestamp, next_entry);
        }
        if matches!(
            frame_recovery_disposition,
            FrameRecoveryDisposition::UnrecoverableReferenceChain
        ) {
            self.last_chain_break_evidence =
                Some("frameLedgerUnrecoverableReferenceChain".to_string());
        }
        self.prune_frame_recovery_ledger();
    }

    pub(crate) fn take_frame_recovery(
        &mut self,
        frame_rtp_timestamp: u32,
    ) -> Option<FrameRecoveryLedgerEntry> {
        self.frame_recovery_ledger.remove(&frame_rtp_timestamp)
    }

    pub(crate) fn snapshot_for_observation_with_receiver_state(
        &self,
        receiver_state: super::receiver_state::ReceiverState,
        observation_id: u64,
        source_event: &str,
        gap_sequence: Option<u16>,
        frame_rtp_timestamp: Option<u32>,
        now_ms: f64,
    ) -> XbxEngineVideoTimelineObservation {
        let mut snapshot = self.build_snapshot(gap_sequence, frame_rtp_timestamp, now_ms);
        snapshot.chain.state = receiver_state.timeline_chain_state_label().to_string();
        snapshot.chain.reason = receiver_state.timeline_chain_reason().map(str::to_string);
        XbxEngineVideoTimelineObservation {
            observation_id,
            source_event: source_event.to_string(),
            gap: snapshot.gap,
            frame: snapshot.frame,
            chain: snapshot.chain,
            observed_at_ms: now_ms,
        }
    }

    #[cfg(test)]
    pub(crate) fn has_hard_recovery_risk_for_test(&self) -> bool {
        self.has_hard_recovery_gap_risk()
    }

    fn update_gap(
        &mut self,
        sequence: u16,
        state: GapState,
        now_ms: f64,
        frame_rtp_timestamp: Option<u32>,
        budget_importance: &'static str,
        evidence_importance: &'static str,
        close_reason: Option<&'static str>,
    ) {
        if let Some(entry) = self.gaps.get_mut(&sequence) {
            // RTP sequence 是 16-bit，同号缺洞不能跨不同 frame RTP 继承 anchor 证据。
            let same_frame_or_unbound = frame_rtp_timestamp.is_none()
                || entry.frame_rtp_timestamp.is_none()
                || entry.frame_rtp_timestamp == frame_rtp_timestamp;
            entry.state = state;
            entry.last_updated_at_ms = now_ms;
            if same_frame_or_unbound {
                entry.frame_rtp_timestamp = entry.frame_rtp_timestamp.or(frame_rtp_timestamp);
                entry.budget_importance =
                    merge_importance_lane(entry.budget_importance, budget_importance);
                entry.evidence_importance =
                    merge_importance_lane(entry.evidence_importance, evidence_importance);
            }
            let merged_ts = entry.frame_rtp_timestamp;
            let media = effective_media_importance_for_gap(
                entry.budget_importance,
                entry.evidence_importance,
                merged_ts,
            );
            let (provenance, severity) = classify_gap(state, media, merged_ts, close_reason);
            entry.provenance = provenance;
            entry.severity = severity;
            return;
        }
        let merged_ts = frame_rtp_timestamp;
        let media =
            effective_media_importance_for_gap(budget_importance, evidence_importance, merged_ts);
        let (provenance, severity) = classify_gap(state, media, merged_ts, close_reason);
        self.gaps.insert(
            sequence,
            GapEntry {
                state,
                frame_rtp_timestamp,
                budget_importance,
                evidence_importance,
                provenance,
                severity,
                first_observed_at_ms: now_ms,
                last_updated_at_ms: now_ms,
            },
        );
    }

    fn update_frame(
        &mut self,
        frame_rtp_timestamp: u32,
        state: FrameReceiveState,
        now_ms: f64,
        is_keyframe: Option<bool>,
        budget_importance: &'static str,
        evidence_importance: &'static str,
        close_reason: Option<&'static str>,
    ) {
        if let Some(entry) = self.frames.get_mut(&frame_rtp_timestamp) {
            entry.state = state;
            entry.last_updated_at_ms = now_ms;
            entry.is_keyframe = entry.is_keyframe.or(is_keyframe);
            entry.budget_importance =
                merge_importance_lane(entry.budget_importance, budget_importance);
            entry.evidence_importance =
                merge_importance_lane(entry.evidence_importance, evidence_importance);
            if close_reason.is_some() {
                entry.close_reason = close_reason;
            }
            return;
        }
        self.frames.insert(
            frame_rtp_timestamp,
            FrameEntry {
                state,
                first_observed_at_ms: now_ms,
                last_updated_at_ms: now_ms,
                is_keyframe,
                budget_importance,
                evidence_importance,
                close_reason,
            },
        );
    }

    fn prune_frame_recovery_ledger(&mut self) {
        const MAX_LEDGER_ENTRIES: usize = 512;
        while self.frame_recovery_ledger.len() > MAX_LEDGER_ENTRIES {
            let Some((&oldest_timestamp, _)) = self.frame_recovery_ledger.first_key_value() else {
                break;
            };
            self.frame_recovery_ledger.remove(&oldest_timestamp);
        }
    }

    pub(crate) fn has_unresolved_hard_gap_for_internal(&self) -> bool {
        self.has_unresolved_hard_gap()
    }

    #[cfg(test)]
    pub(crate) fn has_unresolved_hard_gap_for_test(&self) -> bool {
        self.has_unresolved_hard_gap()
    }

    fn has_unresolved_hard_gap(&self) -> bool {
        self.gaps.values().any(|entry| {
            entry.severity == TimelineGapHardness::Hard
                && !matches!(entry.state, GapState::Resolved)
                && !matches!(entry.provenance, GapProvenance::LocalLowValueDrop)
        })
    }

    fn build_snapshot(
        &self,
        gap_sequence: Option<u16>,
        frame_rtp_timestamp: Option<u32>,
        now_ms: f64,
    ) -> TimelineSnapshot {
        let gap = self.resolve_gap_snapshot(gap_sequence, frame_rtp_timestamp, now_ms);
        let frame = self.resolve_frame_snapshot(frame_rtp_timestamp, now_ms);
        TimelineSnapshot {
            gap,
            frame,
            chain: XbxEngineVideoTimelineChainSnapshot {
                state: "receiving".to_string(),
                reason: None,
                chain_break_evidence: self.last_chain_break_evidence.clone(),
                observed_at_ms: now_ms,
            },
        }
    }

    fn resolve_gap_snapshot(
        &self,
        gap_sequence: Option<u16>,
        frame_rtp_timestamp: Option<u32>,
        now_ms: f64,
    ) -> Option<XbxEngineVideoTimelineGapSnapshot> {
        let map_entry = |sequence: u16, entry: &GapEntry| -> XbxEngineVideoTimelineGapSnapshot {
            let conf = if entry.frame_rtp_timestamp.is_some() {
                "bound"
            } else {
                "anonymous"
            };
            let evidence = entry.evidence_importance.to_string();
            XbxEngineVideoTimelineGapSnapshot {
                state: entry.state.as_str().to_string(),
                sequence: Some(sequence),
                frame_rtp_timestamp: entry.frame_rtp_timestamp,
                frame_importance: Some(evidence.clone()),
                budget_importance: Some(entry.budget_importance.to_string()),
                evidence_importance: Some(evidence),
                gap_dependency_confidence: Some(conf.to_string()),
                observed_at_ms: entry.last_updated_at_ms.max(now_ms),
            }
        };
        if let Some(sequence) = gap_sequence {
            if let Some(entry) = self.gaps.get(&sequence) {
                return Some(map_entry(sequence, entry));
            }
        }
        let candidate = frame_rtp_timestamp
            .and_then(|frame_ts| {
                self.gaps
                    .iter()
                    .find(|(_, entry)| entry.frame_rtp_timestamp == Some(frame_ts))
            })
            .or_else(|| self.gaps.last_key_value());
        candidate.map(|(sequence, entry)| map_entry(*sequence, entry))
    }

    fn resolve_frame_snapshot(
        &self,
        frame_rtp_timestamp: Option<u32>,
        now_ms: f64,
    ) -> Option<XbxEngineVideoTimelineFrameSnapshot> {
        let candidate = frame_rtp_timestamp
            .and_then(|frame_ts| self.frames.get_key_value(&frame_ts))
            .or_else(|| self.frames.last_key_value());
        candidate.map(|(frame_ts, entry)| {
            let ev = entry.evidence_importance.to_string();
            XbxEngineVideoTimelineFrameSnapshot {
                state: entry.state.as_str().to_string(),
                frame_rtp_timestamp: Some(*frame_ts),
                is_keyframe: entry.is_keyframe,
                frame_importance: Some(ev.clone()),
                budget_importance: Some(entry.budget_importance.to_string()),
                evidence_importance: Some(ev),
                close_reason: entry.close_reason.map(str::to_string),
                observed_at_ms: entry.last_updated_at_ms.max(now_ms),
            }
        })
    }
}

fn reference_observation_with_state(
    base: &ReferenceChainObservation,
    state: ReferenceChainState,
    cause: &'static str,
) -> ReferenceChainObservation {
    ReferenceChainObservation {
        state,
        cause,
        decoder_reference_synced: base.decoder_reference_synced,
        bootstrap_ready: base.bootstrap_ready,
        has_active_gap: true,
        nack_exhausted: base.nack_exhausted,
        submit_age_ms: base.submit_age_ms,
    }
}

fn merge_importance_lane(prev: &'static str, incoming: &'static str) -> &'static str {
    if incoming != "unknown" {
        incoming
    } else {
        prev
    }
}

/// 媒体因果 importance：匿名缺洞不把预算 supply/anchor 当作硬参考。
fn effective_media_importance_for_gap(
    budget: &'static str,
    evidence: &'static str,
    frame_rtp_timestamp: Option<u32>,
) -> &'static str {
    if frame_rtp_timestamp.is_none() {
        return "disposable";
    }
    if evidence != "unknown" {
        evidence
    } else {
        budget
    }
}

fn classify_gap(
    state: GapState,
    media_importance: &'static str,
    frame_rtp_timestamp: Option<u32>,
    close_reason: Option<&'static str>,
) -> (GapProvenance, TimelineGapHardness) {
    if matches!(state, GapState::RepairInFlight) {
        return (
            GapProvenance::Repair,
            if matches!(media_importance, "supply" | "anchor") {
                TimelineGapHardness::Hard
            } else {
                TimelineGapHardness::Soft
            },
        );
    }
    if matches!(close_reason, Some(reason) if is_local_low_value_gap_reason(reason)) {
        return (GapProvenance::LocalLowValueDrop, TimelineGapHardness::Soft);
    }
    (
        GapProvenance::NetworkOrUnknown,
        if matches!(media_importance, "supply" | "anchor")
            || (frame_rtp_timestamp.is_none()
                && media_importance == "disposable"
                && matches!(close_reason, Some("awaitingRecoveryAnchor")))
        {
            TimelineGapHardness::Hard
        } else {
            TimelineGapHardness::Soft
        },
    )
}

fn is_local_low_value_gap_reason(reason: &str) -> bool {
    matches!(
        reason,
        "cloudHighRttLowValueAdmission"
            | "localBackpressureDeltaGap"
            | "displayStarvedLowValueAdmission"
            | "estimatedArrivalNearDeadlineLowValue"
    )
}

fn should_inherit_anonymous_anchor_frame(
    source_event: &str,
    candidate: &AnchorCandidateEntry,
) -> bool {
    if !matches!(source_event, "gap-repair-in-flight" | "gap-resolved") {
        return true;
    }
    matches!(
        candidate.state,
        XbxEngineAnchorCandidateState::AwaitingRecovery
            | XbxEngineAnchorCandidateState::Repaired
            | XbxEngineAnchorCandidateState::Rejected
    )
}

#[cfg(test)]
mod inline_recovery_tests {
    use super::ReceiverTraceLedger;

    #[test]
    fn resolved_hard_gap_no_longer_keeps_hard_recovery_risk() {
        let mut state = ReceiverTraceLedger::new();
        state.mark_gap_reorder_pending(&[501], 1.0, Some(90_001), "supply", "supply");
        assert!(state.has_hard_recovery_risk_for_test());

        state.mark_gap_resolved(501, 2.0, Some(90_001), "supply", "supply");
        assert!(!state.has_hard_recovery_risk_for_test());
    }

    #[test]
    fn hard_gap_risk_persists_after_gap_observation() {
        let mut state = ReceiverTraceLedger::new();
        state.mark_gap_reorder_pending(&[601], 1.0, Some(90_001), "supply", "supply");
        assert!(state.has_hard_recovery_risk_for_test());
        state.mark_gap_resolved(601, 2.0, Some(90_001), "supply", "supply");
        assert!(!state.has_hard_recovery_risk_for_test());
    }

    #[test]
    fn hard_receive_ledger_gap_derives_repairing_reference_state() {
        let mut state = ReceiverTraceLedger::new();
        state.mark_gap_nack_candidate(&[701], 1.0, Some(90_001), "supply", "supply");
        let stats = crate::XbxEngineMediaRuntimeStats {
            recovery_decoder_reference_synced_at_ms: Some(1.0),
            ..Default::default()
        };

        let observation = state.reference_chain_observation(&stats, 2.0, 80.0);

        assert_eq!(
            observation.state,
            crate::transport::rtc::recovery::contract::ReferenceChainState::Repairing
        );
        assert_eq!(observation.cause, "receive-ledger-hard-gap-repairing");
    }

    #[test]
    fn clean_anchor_clears_prior_gap_and_anchor_candidate_projection() {
        let mut state = ReceiverTraceLedger::new();
        state.mark_gap_nack_candidate(&[701], 1.0, Some(90_000), "supply", "supply");
        state.observe_anchor_candidate(
            0,
            Some(90_000),
            "insert-gate-hold-repair",
            crate::XbxEngineAnchorCandidateState::Rejected,
            Some(crate::XbxEngineAnchorCandidateFailureReason::AwaitingRecoveryKeyframe),
            1.0,
        );

        state.note_clean_anchor_committed(Some(90_001));

        let stats = crate::XbxEngineMediaRuntimeStats {
            recovery_decoder_reference_synced_at_ms: Some(2.0),
            ..Default::default()
        };
        let observation = state.reference_chain_observation(&stats, 2.0, 80.0);
        assert_eq!(
            observation.state,
            crate::transport::rtc::recovery::contract::ReferenceChainState::Continuous
        );
        assert_eq!(observation.cause, "ledger-clean-anchor-committed");
    }

    #[test]
    fn active_hard_gap_after_clean_anchor_keeps_reference_chain_serviceable() {
        let mut state = ReceiverTraceLedger::new();
        state.note_clean_anchor_committed(Some(90_001));
        state.recovery.note_decoder_reference_synced(2.0);
        state.mark_gap_repair_in_flight(&[702], 3.0, Some(90_002), "supply", "supply");
        let stats = crate::XbxEngineMediaRuntimeStats {
            recovery_decoder_reference_synced_at_ms: Some(2.0),
            ..Default::default()
        };

        let observation = state.reference_chain_observation(&stats, 4.0, 80.0);

        assert_eq!(
            observation.state,
            crate::transport::rtc::recovery::contract::ReferenceChainState::Continuous
        );
        assert_eq!(observation.cause, "ledger-clean-anchor-committed");
        assert!(observation.has_active_gap);
    }

    #[test]
    fn nack_exhausted_after_clean_anchor_still_requests_keyframe() {
        let mut state = ReceiverTraceLedger::new();
        state.note_clean_anchor_committed(Some(90_001));
        state.recovery.note_decoder_reference_synced(2.0);
        state.mark_gap_repair_in_flight(&[703], 3.0, Some(90_002), "supply", "supply");
        state.recovery.note_nack_exhausted();
        let stats = crate::XbxEngineMediaRuntimeStats {
            recovery_decoder_reference_synced_at_ms: Some(2.0),
            ..Default::default()
        };

        let observation = state.reference_chain_observation(&stats, 4.0, 80.0);

        assert_eq!(
            observation.state,
            crate::transport::rtc::recovery::contract::ReferenceChainState::NeedKeyframe
        );
        assert_eq!(observation.cause, "receive-ledger-hard-gap-nack-exhausted");
    }

    #[test]
    fn disposable_nack_exhausted_after_clean_anchor_reopens_keyframe_window() {
        let mut state = ReceiverTraceLedger::new();
        state.note_clean_anchor_committed(Some(90_001));
        state.recovery.note_decoder_reference_synced(2.0);
        state.mark_gap_nack_candidate(&[704], 3.0, Some(90_002), "disposable", "unknown");
        state.recovery.note_nack_exhausted();
        let stats = crate::XbxEngineMediaRuntimeStats {
            recovery_decoder_reference_synced_at_ms: Some(2.0),
            ..Default::default()
        };

        let observation = state.reference_chain_observation(&stats, 4.0, 80.0);

        assert_eq!(
            observation.state,
            crate::transport::rtc::recovery::contract::ReferenceChainState::NeedKeyframe
        );
        assert_eq!(observation.cause, "nack-exhausted");
    }

    #[test]
    fn ledger_reference_observation_drives_insert_context_state() {
        use crate::transport::rtc::receive::insert_gate::InsertContext;
        use crate::transport::rtc::recovery::contract::ReferenceChainState;

        let mut state = ReceiverTraceLedger::new();
        state.mark_gap_nack_candidate(&[901], 1.0, Some(90_001), "supply", "supply");
        let stats = crate::XbxEngineMediaRuntimeStats {
            recovery_decoder_reference_synced_at_ms: Some(1.0),
            ..Default::default()
        };
        let observation = state.reference_chain_observation(&stats, 2.0, 80.0);
        assert_eq!(observation.state, ReferenceChainState::Repairing);
        let decode =
            crate::transport::rtc::receive::decode_gate::receiver_decode_context_from_stats(
                &stats, 2.0,
            );
        let ctx = InsertContext::from_runtime_with_reference(
            decode,
            &stats,
            2.0,
            80.0,
            observation,
            false,
        );
        assert_eq!(ctx.reference_chain_state, ReferenceChainState::Repairing);
    }

    #[test]
    fn ledger_repairing_persists_over_stats_submit_starved() {
        let mut state = ReceiverTraceLedger::new();
        state.mark_gap_nack_candidate(&[801], 1.0, Some(90_001), "supply", "supply");
        let stats = crate::XbxEngineMediaRuntimeStats {
            recovery_decoder_reference_synced_at_ms: Some(1.0),
            latest_video_decode_ok_time_ms: Some(1.0),
            latest_video_decode_ok_rtp_timestamp: Some(90_000),
            recovery_displayed_idr_at_ms: Some(1.0),
            recovery_displayed_idr_rtp: Some(90_000),
            submit_age_ms: Some(5_000.0),
            latest_video_receiver_observation: Some(crate::XbxEngineVideoReceiverObservation {
                observation_id: 1,
                receiver_state: "repairing".to_string(),
                gap_sequence: Some(801),
                gap_span: Some(1),
                nack_in_flight: false,
                keyframe_request_pending: true,
                bootstrap_reject_reason: None,
                observed_at_ms: 2.0,
            }),
            ..Default::default()
        };

        let merged = state.reference_chain_observation(&stats, 5_000.0, 80.0);
        assert_eq!(
            merged.state,
            crate::transport::rtc::recovery::contract::ReferenceChainState::Repairing
        );
        assert_eq!(merged.cause, "receive-ledger-hard-gap-repairing");
    }

    #[test]
    fn ledger_nack_exhausted_wins_over_stats_repairing_projection() {
        let mut state = ReceiverTraceLedger::new();
        state.mark_gap_nack_candidate(&[802], 1.0, Some(90_002), "supply", "supply");
        state.recovery.nack_state =
            crate::transport::rtc::receive::recovery_ledger::RecoveryNackState::Exhausted;
        let stats = crate::XbxEngineMediaRuntimeStats {
            recovery_decoder_reference_synced_at_ms: Some(1.0),
            ..Default::default()
        };

        let observation = state.reference_chain_observation(&stats, 2.0, 80.0);

        assert_eq!(
            observation.state,
            crate::transport::rtc::recovery::contract::ReferenceChainState::NeedKeyframe
        );
        assert_eq!(observation.cause, "receive-ledger-hard-gap-nack-exhausted");
    }

    #[test]
    fn disposable_transport_gap_does_not_reopen_keyframe_required() {
        let mut state = ReceiverTraceLedger::new();
        state.mark_gap_nack_candidate(&[804], 1.0, Some(90_004), "disposable", "unknown");
        state.recovery.note_nack_exhausted();

        state.note_packet_recovery_progress();
        state.recovery.note_decoder_reference_synced(2.0);

        assert!(!state.has_unresolved_hard_gap_for_test());
        assert_eq!(
            state.recovery.nack_state,
            crate::transport::rtc::receive::recovery_ledger::RecoveryNackState::None
        );
        assert!(!state.recovery.keyframe_required);

        let stats = crate::XbxEngineMediaRuntimeStats {
            recovery_decoder_reference_synced_at_ms: Some(1.0),
            ..Default::default()
        };
        let observation = state.reference_chain_observation(&stats, 2.0, 80.0);
        assert_eq!(
            observation.state,
            crate::transport::rtc::recovery::contract::ReferenceChainState::Continuous
        );
    }

    #[test]
    fn same_sequence_on_different_frame_does_not_upgrade_disposable_gap_to_anchor() {
        let mut state = ReceiverTraceLedger::new();
        state.mark_gap_nack_candidate(&[64271], 1.0, Some(2458554852), "disposable", "unknown");
        state.mark_gap_repair_in_flight(&[64271], 2.0, Some(2459104302), "anchor", "anchor");

        assert!(!state.has_unresolved_hard_gap_for_test());

        state.recovery.note_nack_exhausted();
        state.note_packet_recovery_progress();
        state.recovery.note_decoder_reference_synced(3.0);
        let stats = crate::XbxEngineMediaRuntimeStats {
            recovery_decoder_reference_synced_at_ms: Some(3.0),
            ..Default::default()
        };
        let observation = state.reference_chain_observation(&stats, 3.0, 80.0);
        assert_eq!(
            observation.state,
            crate::transport::rtc::recovery::contract::ReferenceChainState::Continuous
        );
    }

    #[test]
    fn same_frame_anchor_gap_remains_hard_when_repair_starts() {
        let mut state = ReceiverTraceLedger::new();
        state.mark_gap_nack_candidate(&[64271], 1.0, Some(2459104302), "anchor", "anchor");
        state.mark_gap_repair_in_flight(&[64271], 2.0, Some(2459104302), "anchor", "anchor");

        assert!(state.has_unresolved_hard_gap_for_test());
    }

    #[test]
    fn gap_resolved_clears_stale_nack_exhausted_debt() {
        let mut state = ReceiverTraceLedger::new();
        state.mark_gap_nack_candidate(&[803], 1.0, Some(90_003), "supply", "supply");
        state.recovery.note_nack_exhausted();

        state.mark_gap_resolved(803, 2.0, Some(90_003), "supply", "supply");

        assert_eq!(
            state.recovery.nack_state,
            crate::transport::rtc::receive::recovery_ledger::RecoveryNackState::None
        );
        assert!(!state.recovery.keyframe_required);
        assert!(!state.has_unresolved_hard_gap_for_test());
    }

    #[test]
    fn complete_candidate_without_hard_gap_clears_stale_nack_exhausted_debt() {
        let mut state = ReceiverTraceLedger::new();
        state.recovery.note_nack_exhausted();

        state.mark_frame_complete_candidate(90_004, 2.0, Some(false), "supply");

        assert_eq!(
            state.recovery.nack_state,
            crate::transport::rtc::receive::recovery_ledger::RecoveryNackState::None
        );
        assert!(!state.recovery.keyframe_required);
    }

    #[test]
    fn unrecoverable_receive_ledger_frame_derives_need_keyframe() {
        let mut state = ReceiverTraceLedger::new();
        state.record_frame_recovery(
            90_001,
            None,
            crate::media::video::types::FrameRecoveryDisposition::UnrecoverableReferenceChain,
            Some("referenceChainUnrecoverable"),
            crate::media::video::ingress::budget::FrameBudgetContext::default(),
        );
        let stats = crate::XbxEngineMediaRuntimeStats {
            recovery_decoder_reference_synced_at_ms: Some(1.0),
            ..Default::default()
        };

        let observation = state.reference_chain_observation(&stats, 2.0, 80.0);

        assert_eq!(
            observation.state,
            crate::transport::rtc::recovery::contract::ReferenceChainState::NeedKeyframe
        );
        assert_eq!(
            observation.cause,
            "receive-ledger-unrecoverable-reference-chain"
        );
    }

    #[test]
    fn bootstrap_priming_stats_unknown_is_not_elevated_to_continuous() {
        let state = ReceiverTraceLedger::new();
        let stats = crate::XbxEngineMediaRuntimeStats::default();
        let observation = state.reference_chain_observation(&stats, 100.0, 80.0);
        assert_eq!(
            observation.state,
            crate::transport::rtc::recovery::contract::ReferenceChainState::Unknown
        );
        assert_eq!(observation.cause, "ledger-bootstrap-missing-priming");
    }
}
