use std::collections::BTreeMap;

use crate::media::video::ingress::budget::FrameBudgetContext;
use crate::media::video::types::FrameRecoveryDisposition;
use crate::{
    XbxEngineAnchorCandidateFailureReason, XbxEngineAnchorCandidateLedger,
    XbxEngineAnchorCandidateState, XbxEngineVideoTimelineChainSnapshot,
    XbxEngineVideoTimelineFrameSnapshot, XbxEngineVideoTimelineGapSnapshot,
    XbxEngineVideoTimelineObservation,
};

const RECOVERY_STABLE_MIN_CLEAN_FRAMES: u8 = 2;
const RECOVERY_STABLE_MIN_WINDOW_MS: f64 = 120.0;
const CLEAN_ANCHOR_SOFT_REENTRY_WINDOW_MS: f64 = 1_200.0;
const CLEAN_ANCHOR_SOFT_REENTRY_BUDGET: u8 = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GapState {
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ChainState {
    Healthy,
    Repairing,
    Broken,
    Recovering,
    Stalled,
}

impl ChainState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Healthy => "healthy",
            Self::Repairing => "repairing",
            Self::Broken => "broken",
            Self::Recovering => "recovering",
            Self::Stalled => "stalled",
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct FrameRecoveryLedgerEntry {
    pub(super) frame_playout_deadline_at_ms: Option<f64>,
    pub(super) frame_recovery_disposition: FrameRecoveryDisposition,
    pub(super) frame_unrecoverable_reason: Option<String>,
    pub(super) budget_context: FrameBudgetContext,
}

#[derive(Clone, Debug)]
struct GapEntry {
    state: GapState,
    frame_rtp_timestamp: Option<u32>,
    frame_importance: &'static str,
    provenance: GapProvenance,
    severity: GapSeverity,
    first_observed_at_ms: f64,
    last_updated_at_ms: f64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GapProvenance {
    NetworkOrUnknown,
    LocalLowValueDrop,
    Repair,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GapSeverity {
    Soft,
    Hard,
}

#[derive(Clone, Debug)]
struct FrameEntry {
    state: FrameReceiveState,
    first_observed_at_ms: f64,
    last_updated_at_ms: f64,
    is_keyframe: Option<bool>,
    frame_importance: &'static str,
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

pub(super) struct VideoTimelineState {
    chain_state: ChainState,
    gaps: BTreeMap<u16, GapEntry>,
    frames: BTreeMap<u32, FrameEntry>,
    frame_recovery_ledger: BTreeMap<u32, FrameRecoveryLedgerEntry>,
    latest_anchor_candidate: Option<AnchorCandidateEntry>,
    has_chain_debt: bool,
    chain_debt_reason: Option<String>,
    timeout_reason: Option<&'static str>,
    stable_recovery_started_at_ms: Option<f64>,
    stable_recovery_clean_frame_streak: u8,
    stable_recovery_last_frame_rtp_timestamp: Option<u32>,
    soft_reentry_protection_until_ms: Option<f64>,
    soft_reentry_budget_remaining: u8,
}

impl VideoTimelineState {
    pub(super) fn new() -> Self {
        Self {
            chain_state: ChainState::Healthy,
            gaps: BTreeMap::new(),
            frames: BTreeMap::new(),
            frame_recovery_ledger: BTreeMap::new(),
            latest_anchor_candidate: None,
            has_chain_debt: false,
            chain_debt_reason: None,
            timeout_reason: None,
            stable_recovery_started_at_ms: None,
            stable_recovery_clean_frame_streak: 0,
            stable_recovery_last_frame_rtp_timestamp: None,
            soft_reentry_protection_until_ms: None,
            soft_reentry_budget_remaining: 0,
        }
    }

    pub(super) fn waiting_for_recovery_keyframe(&self) -> bool {
        matches!(
            self.chain_state,
            ChainState::Broken | ChainState::Recovering
        )
    }

    pub(super) fn apply_wait_keyframe_gate(&mut self, waiting: bool) {
        if waiting {
            self.has_chain_debt = true;
            self.chain_debt_reason = Some("awaitRecoveryKeyframe".to_string());
            self.reset_stable_recovery_gate();
            self.chain_state = ChainState::Recovering;
            return;
        }
        if self.has_unrecoverable_frame_or_chain_debt() {
            self.reset_stable_recovery_gate();
            self.chain_state = ChainState::Recovering;
        } else {
            self.chain_state = ChainState::Healthy;
        }
    }

    pub(super) fn on_admission_await_recovery_keyframe(&mut self, reason: Option<&'static str>) {
        self.has_chain_debt = true;
        self.chain_debt_reason = Some(reason.unwrap_or("awaitingRecoveryKeyframe").to_string());
        self.reset_stable_recovery_gate();
        self.chain_state = ChainState::Recovering;
    }

    pub(super) fn has_hard_recovery_gap_risk(&self) -> bool {
        if matches!(self.chain_state, ChainState::Broken) {
            return true;
        }
        if self
            .gaps
            .values()
            .any(|entry| entry.severity == GapSeverity::Hard)
        {
            return true;
        }
        if self.frame_recovery_ledger.values().any(|entry| {
            matches!(
                entry.frame_recovery_disposition,
                FrameRecoveryDisposition::UnrecoverableReferenceChain
            )
        }) {
            return true;
        }
        self.chain_debt_reason
            .as_deref()
            .is_some_and(is_hard_recovery_reason)
    }

    pub(super) fn on_recovery_keyframe_requested(&mut self) {
        if self.chain_debt_reason.is_none() {
            self.chain_debt_reason = Some("awaitRecoveryKeyframe".to_string());
        }
        self.reset_stable_recovery_gate();
        self.chain_state = ChainState::Recovering;
    }

    pub(super) fn on_chain_broken(&mut self) {
        if self.chain_debt_reason.is_none() {
            self.chain_debt_reason = Some("referenceChainUnrecoverable".to_string());
        }
        self.reset_stable_recovery_gate();
        self.chain_state = ChainState::Broken;
    }

    pub(super) fn on_clean_keyframe_submitted(&mut self) {
        if self.has_unresolved_hard_gap_issue() {
            self.reset_stable_recovery_gate();
            if !matches!(self.chain_state, ChainState::Broken) {
                self.chain_state = ChainState::Recovering;
            }
            return;
        }
        self.clear_chain_debt();
        self.reset_stable_recovery_gate();
        self.chain_state = ChainState::Healthy;
        self.gaps.clear();
        self.arm_soft_reentry_protection_window();
    }

    pub(super) fn on_timeout_detected(&mut self) {
        if self.waiting_for_recovery_keyframe() {
            return;
        }
        self.reset_stable_recovery_gate();
        self.chain_state = ChainState::Stalled;
    }

    pub(super) fn observe_gap(
        &mut self,
        sequences: &[u16],
        now_ms: f64,
        frame_rtp_timestamp: Option<u32>,
        frame_importance: &'static str,
    ) {
        for sequence in sequences {
            self.update_gap(
                *sequence,
                GapState::Observed,
                now_ms,
                frame_rtp_timestamp,
                frame_importance,
                None,
            );
        }
        if let Some(frame_rtp_timestamp) = frame_rtp_timestamp {
            self.update_frame(
                frame_rtp_timestamp,
                FrameReceiveState::GapPresent,
                now_ms,
                None,
                frame_importance,
                None,
            );
        }
        if matches!(self.chain_state, ChainState::Healthy) {
            self.reset_stable_recovery_gate();
            self.chain_state = ChainState::Repairing;
        }
    }

    pub(super) fn mark_gap_reorder_pending(
        &mut self,
        sequences: &[u16],
        now_ms: f64,
        frame_rtp_timestamp: Option<u32>,
        frame_importance: &'static str,
    ) {
        let soft_reentry = self.try_consume_soft_reentry_budget(now_ms, frame_importance);
        for sequence in sequences {
            self.update_gap(
                *sequence,
                GapState::ReorderPending,
                now_ms,
                frame_rtp_timestamp,
                frame_importance,
                None,
            );
        }
        if soft_reentry
            && matches!(
                self.chain_state,
                ChainState::Healthy | ChainState::Repairing
            )
        {
            // clean anchor 短窗内的 delta 重入只做软观测，不把 owner 重新拖回恢复态。
            self.chain_state = ChainState::Healthy;
            return;
        }
        if matches!(self.chain_state, ChainState::Healthy) {
            self.reset_stable_recovery_gate();
            self.chain_state = ChainState::Repairing;
        }
    }

    pub(super) fn mark_gap_nack_candidate(
        &mut self,
        sequences: &[u16],
        now_ms: f64,
        frame_rtp_timestamp: Option<u32>,
        frame_importance: &'static str,
    ) {
        let soft_reentry = self.try_consume_soft_reentry_budget(now_ms, frame_importance);
        for sequence in sequences {
            self.update_gap(
                *sequence,
                GapState::NackCandidate,
                now_ms,
                frame_rtp_timestamp,
                frame_importance,
                None,
            );
        }
        if let Some(frame_rtp_timestamp) = frame_rtp_timestamp {
            self.update_frame(
                frame_rtp_timestamp,
                FrameReceiveState::Repairing,
                now_ms,
                None,
                frame_importance,
                None,
            );
        }
        if soft_reentry
            && matches!(
                self.chain_state,
                ChainState::Healthy | ChainState::Repairing
            )
        {
            self.chain_state = ChainState::Healthy;
            return;
        }
        if matches!(self.chain_state, ChainState::Healthy) {
            self.reset_stable_recovery_gate();
            self.chain_state = ChainState::Repairing;
        }
    }

    pub(super) fn mark_gap_repair_in_flight(
        &mut self,
        sequences: &[u16],
        now_ms: f64,
        frame_rtp_timestamp: Option<u32>,
        frame_importance: &'static str,
    ) {
        let soft_reentry = self.try_consume_soft_reentry_budget(now_ms, frame_importance);
        for sequence in sequences {
            self.update_gap(
                *sequence,
                GapState::RepairInFlight,
                now_ms,
                frame_rtp_timestamp,
                frame_importance,
                None,
            );
        }
        if let Some(frame_rtp_timestamp) = frame_rtp_timestamp {
            self.update_frame(
                frame_rtp_timestamp,
                FrameReceiveState::Repairing,
                now_ms,
                None,
                frame_importance,
                None,
            );
        }
        if soft_reentry
            && matches!(
                self.chain_state,
                ChainState::Healthy | ChainState::Repairing
            )
        {
            self.chain_state = ChainState::Healthy;
            return;
        }
        if matches!(self.chain_state, ChainState::Healthy) {
            self.reset_stable_recovery_gate();
            self.chain_state = ChainState::Repairing;
        }
    }

    pub(super) fn mark_gap_resolved(
        &mut self,
        sequence: u16,
        now_ms: f64,
        frame_rtp_timestamp: Option<u32>,
        frame_importance: &'static str,
    ) {
        let soft_reentry = self.try_consume_soft_reentry_budget(now_ms, frame_importance);
        self.update_gap(
            sequence,
            GapState::Resolved,
            now_ms,
            frame_rtp_timestamp,
            frame_importance,
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
                frame_importance,
                None,
            );
            if !has_pending_gap {
                if self.waiting_for_recovery_keyframe()
                    || self.has_unrecoverable_frame_or_chain_debt()
                {
                    if soft_reentry {
                        self.chain_state = ChainState::Healthy;
                    } else {
                        self.chain_state = ChainState::Recovering;
                    }
                } else if !matches!(
                    self.chain_state,
                    ChainState::Broken | ChainState::Recovering
                ) {
                    // gap-resolved 只意味着 supply debt 减轻，不能直接把链路回白为 healthy。
                    self.chain_state = if soft_reentry {
                        ChainState::Healthy
                    } else {
                        ChainState::Repairing
                    };
                }
            }
        }
    }

    pub(super) fn mark_gap_expired(
        &mut self,
        sequences: &[u16],
        now_ms: f64,
        frame_rtp_timestamp: Option<u32>,
        frame_importance: &'static str,
        close_reason: Option<&'static str>,
    ) -> bool {
        let soft_reentry =
            self.can_soften_expired_delta_reentry(now_ms, frame_importance, close_reason);
        for sequence in sequences {
            self.update_gap(
                *sequence,
                GapState::Expired,
                now_ms,
                frame_rtp_timestamp,
                frame_importance,
                close_reason,
            );
        }
        if let Some(frame_rtp_timestamp) = frame_rtp_timestamp {
            self.update_frame(
                frame_rtp_timestamp,
                FrameReceiveState::Closed,
                now_ms,
                None,
                frame_importance,
                close_reason,
            );
        }
        let chain_broken = self.should_expired_gap_break_chain(
            frame_rtp_timestamp,
            frame_importance,
            close_reason,
            soft_reentry,
        );
        if chain_broken {
            self.has_chain_debt = true;
            self.chain_debt_reason = Some(
                self.expired_gap_chain_break_reason(
                    frame_rtp_timestamp,
                    frame_importance,
                    close_reason,
                )
                .to_string(),
            );
            self.reset_stable_recovery_gate();
            self.chain_state = ChainState::Broken;
        } else if (matches!(
            close_reason,
            Some("cloudHighRttLowValueAdmission" | "displayStarvedLowValueAdmission")
        ) || soft_reentry)
            && !self.has_pending_gap_risk()
            && !matches!(
                self.chain_state,
                ChainState::Broken | ChainState::Recovering
            )
        {
            // 低价值 delta 失包只应降级为局部 repair，不应直接把链路打碎。
            self.chain_state = ChainState::Healthy;
        }
        chain_broken
    }

    pub(super) fn observe_frame(
        &mut self,
        frame_rtp_timestamp: u32,
        now_ms: f64,
        is_keyframe: Option<bool>,
        frame_importance: &'static str,
    ) {
        // 一旦有新帧进入，timeout 原因仅作为“最近一次”观测信息，不应持续粘住后续链路原因。
        self.timeout_reason = None;
        if matches!(self.chain_state, ChainState::Stalled) {
            self.reset_stable_recovery_gate();
            self.chain_state = ChainState::Repairing;
        }
        if matches!(
            self.chain_state,
            ChainState::Repairing | ChainState::Recovering | ChainState::Stalled
        ) && self.stable_recovery_started_at_ms.is_none()
        {
            self.stable_recovery_started_at_ms = Some(now_ms);
        }
        self.update_frame(
            frame_rtp_timestamp,
            FrameReceiveState::Open,
            now_ms,
            is_keyframe,
            frame_importance,
            None,
        );
    }

    pub(super) fn record_timeout_reason(&mut self, reason: &'static str) {
        self.timeout_reason = Some(reason);
    }

    pub(super) fn observe_anchor_candidate(
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
                    (candidate.recovery_epoch == recovery_epoch)
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

    pub(super) fn latest_anchor_candidate_ledger(&self) -> Option<XbxEngineAnchorCandidateLedger> {
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

    pub(super) fn mark_frame_closed(
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
            close_reason,
        );
    }

    pub(super) fn mark_frame_complete_candidate(
        &mut self,
        frame_rtp_timestamp: u32,
        now_ms: f64,
        _is_keyframe: Option<bool>,
        _frame_importance: &'static str,
    ) {
        self.update_frame(
            frame_rtp_timestamp,
            FrameReceiveState::CompleteCandidate,
            now_ms,
            _is_keyframe,
            _frame_importance,
            None,
        );
        if matches!(
            self.chain_state,
            ChainState::Broken | ChainState::Recovering
        ) {
            // recovering/broken 链路上，普通 complete-candidate 不能洗白 debt。
            return;
        }
        if matches!(
            self.chain_state,
            ChainState::Repairing | ChainState::Stalled
        ) && !self.has_pending_gap_risk()
            && !self.has_unrecoverable_frame_or_chain_debt()
            && self.passes_stable_recovery_gate(frame_rtp_timestamp, now_ms)
        {
            self.chain_state = ChainState::Healthy;
        }
    }

    pub(super) fn record_frame_recovery(
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
        if !matches!(
            frame_recovery_disposition,
            FrameRecoveryDisposition::Repairing
        ) {
            self.has_chain_debt = true;
            self.chain_debt_reason = Some(
                frame_unrecoverable_reason
                    .unwrap_or("referenceChainUnrecoverable")
                    .to_string(),
            );
            if matches!(
                frame_recovery_disposition,
                FrameRecoveryDisposition::UnrecoverableReferenceChain
            ) {
                self.reset_stable_recovery_gate();
                self.chain_state = ChainState::Broken;
            }
        }
        self.prune_frame_recovery_ledger();
    }

    pub(super) fn take_frame_recovery(
        &mut self,
        frame_rtp_timestamp: u32,
    ) -> Option<FrameRecoveryLedgerEntry> {
        self.frame_recovery_ledger.remove(&frame_rtp_timestamp)
    }

    pub(super) fn snapshot_for_observation(
        &self,
        observation_id: u64,
        source_event: &str,
        gap_sequence: Option<u16>,
        frame_rtp_timestamp: Option<u32>,
        now_ms: f64,
    ) -> XbxEngineVideoTimelineObservation {
        let snapshot = self.build_snapshot(gap_sequence, frame_rtp_timestamp, now_ms);
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
    pub(super) fn gap_state_of(&self, sequence: u16) -> Option<GapState> {
        self.gaps.get(&sequence).map(|entry| entry.state)
    }

    #[cfg(test)]
    pub(super) fn frame_state_of(&self, frame_rtp_timestamp: u32) -> Option<FrameReceiveState> {
        self.frames
            .get(&frame_rtp_timestamp)
            .map(|entry| entry.state)
    }

    #[cfg(test)]
    pub(super) fn chain_state(&self) -> ChainState {
        self.chain_state
    }

    #[cfg(test)]
    pub(super) fn has_hard_recovery_risk_for_test(&self) -> bool {
        self.has_hard_recovery_gap_risk()
    }

    fn update_gap(
        &mut self,
        sequence: u16,
        state: GapState,
        now_ms: f64,
        frame_rtp_timestamp: Option<u32>,
        frame_importance: &'static str,
        close_reason: Option<&'static str>,
    ) {
        let (provenance, severity) =
            classify_gap(state, frame_importance, frame_rtp_timestamp, close_reason);
        if let Some(entry) = self.gaps.get_mut(&sequence) {
            entry.state = state;
            entry.last_updated_at_ms = now_ms;
            entry.frame_rtp_timestamp = entry.frame_rtp_timestamp.or(frame_rtp_timestamp);
            if entry.frame_importance == "unknown" && frame_importance != "unknown" {
                entry.frame_importance = frame_importance;
            }
            entry.provenance = provenance;
            entry.severity = severity;
            return;
        }
        self.gaps.insert(
            sequence,
            GapEntry {
                state,
                frame_rtp_timestamp,
                frame_importance,
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
        frame_importance: &'static str,
        close_reason: Option<&'static str>,
    ) {
        if let Some(entry) = self.frames.get_mut(&frame_rtp_timestamp) {
            entry.state = state;
            entry.last_updated_at_ms = now_ms;
            entry.is_keyframe = entry.is_keyframe.or(is_keyframe);
            if entry.frame_importance == "unknown" && frame_importance != "unknown" {
                entry.frame_importance = frame_importance;
            }
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
                frame_importance,
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
                state: self.chain_state.as_str().to_string(),
                reason: self.chain_reason(frame_rtp_timestamp),
                observed_at_ms: now_ms,
            },
        }
    }

    fn has_pending_gap_risk(&self) -> bool {
        self.gaps
            .values()
            .any(|entry| !matches!(entry.state, GapState::Resolved | GapState::Expired))
    }

    fn has_unrecoverable_frame_or_chain_debt(&self) -> bool {
        self.has_chain_debt
            || self.frame_recovery_ledger.values().any(|entry| {
                !matches!(
                    entry.frame_recovery_disposition,
                    FrameRecoveryDisposition::Repairing
                )
            })
    }

    fn has_unresolved_hard_gap_issue(&self) -> bool {
        if self.gaps.values().any(|entry| {
            entry.severity == GapSeverity::Hard
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

    fn should_expired_gap_break_chain(
        &self,
        frame_rtp_timestamp: Option<u32>,
        frame_importance: &str,
        close_reason: Option<&'static str>,
        soft_reentry: bool,
    ) -> bool {
        if matches!(frame_importance, "reference" | "keyframe") {
            return true;
        }
        if matches!(close_reason, Some(reason) if is_local_low_value_gap_reason(reason)) {
            return false;
        }
        if soft_reentry {
            return false;
        }
        frame_rtp_timestamp.is_none()
            && frame_importance == "delta"
            && matches!(close_reason, Some("awaitingRecoveryKeyframe"))
    }

    fn expired_gap_chain_break_reason(
        &self,
        frame_rtp_timestamp: Option<u32>,
        frame_importance: &str,
        close_reason: Option<&'static str>,
    ) -> &'static str {
        if frame_rtp_timestamp.is_none() && frame_importance == "delta" {
            if let Some(reason @ "awaitingRecoveryKeyframe") = close_reason {
                return reason;
            }
        }
        "referenceChainUnrecoverable"
    }

    fn clear_chain_debt(&mut self) {
        self.has_chain_debt = false;
        self.chain_debt_reason = None;
        self.frame_recovery_ledger.retain(|_, entry| {
            matches!(
                entry.frame_recovery_disposition,
                FrameRecoveryDisposition::Repairing
            )
        });
    }

    fn reset_stable_recovery_gate(&mut self) {
        self.stable_recovery_started_at_ms = None;
        self.stable_recovery_clean_frame_streak = 0;
        self.stable_recovery_last_frame_rtp_timestamp = None;
    }

    fn arm_soft_reentry_protection_window(&mut self) {
        let Some(candidate) = self.latest_anchor_candidate.as_ref() else {
            self.soft_reentry_protection_until_ms = None;
            self.soft_reentry_budget_remaining = 0;
            return;
        };
        if candidate.state != XbxEngineAnchorCandidateState::SubmittedCleanAnchor
            || candidate.source_event != "chain-clean-keyframe-submitted"
        {
            self.soft_reentry_protection_until_ms = None;
            self.soft_reentry_budget_remaining = 0;
            return;
        }
        self.soft_reentry_protection_until_ms =
            Some(candidate.observed_at_ms + CLEAN_ANCHOR_SOFT_REENTRY_WINDOW_MS);
        self.soft_reentry_budget_remaining = CLEAN_ANCHOR_SOFT_REENTRY_BUDGET;
    }

    // 供 source 层在 clean anchor 后的短窗内消费，避免 transport-level WaitKeyframe 反复抖动。
    pub(super) fn try_consume_soft_reentry_budget(
        &mut self,
        now_ms: f64,
        frame_importance: &'static str,
    ) -> bool {
        if frame_importance != "delta" {
            return false;
        }
        self.refresh_soft_reentry_protection(now_ms);
        if self.soft_reentry_budget_remaining == 0 {
            return false;
        }
        self.soft_reentry_budget_remaining = self.soft_reentry_budget_remaining.saturating_sub(1);
        true
    }

    fn can_soften_expired_delta_reentry(
        &mut self,
        now_ms: f64,
        frame_importance: &'static str,
        close_reason: Option<&'static str>,
    ) -> bool {
        if !matches!(close_reason, Some("awaitingRecoveryKeyframe")) {
            return false;
        }
        self.try_consume_soft_reentry_budget(now_ms, frame_importance)
    }

    fn refresh_soft_reentry_protection(&mut self, now_ms: f64) {
        let Some(until_ms) = self.soft_reentry_protection_until_ms else {
            self.soft_reentry_budget_remaining = 0;
            return;
        };
        if now_ms > until_ms {
            self.soft_reentry_protection_until_ms = None;
            self.soft_reentry_budget_remaining = 0;
        }
    }

    fn passes_stable_recovery_gate(&mut self, frame_rtp_timestamp: u32, now_ms: f64) -> bool {
        let started_at = self.stable_recovery_started_at_ms.get_or_insert(now_ms);
        let last_frame = self.stable_recovery_last_frame_rtp_timestamp;
        if last_frame != Some(frame_rtp_timestamp) {
            if last_frame.is_none_or(|last| frame_rtp_timestamp > last) {
                self.stable_recovery_clean_frame_streak =
                    self.stable_recovery_clean_frame_streak.saturating_add(1);
            }
            self.stable_recovery_last_frame_rtp_timestamp = Some(frame_rtp_timestamp);
        }
        let stable_window_elapsed = now_ms - *started_at >= RECOVERY_STABLE_MIN_WINDOW_MS;
        stable_window_elapsed
            && self.stable_recovery_clean_frame_streak >= RECOVERY_STABLE_MIN_CLEAN_FRAMES
    }

    fn resolve_gap_snapshot(
        &self,
        gap_sequence: Option<u16>,
        frame_rtp_timestamp: Option<u32>,
        now_ms: f64,
    ) -> Option<XbxEngineVideoTimelineGapSnapshot> {
        if let Some(sequence) = gap_sequence {
            if let Some(entry) = self.gaps.get(&sequence) {
                return Some(XbxEngineVideoTimelineGapSnapshot {
                    state: entry.state.as_str().to_string(),
                    sequence: Some(sequence),
                    frame_rtp_timestamp: entry.frame_rtp_timestamp,
                    frame_importance: Some(entry.frame_importance.to_string()),
                    observed_at_ms: entry.last_updated_at_ms.max(now_ms),
                });
            }
        }
        let candidate = frame_rtp_timestamp
            .and_then(|frame_ts| {
                self.gaps
                    .iter()
                    .find(|(_, entry)| entry.frame_rtp_timestamp == Some(frame_ts))
            })
            .or_else(|| self.gaps.last_key_value());
        candidate.map(|(sequence, entry)| XbxEngineVideoTimelineGapSnapshot {
            state: entry.state.as_str().to_string(),
            sequence: Some(*sequence),
            frame_rtp_timestamp: entry.frame_rtp_timestamp,
            frame_importance: Some(entry.frame_importance.to_string()),
            observed_at_ms: entry.last_updated_at_ms.max(now_ms),
        })
    }

    fn resolve_frame_snapshot(
        &self,
        frame_rtp_timestamp: Option<u32>,
        now_ms: f64,
    ) -> Option<XbxEngineVideoTimelineFrameSnapshot> {
        let candidate = frame_rtp_timestamp
            .and_then(|frame_ts| self.frames.get_key_value(&frame_ts))
            .or_else(|| self.frames.last_key_value());
        candidate.map(|(frame_ts, entry)| XbxEngineVideoTimelineFrameSnapshot {
            state: entry.state.as_str().to_string(),
            frame_rtp_timestamp: Some(*frame_ts),
            is_keyframe: entry.is_keyframe,
            frame_importance: Some(entry.frame_importance.to_string()),
            close_reason: entry.close_reason.map(str::to_string),
            observed_at_ms: entry.last_updated_at_ms.max(now_ms),
        })
    }

    fn chain_reason(&self, frame_rtp_timestamp: Option<u32>) -> Option<String> {
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
        if let Some(reason) = self.chain_debt_reason.as_ref() {
            return Some(reason.clone());
        }
        match self.chain_state {
            ChainState::Broken => Some("referenceChainUnrecoverable".to_string()),
            ChainState::Recovering => Some("awaitRecoveryKeyframe".to_string()),
            ChainState::Repairing => Some("gapRepairInFlight".to_string()),
            ChainState::Stalled => Some("streamStalled".to_string()),
            ChainState::Healthy => None,
        }
    }
}

fn classify_gap(
    state: GapState,
    frame_importance: &'static str,
    frame_rtp_timestamp: Option<u32>,
    close_reason: Option<&'static str>,
) -> (GapProvenance, GapSeverity) {
    if matches!(state, GapState::RepairInFlight) {
        return (
            GapProvenance::Repair,
            if matches!(frame_importance, "reference" | "keyframe") {
                GapSeverity::Hard
            } else {
                GapSeverity::Soft
            },
        );
    }
    if matches!(close_reason, Some(reason) if is_local_low_value_gap_reason(reason)) {
        return (GapProvenance::LocalLowValueDrop, GapSeverity::Soft);
    }
    (
        GapProvenance::NetworkOrUnknown,
        if matches!(frame_importance, "reference" | "keyframe")
            || (frame_rtp_timestamp.is_none()
                && frame_importance == "delta"
                && matches!(close_reason, Some("awaitingRecoveryKeyframe")))
        {
            GapSeverity::Hard
        } else {
            GapSeverity::Soft
        },
    )
}

fn is_local_low_value_gap_reason(reason: &str) -> bool {
    matches!(
        reason,
        "cloudHighRttLowValueAdmission"
            | "localBackpressureDeltaGap"
            | "displayStarvedLowValueAdmission"
    )
}

fn is_hard_recovery_reason(reason: &str) -> bool {
    !is_local_low_value_gap_reason(reason)
        && matches!(
            reason,
            "awaitingRecoveryKeyframe"
                | "awaitRecoveryKeyframe"
                | "referenceChainUnrecoverable"
                | "inspectionError"
                | "inspectionRejectNoVcl"
                | "bootstrapMissingSps"
                | "bootstrapMissingPps"
                | "inspectionRejectNonIdrVcl"
                | "inspectionRejectInvalidSliceHeader"
        )
}

#[cfg(test)]
#[path = "timeline.test.rs"]
mod tests;
