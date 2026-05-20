use std::collections::BTreeMap;

use crate::media::video::ingress::budget::FrameBudgetContext;
use crate::media::video::types::FrameRecoveryDisposition;
use crate::{
    XbxEngineAnchorCandidateFailureReason, XbxEngineAnchorCandidateLedger,
    XbxEngineAnchorCandidateState, XbxEngineVideoTimelineChainSnapshot,
    XbxEngineVideoTimelineFrameSnapshot, XbxEngineVideoTimelineGapSnapshot,
    XbxEngineVideoTimelineObservation,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GapState {
    #[allow(dead_code)]
    Idle,
    Observed,
    ReorderPending,
    NackCandidate,
    RepairInFlight,
    Resolved,
    Expired,
}

impl GapState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Observed => "observed",
            Self::ReorderPending => "reorder-pending",
            Self::NackCandidate => "nack-candidate",
            Self::RepairInFlight => "repair-in-flight",
            Self::Resolved => "resolved",
            Self::Expired => "expired",
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
}

#[allow(dead_code)] // 兼容旧测试名；新代码用 ReceiverTraceLedger
pub(crate) type VideoTimelineState = ReceiverTraceLedger;

impl ReceiverTraceLedger {
    pub(crate) fn new() -> Self {
        Self {
            gaps: BTreeMap::new(),
            frames: BTreeMap::new(),
            frame_recovery_ledger: BTreeMap::new(),
            latest_anchor_candidate: None,
            timeout_reason: None,
            last_chain_break_evidence: None,
        }
    }

    /// RFC：pre-decode 裁决只用 [`ReceiverState`]；保留供旧 trace 测试编译。
    pub(crate) fn chain_requires_recovery_anchor(&self) -> bool {
        false
    }

    pub(crate) fn has_active_gap(&self) -> bool {
        self.gaps.values().any(|gap| {
            !matches!(
                gap.state,
                GapState::Resolved | GapState::Expired | GapState::Idle
            )
        })
    }

    pub(crate) fn in_sustaining_recovery(&self) -> bool {
        false
    }

    pub(crate) fn apply_wait_keyframe_gate(&mut self, _waiting: bool) {}

    pub(crate) fn on_admission_await_recovery_keyframe(&mut self, _reason: Option<&'static str>) {}

    pub(crate) fn on_sustaining_recovery_failed(&mut self, _reason: &'static str) {}

    pub(crate) fn has_hard_recovery_gap_risk(&self) -> bool {
        if self.gaps.values().any(|entry| {
            entry.severity == TimelineGapHardness::Hard
                && !matches!(entry.state, GapState::Resolved | GapState::Expired)
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

    pub(crate) fn on_recovery_keyframe_requested(&mut self) {}

    pub(crate) fn on_recovery_keyframe_requested_soft(&mut self) {}

    pub(crate) fn on_chain_broken(&mut self) {
        self.last_chain_break_evidence
            .get_or_insert_with(|| "externalChainBroken".to_string());
    }

    pub(crate) fn on_clean_anchor_ingress(
        &mut self,
        _anchor_rtp_timestamp: u32,
        _observed_at_ms: f64,
    ) {
    }

    pub(crate) fn on_clean_anchor_stats_committed(&mut self) {}

    pub(crate) fn on_clean_anchor_submitted(&mut self) {}

    pub(crate) fn peek_clean_anchor_stats_commit_candidate_if_stable(
        &mut self,
        _complete_candidate_rtp_ts: u32,
        _now_ms: f64,
    ) -> Option<u32> {
        None
    }

    pub(crate) fn ack_clean_anchor_stats_committed(&mut self, _committed_rtp_ts: u32) -> bool {
        false
    }

    pub(crate) fn ack_pending_clean_anchor_stats_committed(&mut self) -> bool {
        false
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

    #[allow(dead_code)]
    pub(crate) fn abandon_clean_anchor_pipeline_pending(&mut self) {}

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
                    && !matches!(entry.state, GapState::Resolved | GapState::Expired)
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
    }

    pub(crate) fn mark_gap_expired(
        &mut self,
        sequences: &[u16],
        now_ms: f64,
        frame_rtp_timestamp: Option<u32>,
        budget_importance: &'static str,
        evidence_importance: &'static str,
        close_reason: Option<&'static str>,
    ) -> bool {
        let soft_reentry = can_soften_expired_delta_reentry(close_reason);
        for sequence in sequences {
            self.update_gap(
                *sequence,
                GapState::Expired,
                now_ms,
                frame_rtp_timestamp,
                budget_importance,
                evidence_importance,
                close_reason,
            );
        }
        if let Some(frame_rtp_timestamp) = frame_rtp_timestamp {
            self.update_frame(
                frame_rtp_timestamp,
                FrameReceiveState::Closed,
                now_ms,
                None,
                budget_importance,
                evidence_importance,
                close_reason,
            );
        }
        let chain_broken = self.should_expired_gap_break_chain(
            frame_rtp_timestamp,
            budget_importance,
            evidence_importance,
            close_reason,
            soft_reentry,
        );
        if chain_broken {
            self.last_chain_break_evidence = Some(
                if self.frame_rtp_has_unrecoverable_reference_ledger(frame_rtp_timestamp) {
                    "frameLedgerUnrecoverableReferenceChain".to_string()
                } else {
                    expired_gap_chain_break_evidence(
                        frame_rtp_timestamp,
                        budget_importance,
                        evidence_importance,
                        close_reason,
                    )
                    .to_string()
                },
            );
        }
        chain_broken
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

    pub(crate) fn snapshot_for_observation(
        &self,
        observation_id: u64,
        source_event: &str,
        gap_sequence: Option<u16>,
        frame_rtp_timestamp: Option<u32>,
        now_ms: f64,
    ) -> XbxEngineVideoTimelineObservation {
        self.snapshot_for_observation_with_receiver_state(
            super::receiver_state::ReceiverState::Receiving,
            observation_id,
            source_event,
            gap_sequence,
            frame_rtp_timestamp,
            now_ms,
        )
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
    pub(crate) fn gap_state_of(&self, sequence: u16) -> Option<GapState> {
        self.gaps.get(&sequence).map(|entry| entry.state)
    }

    #[cfg(test)]
    pub(crate) fn frame_state_of(&self, frame_rtp_timestamp: u32) -> Option<FrameReceiveState> {
        self.frames
            .get(&frame_rtp_timestamp)
            .map(|entry| entry.state)
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
            entry.state = state;
            entry.last_updated_at_ms = now_ms;
            entry.frame_rtp_timestamp = entry.frame_rtp_timestamp.or(frame_rtp_timestamp);
            entry.budget_importance =
                merge_importance_lane(entry.budget_importance, budget_importance);
            entry.evidence_importance =
                merge_importance_lane(entry.evidence_importance, evidence_importance);
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
                state: trace_chain_state_placeholder(self).to_string(),
                reason: self.trace_chain_reason(frame_rtp_timestamp),
                chain_break_evidence: self.last_chain_break_evidence.clone(),
                observed_at_ms: now_ms,
            },
        }
    }

    fn frame_rtp_has_unrecoverable_reference_ledger(
        &self,
        frame_rtp_timestamp: Option<u32>,
    ) -> bool {
        frame_rtp_timestamp.is_some_and(|ts| {
            self.frame_recovery_ledger.get(&ts).is_some_and(|entry| {
                matches!(
                    entry.frame_recovery_disposition,
                    FrameRecoveryDisposition::UnrecoverableReferenceChain
                )
            })
        })
    }

    fn should_expired_gap_break_chain(
        &self,
        frame_rtp_timestamp: Option<u32>,
        budget_importance: &'static str,
        evidence_importance: &'static str,
        close_reason: Option<&'static str>,
        soft_reentry: bool,
    ) -> bool {
        if self.frame_rtp_has_unrecoverable_reference_ledger(frame_rtp_timestamp) {
            return true;
        }
        let media = effective_media_importance_for_gap(
            budget_importance,
            evidence_importance,
            frame_rtp_timestamp,
        );
        if matches!(media, "supply" | "anchor") {
            return true;
        }
        if matches!(close_reason, Some(reason) if is_local_low_value_gap_reason(reason)) {
            return false;
        }
        if soft_reentry {
            return false;
        }
        frame_rtp_timestamp.is_none()
            && media == "disposable"
            && matches!(close_reason, Some("awaitingRecoveryAnchor"))
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

    /// 未传入 `ReceiverState` 时的 trace-only 占位；生产观测由 `snapshot_for_observation_with_receiver_state` 覆盖。
    fn trace_chain_reason(&self, frame_rtp_timestamp: Option<u32>) -> Option<String> {
        if let Some(frame_ts) = frame_rtp_timestamp {
            if let Some(entry) = self.frames.get(&frame_ts) {
                if let Some(reason) = entry.close_reason {
                    return Some(reason.to_string());
                }
            }
        }
        if let Some(reason) = self.timeout_reason {
            return Some(reason.to_string());
        }
        if self.has_active_gap() {
            return Some("gapRepairInFlight".to_string());
        }
        None
    }
}

fn trace_chain_state_placeholder(ledger: &ReceiverTraceLedger) -> &'static str {
    if ledger.has_hard_recovery_gap_risk() {
        "waiting-keyframe"
    } else if ledger.has_active_gap() {
        "repairing"
    } else {
        "receiving"
    }
}

fn can_soften_expired_delta_reentry(close_reason: Option<&'static str>) -> bool {
    matches!(close_reason, Some("awaitingRecoveryAnchor"))
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

fn expired_gap_chain_break_evidence(
    frame_rtp_timestamp: Option<u32>,
    budget_importance: &'static str,
    evidence_importance: &'static str,
    close_reason: Option<&'static str>,
) -> &'static str {
    let media = effective_media_importance_for_gap(
        budget_importance,
        evidence_importance,
        frame_rtp_timestamp,
    );
    if frame_rtp_timestamp.is_none()
        && media == "disposable"
        && matches!(close_reason, Some("awaitingRecoveryAnchor"))
    {
        return "anonymousAwaitingKeyframeDelta";
    }
    if matches!(media, "supply" | "anchor") {
        return "boundMediaLikeGapExpired";
    }
    "genericGapExpired"
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
    fn clean_anchor_ingress_does_not_mutate_trace_chain_debt() {
        let mut state = ReceiverTraceLedger::new();
        state.on_admission_await_recovery_keyframe(Some("awaitingRecoveryAnchor"));
        state.mark_gap_reorder_pending(&[601], 1.0, Some(90_001), "supply", "supply");
        assert!(state.has_hard_recovery_risk_for_test());

        state.on_clean_anchor_ingress(90_050, 2.0);

        assert!(!state.chain_requires_recovery_anchor());
        assert!(state.has_hard_recovery_risk_for_test());
    }
}
