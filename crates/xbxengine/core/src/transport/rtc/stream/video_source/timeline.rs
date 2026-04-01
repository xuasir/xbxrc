use std::collections::BTreeMap;

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
}

#[derive(Clone, Debug)]
struct GapEntry {
    state: GapState,
    frame_rtp_timestamp: Option<u32>,
    frame_importance: &'static str,
    first_observed_at_ms: f64,
    last_updated_at_ms: f64,
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
        } else if (matches!(close_reason, Some("cloudHighRttLowValueAdmission")) || soft_reentry)
            && !self.has_pending_gap_risk()
            && !matches!(
                self.chain_state,
                ChainState::Broken | ChainState::Recovering
            )
        {
            // Cloud 高 RTT 下的低价值 delta 失包只应降级为局部 repair，不应直接把链路打碎。
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
    ) {
        let next_entry = FrameRecoveryLedgerEntry {
            frame_playout_deadline_at_ms,
            frame_recovery_disposition,
            frame_unrecoverable_reason: frame_unrecoverable_reason.map(str::to_string),
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

    fn update_gap(
        &mut self,
        sequence: u16,
        state: GapState,
        now_ms: f64,
        frame_rtp_timestamp: Option<u32>,
        frame_importance: &'static str,
    ) {
        if let Some(entry) = self.gaps.get_mut(&sequence) {
            entry.state = state;
            entry.last_updated_at_ms = now_ms;
            entry.frame_rtp_timestamp = entry.frame_rtp_timestamp.or(frame_rtp_timestamp);
            if entry.frame_importance == "unknown" && frame_importance != "unknown" {
                entry.frame_importance = frame_importance;
            }
            return;
        }
        self.gaps.insert(
            sequence,
            GapEntry {
                state,
                frame_rtp_timestamp,
                frame_importance,
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

    fn try_consume_soft_reentry_budget(
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

#[cfg(test)]
mod tests {
    use super::{ChainState, FrameReceiveState, GapState, VideoTimelineState};
    use crate::media::video::types::FrameRecoveryDisposition;
    use crate::{XbxEngineAnchorCandidateFailureReason, XbxEngineAnchorCandidateState};

    #[test]
    fn wait_keyframe_gate_moves_chain_between_recovering_and_healthy() {
        let mut state = VideoTimelineState::new();
        assert!(!state.waiting_for_recovery_keyframe());
        assert_eq!(state.chain_state(), ChainState::Healthy);
        state.apply_wait_keyframe_gate(true);
        assert!(state.waiting_for_recovery_keyframe());
        assert_eq!(state.chain_state(), ChainState::Recovering);
    }

    #[test]
    fn chain_broken_then_keyframe_request_enters_recovering() {
        let mut state = VideoTimelineState::new();
        state.on_chain_broken();
        assert_eq!(state.chain_state(), ChainState::Broken);
        state.on_recovery_keyframe_requested();
        assert_eq!(state.chain_state(), ChainState::Recovering);
        assert!(state.waiting_for_recovery_keyframe());
    }

    #[test]
    fn per_gap_lifecycle_is_tracked() {
        let mut state = VideoTimelineState::new();
        state.observe_gap(&[10, 11], 1.0, Some(90_000), "reference");
        state.mark_gap_reorder_pending(&[10, 11], 2.0, Some(90_000), "reference");
        state.mark_gap_nack_candidate(&[10], 3.0, Some(90_000), "reference");
        state.mark_gap_repair_in_flight(&[10], 4.0, Some(90_000), "reference");
        state.mark_gap_resolved(10, 5.0, Some(90_000), "reference");
        state.mark_gap_expired(&[11], 6.0, Some(90_000), "reference", Some("deadline"));
        assert_eq!(state.gap_state_of(10), Some(GapState::Resolved));
        assert_eq!(state.gap_state_of(11), Some(GapState::Expired));
        assert_eq!(
            state.frame_state_of(90_000),
            Some(FrameReceiveState::Closed)
        );
    }

    #[test]
    fn anchor_candidate_ledger_tracks_rejected_candidate() {
        let mut state = VideoTimelineState::new();
        state.observe_anchor_candidate(
            3,
            Some(91_200),
            "frame-inspection-rejected-await-keyframe",
            XbxEngineAnchorCandidateState::Rejected,
            Some(XbxEngineAnchorCandidateFailureReason::InspectionRejectedInvalidSliceHeader),
            3.0,
        );
        let ledger = state
            .latest_anchor_candidate_ledger()
            .expect("anchor candidate");
        assert_eq!(ledger.recovery_epoch, 3);
        assert_eq!(ledger.frame_rtp_timestamp, Some(91_200));
        assert_eq!(ledger.state, XbxEngineAnchorCandidateState::Rejected);
        assert_eq!(
            ledger.failure_reason,
            Some(XbxEngineAnchorCandidateFailureReason::InspectionRejectedInvalidSliceHeader)
        );
    }

    #[test]
    fn anchor_candidate_ledger_tracks_clean_anchor_submission() {
        let mut state = VideoTimelineState::new();
        state.observe_anchor_candidate(
            7,
            Some(95_001),
            "frame-complete-candidate",
            XbxEngineAnchorCandidateState::Observed,
            None,
            10.0,
        );
        state.observe_anchor_candidate(
            7,
            Some(95_001),
            "chain-clean-keyframe-submitted",
            XbxEngineAnchorCandidateState::SubmittedCleanAnchor,
            None,
            12.0,
        );
        let ledger = state
            .latest_anchor_candidate_ledger()
            .expect("anchor candidate");
        assert_eq!(ledger.recovery_epoch, 7);
        assert_eq!(
            ledger.state,
            XbxEngineAnchorCandidateState::SubmittedCleanAnchor
        );
        assert_eq!(ledger.source_event, "chain-clean-keyframe-submitted");
        assert_eq!(ledger.failure_reason, None);
    }

    #[test]
    fn anonymous_repair_candidate_inherits_latest_frame_in_same_epoch() {
        let mut state = VideoTimelineState::new();
        state.observe_anchor_candidate(
            9,
            Some(96_001),
            "frame-complete-candidate",
            XbxEngineAnchorCandidateState::Observed,
            None,
            20.0,
        );
        state.observe_anchor_candidate(
            9,
            None,
            "gap-repair-in-flight",
            XbxEngineAnchorCandidateState::Repaired,
            None,
            21.0,
        );
        let ledger = state
            .latest_anchor_candidate_ledger()
            .expect("anchor candidate");
        assert_eq!(ledger.recovery_epoch, 9);
        assert_eq!(ledger.frame_rtp_timestamp, Some(96_001));
        assert_eq!(ledger.state, XbxEngineAnchorCandidateState::Repaired);
        assert_eq!(ledger.source_event, "gap-repair-in-flight");
    }

    #[test]
    fn anchor_candidate_ledger_tracks_observed_to_awaiting_transition() {
        let mut state = VideoTimelineState::new();
        state.observe_anchor_candidate(
            11,
            Some(96_001),
            "frame-complete-candidate",
            XbxEngineAnchorCandidateState::Observed,
            None,
            20.0,
        );
        state.observe_anchor_candidate(
            11,
            Some(96_001),
            "frame-await-recovery-keyframe",
            XbxEngineAnchorCandidateState::AwaitingRecovery,
            Some(XbxEngineAnchorCandidateFailureReason::AwaitingRecoveryKeyframe),
            24.0,
        );
        let ledger = state
            .latest_anchor_candidate_ledger()
            .expect("anchor candidate");
        assert_eq!(ledger.recovery_epoch, 11);
        assert_eq!(ledger.frame_rtp_timestamp, Some(96_001));
        assert_eq!(
            ledger.state,
            XbxEngineAnchorCandidateState::AwaitingRecovery
        );
        assert_eq!(ledger.source_event, "frame-await-recovery-keyframe");
        assert_eq!(
            ledger.failure_reason,
            Some(XbxEngineAnchorCandidateFailureReason::AwaitingRecoveryKeyframe)
        );
    }

    #[test]
    fn anchor_candidate_ledger_tracks_observed_to_repaired_transition() {
        let mut state = VideoTimelineState::new();
        state.observe_anchor_candidate(
            12,
            Some(96_101),
            "frame-complete-candidate",
            XbxEngineAnchorCandidateState::Observed,
            None,
            30.0,
        );
        state.observe_anchor_candidate(
            12,
            Some(96_101),
            "gap-resolved",
            XbxEngineAnchorCandidateState::Repaired,
            None,
            36.0,
        );
        let ledger = state
            .latest_anchor_candidate_ledger()
            .expect("anchor candidate");
        assert_eq!(ledger.recovery_epoch, 12);
        assert_eq!(ledger.frame_rtp_timestamp, Some(96_101));
        assert_eq!(ledger.state, XbxEngineAnchorCandidateState::Repaired);
        assert_eq!(ledger.source_event, "gap-resolved");
        assert_eq!(ledger.failure_reason, None);
    }

    #[test]
    fn anchor_candidate_ledger_tracks_observed_to_rejected_transition() {
        let mut state = VideoTimelineState::new();
        state.observe_anchor_candidate(
            13,
            Some(96_201),
            "frame-complete-candidate",
            XbxEngineAnchorCandidateState::Observed,
            None,
            40.0,
        );
        state.observe_anchor_candidate(
            13,
            Some(96_201),
            "gap-expired-skipped",
            XbxEngineAnchorCandidateState::Rejected,
            Some(XbxEngineAnchorCandidateFailureReason::GapExpiredDeadline),
            47.0,
        );
        let ledger = state
            .latest_anchor_candidate_ledger()
            .expect("anchor candidate");
        assert_eq!(ledger.recovery_epoch, 13);
        assert_eq!(ledger.frame_rtp_timestamp, Some(96_201));
        assert_eq!(ledger.state, XbxEngineAnchorCandidateState::Rejected);
        assert_eq!(ledger.source_event, "gap-expired-skipped");
        assert_eq!(
            ledger.failure_reason,
            Some(XbxEngineAnchorCandidateFailureReason::GapExpiredDeadline)
        );
    }

    #[test]
    fn frame_recovery_ledger_prefers_reference_chain_failure() {
        let mut state = VideoTimelineState::new();
        state.record_frame_recovery(
            90_000,
            Some(100.0),
            FrameRecoveryDisposition::UnrecoverableLate,
            Some("late"),
        );
        state.record_frame_recovery(
            90_000,
            None,
            FrameRecoveryDisposition::UnrecoverableReferenceChain,
            Some("chain"),
        );
        let entry = state.take_frame_recovery(90_000).expect("entry");
        assert_eq!(
            entry.frame_recovery_disposition,
            FrameRecoveryDisposition::UnrecoverableReferenceChain
        );
        assert_eq!(entry.frame_unrecoverable_reason.as_deref(), Some("chain"));
        assert_eq!(entry.frame_playout_deadline_at_ms, Some(100.0));
    }

    #[test]
    fn timeout_reason_is_exposed_via_chain_reason_when_no_frame_close_reason() {
        let mut state = VideoTimelineState::new();
        state.record_timeout_reason("streamIdleTimeout");
        let observation = state.snapshot_for_observation(1, "timeout-stream-idle", None, None, 1.0);
        assert_eq!(
            observation.chain.reason.as_deref(),
            Some("streamIdleTimeout")
        );
    }

    #[test]
    fn timeout_reason_is_cleared_after_new_frame_observed() {
        let mut state = VideoTimelineState::new();
        state.record_timeout_reason("streamThinStall");
        let before =
            state.snapshot_for_observation(1, "timeout-stream-thin-stall", None, None, 1.0);
        assert_eq!(before.chain.reason.as_deref(), Some("streamThinStall"));

        state.observe_frame(90_001, 2.0, Some(false), "delta");
        let after = state.snapshot_for_observation(2, "frame-observed", None, Some(90_001), 2.0);
        assert_eq!(after.chain.reason.as_deref(), None);
    }

    #[test]
    fn timeout_detected_sets_stalled_chain_state() {
        let mut state = VideoTimelineState::new();
        state.on_timeout_detected();
        assert_eq!(state.chain_state(), ChainState::Stalled);
        let observation = state.snapshot_for_observation(1, "timeout-stream-idle", None, None, 1.0);
        assert_eq!(observation.chain.state, "stalled");
    }

    #[test]
    fn frame_observed_after_timeout_moves_stalled_to_repairing_then_healthy() {
        let mut state = VideoTimelineState::new();
        state.on_timeout_detected();
        state.observe_frame(90_001, 2.0, Some(false), "delta");
        assert_eq!(state.chain_state(), ChainState::Repairing);
        state.mark_frame_complete_candidate(90_001, 3.0, Some(false), "delta");
        assert_eq!(state.chain_state(), ChainState::Repairing);
        state.observe_frame(90_002, 130.0, Some(false), "delta");
        state.mark_frame_complete_candidate(90_002, 140.0, Some(false), "delta");
        assert_eq!(state.chain_state(), ChainState::Healthy);
    }

    #[test]
    fn single_complete_candidate_does_not_whiten_recovering_chain_without_stable_window() {
        let mut state = VideoTimelineState::new();
        state.on_timeout_detected();
        state.observe_frame(92_001, 5.0, Some(false), "delta");
        state.mark_frame_complete_candidate(92_001, 10.0, Some(false), "delta");
        assert_eq!(state.chain_state(), ChainState::Repairing);
    }

    #[test]
    fn recovering_chain_requires_stable_clean_frames_before_healthy() {
        let mut state = VideoTimelineState::new();
        state.on_timeout_detected();

        state.observe_frame(93_001, 10.0, Some(false), "delta");
        state.mark_frame_complete_candidate(93_001, 20.0, Some(false), "delta");
        assert_eq!(state.chain_state(), ChainState::Repairing);

        state.observe_frame(93_002, 121.0, Some(false), "delta");
        state.mark_frame_complete_candidate(93_002, 132.0, Some(false), "delta");
        assert_eq!(state.chain_state(), ChainState::Healthy);
    }

    #[test]
    fn timeout_does_not_override_recovering_chain() {
        let mut state = VideoTimelineState::new();
        state.apply_wait_keyframe_gate(true);
        assert_eq!(state.chain_state(), ChainState::Recovering);
        state.on_timeout_detected();
        assert_eq!(state.chain_state(), ChainState::Recovering);
    }

    #[test]
    fn expired_reference_gap_does_not_coexist_with_healthy_chain() {
        let mut state = VideoTimelineState::new();
        state.observe_gap(&[11], 1.0, Some(90_000), "reference");
        assert_eq!(state.chain_state(), ChainState::Repairing);
        state.mark_gap_expired(&[11], 2.0, Some(90_000), "reference", Some("deadline"));
        assert_eq!(state.gap_state_of(11), Some(GapState::Expired));
        assert_eq!(state.chain_state(), ChainState::Broken);

        state.mark_frame_complete_candidate(90_001, 3.0, Some(false), "delta");
        assert_eq!(state.chain_state(), ChainState::Broken);
    }

    #[test]
    fn gap_resolved_does_not_whiten_chain_without_stable_completion() {
        let mut state = VideoTimelineState::new();
        state.observe_gap(&[41], 1.0, Some(99_000), "reference");
        assert_eq!(state.chain_state(), ChainState::Repairing);

        state.mark_gap_repair_in_flight(&[41], 2.0, Some(99_000), "reference");
        assert_eq!(state.chain_state(), ChainState::Repairing);
        state.mark_gap_resolved(41, 3.0, Some(99_000), "reference");
        assert_eq!(state.chain_state(), ChainState::Repairing);

        state.observe_frame(99_001, 20.0, Some(false), "delta");
        state.mark_frame_complete_candidate(99_001, 30.0, Some(false), "delta");
        assert_eq!(state.chain_state(), ChainState::Repairing);

        state.observe_frame(99_002, 160.0, Some(false), "delta");
        state.mark_frame_complete_candidate(99_002, 170.0, Some(false), "delta");
        assert_eq!(state.chain_state(), ChainState::Healthy);
    }

    #[test]
    fn broken_chain_is_not_whitened_by_delta_until_clean_keyframe() {
        let mut state = VideoTimelineState::new();
        state.observe_gap(&[21], 1.0, Some(91_000), "reference");
        state.mark_gap_expired(&[21], 2.0, Some(91_000), "reference", Some("deadline"));
        assert_eq!(state.chain_state(), ChainState::Broken);

        state.mark_frame_complete_candidate(91_001, 3.0, Some(false), "delta");
        assert_eq!(state.chain_state(), ChainState::Broken);

        state.mark_frame_complete_candidate(91_002, 4.0, Some(true), "keyframe");
        assert_eq!(state.chain_state(), ChainState::Broken);

        state.on_clean_keyframe_submitted();
        assert_eq!(state.chain_state(), ChainState::Healthy);
    }

    #[test]
    fn anonymous_cloud_low_value_delta_gap_does_not_break_chain() {
        let mut state = VideoTimelineState::new();
        let chain_broken = state.mark_gap_expired(
            &[38022],
            2.0,
            None,
            "delta",
            Some("cloudHighRttLowValueAdmission"),
        );
        assert!(!chain_broken);
        assert_eq!(state.chain_state(), ChainState::Healthy);

        state.observe_frame(91_100, 3.0, Some(false), "delta");
        state.mark_frame_complete_candidate(91_100, 4.0, Some(false), "delta");
        assert_eq!(state.chain_state(), ChainState::Healthy);
        let observation = state.snapshot_for_observation(
            1,
            "frame-complete-candidate",
            Some(38022),
            Some(91_100),
            4.0,
        );
        assert_eq!(observation.chain.state, "healthy");
        assert_eq!(observation.chain.reason.as_deref(), None);

        state.mark_frame_complete_candidate(91_101, 5.0, Some(true), "keyframe");
        assert_eq!(state.chain_state(), ChainState::Healthy);

        state.on_clean_keyframe_submitted();
        assert_eq!(state.chain_state(), ChainState::Healthy);
    }

    #[test]
    fn anonymous_delta_gap_low_value_admission_keeps_chain_healthy() {
        let mut state = VideoTimelineState::new();
        state.mark_gap_expired(
            &[38022],
            2.0,
            None,
            "delta",
            Some("cloudHighRttLowValueAdmission"),
        );

        state.observe_frame(91_100, 3.0, Some(false), "delta");
        let observation =
            state.snapshot_for_observation(1, "frame-observed", Some(38022), Some(91_100), 3.0);
        assert_eq!(observation.chain.state, "healthy");
        assert_eq!(observation.chain.reason.as_deref(), None);
    }

    #[test]
    fn inspection_reject_reason_projects_through_frame_and_chain_snapshot() {
        let mut state = VideoTimelineState::new();
        state.on_admission_await_recovery_keyframe(Some("inspectionRejectInvalidSliceHeader"));
        state.observe_frame(91_200, 3.0, None, "unknown");
        state.mark_frame_closed(
            91_200,
            3.0,
            None,
            "unknown",
            Some("inspectionRejectInvalidSliceHeader"),
        );

        let observation = state.snapshot_for_observation(
            1,
            "frame-inspection-rejected-await-keyframe",
            None,
            Some(91_200),
            3.0,
        );
        assert_eq!(observation.chain.state, "recovering");
        assert_eq!(
            observation.chain.reason.as_deref(),
            Some("inspectionRejectInvalidSliceHeader")
        );
        assert_eq!(
            observation
                .frame
                .as_ref()
                .and_then(|frame| frame.close_reason.as_deref()),
            Some("inspectionRejectInvalidSliceHeader")
        );
        assert_eq!(
            observation.frame.as_ref().map(|frame| frame.state.as_str()),
            Some("closed")
        );
    }

    #[test]
    fn wait_keyframe_gate_creates_chain_debt_until_clean_keyframe() {
        let mut state = VideoTimelineState::new();
        state.apply_wait_keyframe_gate(true);
        assert_eq!(state.chain_state(), ChainState::Recovering);

        state.observe_frame(90_010, 2.0, Some(false), "delta");
        state.mark_frame_complete_candidate(90_010, 3.0, Some(false), "delta");
        assert_eq!(state.chain_state(), ChainState::Recovering);

        state.apply_wait_keyframe_gate(false);
        assert_eq!(state.chain_state(), ChainState::Recovering);

        state.observe_frame(90_011, 4.0, Some(true), "keyframe");
        state.mark_frame_complete_candidate(90_011, 5.0, Some(true), "keyframe");
        assert_eq!(state.chain_state(), ChainState::Recovering);

        state.on_clean_keyframe_submitted();
        assert_eq!(state.chain_state(), ChainState::Healthy);
    }

    #[test]
    fn recovering_chain_multiple_complete_candidates_cannot_whiten_without_clean_anchor() {
        let mut state = VideoTimelineState::new();
        state.on_admission_await_recovery_keyframe(Some("awaitingRecoveryKeyframe"));
        assert_eq!(state.chain_state(), ChainState::Recovering);

        state.observe_frame(95_001, 2.0, Some(false), "delta");
        state.mark_frame_complete_candidate(95_001, 3.0, Some(false), "delta");
        assert_eq!(state.chain_state(), ChainState::Recovering);

        state.observe_frame(95_002, 130.0, Some(false), "delta");
        state.mark_frame_complete_candidate(95_002, 132.0, Some(false), "delta");
        assert_eq!(state.chain_state(), ChainState::Recovering);
    }

    #[test]
    fn recovering_chain_only_clean_anchor_submission_can_return_healthy() {
        let mut state = VideoTimelineState::new();
        state.on_admission_await_recovery_keyframe(Some("awaitingRecoveryKeyframe"));
        state.observe_frame(96_001, 2.0, Some(true), "keyframe");
        state.mark_frame_complete_candidate(96_001, 3.0, Some(true), "keyframe");
        assert_eq!(state.chain_state(), ChainState::Recovering);

        state.on_clean_keyframe_submitted();
        assert_eq!(state.chain_state(), ChainState::Healthy);
    }

    #[test]
    fn expired_and_awaiting_debt_are_not_cleared_by_complete_candidate() {
        let mut broken = VideoTimelineState::new();
        broken.mark_gap_expired(&[31], 1.0, Some(97_000), "reference", Some("deadline"));
        assert_eq!(broken.chain_state(), ChainState::Broken);
        broken.observe_frame(97_001, 2.0, Some(false), "delta");
        broken.mark_frame_complete_candidate(97_001, 3.0, Some(false), "delta");
        assert_eq!(broken.chain_state(), ChainState::Broken);

        let mut awaiting = VideoTimelineState::new();
        awaiting.on_admission_await_recovery_keyframe(Some("awaitingRecoveryKeyframe"));
        assert_eq!(awaiting.chain_state(), ChainState::Recovering);
        awaiting.observe_frame(98_001, 2.0, Some(false), "delta");
        awaiting.mark_frame_complete_candidate(98_001, 3.0, Some(false), "delta");
        assert_eq!(awaiting.chain_state(), ChainState::Recovering);
    }

    #[test]
    fn clean_anchor_short_window_softens_delta_reorder_reentry() {
        let mut state = VideoTimelineState::new();
        state.observe_anchor_candidate(
            21,
            Some(110_001),
            "chain-clean-keyframe-submitted",
            XbxEngineAnchorCandidateState::SubmittedCleanAnchor,
            None,
            10.0,
        );
        state.on_clean_keyframe_submitted();
        assert_eq!(state.chain_state(), ChainState::Healthy);

        state.mark_gap_reorder_pending(&[501], 10.5, None, "delta");
        assert_eq!(state.chain_state(), ChainState::Healthy);

        let chain_broken = state.mark_gap_expired(
            &[501],
            10.6,
            None,
            "delta",
            Some("awaitingRecoveryKeyframe"),
        );
        assert!(!chain_broken);
        assert_eq!(state.chain_state(), ChainState::Healthy);
    }

    #[test]
    fn clean_anchor_soft_reentry_budget_exhaustion_restores_hard_semantics() {
        let mut state = VideoTimelineState::new();
        state.observe_anchor_candidate(
            22,
            Some(120_001),
            "chain-clean-keyframe-submitted",
            XbxEngineAnchorCandidateState::SubmittedCleanAnchor,
            None,
            20.0,
        );
        state.on_clean_keyframe_submitted();
        assert_eq!(state.chain_state(), ChainState::Healthy);

        for sequence in [601u16, 602, 603] {
            let chain_broken = state.mark_gap_expired(
                &[sequence],
                20.1,
                None,
                "delta",
                Some("awaitingRecoveryKeyframe"),
            );
            assert!(!chain_broken);
            assert_eq!(state.chain_state(), ChainState::Healthy);
        }

        let chain_broken = state.mark_gap_expired(
            &[604],
            20.2,
            None,
            "delta",
            Some("awaitingRecoveryKeyframe"),
        );
        assert!(chain_broken);
        assert_eq!(state.chain_state(), ChainState::Broken);
    }

    #[test]
    fn clean_anchor_soft_reentry_does_not_relax_reference_break() {
        let mut state = VideoTimelineState::new();
        state.observe_anchor_candidate(
            23,
            Some(130_001),
            "chain-clean-keyframe-submitted",
            XbxEngineAnchorCandidateState::SubmittedCleanAnchor,
            None,
            30.0,
        );
        state.on_clean_keyframe_submitted();
        assert_eq!(state.chain_state(), ChainState::Healthy);

        let chain_broken = state.mark_gap_expired(
            &[701],
            30.1,
            Some(130_050),
            "reference",
            Some("awaitingRecoveryKeyframe"),
        );
        assert!(chain_broken);
        assert_eq!(state.chain_state(), ChainState::Broken);
    }
}
