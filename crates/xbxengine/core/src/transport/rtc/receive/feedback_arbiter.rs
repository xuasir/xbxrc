//! Receive 层单一 feedback 仲裁：NACK / PLI / FIR 优先级与 coalescing 预测。

use crate::transport::rtc::receive::recovery_ledger::KeyframeRequiredCause;
use crate::transport::rtc::recovery::contract::{ReferenceChainState, SparseIdrRhythm};

use super::insert_gate::InsertDecision;

/// 本 tick 拟执行的 RTCP feedback 动作。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReceiveFeedbackAction {
    None,
    SendNack,
    RequestPli,
    RequestFir,
}

impl ReceiveFeedbackAction {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::SendNack => "sendNack",
            Self::RequestPli => "requestPli",
            Self::RequestFir => "requestFir",
        }
    }
}

/// coalescing / 节流 / 目标不可用 投影标签。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReceiveFeedbackCoalescing {
    FreshSent,
    SameInterval,
    RateLimited,
    TargetUnavailable,
    NotApplicable,
}

impl ReceiveFeedbackCoalescing {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::FreshSent => "fresh-sent",
            Self::SameInterval => "same-interval",
            Self::RateLimited => "rate-limited",
            Self::TargetUnavailable => "target-unavailable",
            Self::NotApplicable => "not-applicable",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ReceiveFeedbackDecision {
    pub action: ReceiveFeedbackAction,
    pub reason: &'static str,
    pub coalescing: ReceiveFeedbackCoalescing,
    pub sparse_active: bool,
    pub reference_state: ReferenceChainState,
    pub feedback_target_state: &'static str,
    pub gap_sequence: Option<u16>,
    pub nack_packet_count: u32,
    pub keyframe_required: bool,
    pub keyframe_required_cause: KeyframeRequiredCause,
    pub response_state: &'static str,
    pub terminal_candidate: bool,
    pub ledger_generation: u64,
}

impl ReceiveFeedbackDecision {
    fn is_keyframe_reason(reason: &'static str) -> bool {
        matches!(
            reason,
            "need-keyframe"
                | "forced-keyframe"
                | "sparse-idr"
                | "gap-too-large"
                | "feedback-target-unavailable"
                | "gap-repair"
        )
    }

    /// 是否应走 KeyframeRequester 并写入 `keyframeRequestOutcome`（含 coalesced/throttled）。
    pub(crate) fn should_touch_keyframe_executor(self) -> bool {
        matches!(
            self.action,
            ReceiveFeedbackAction::RequestPli | ReceiveFeedbackAction::RequestFir
        ) || (matches!(
            self.coalescing,
            ReceiveFeedbackCoalescing::SameInterval | ReceiveFeedbackCoalescing::RateLimited
        ) && Self::is_keyframe_reason(self.reason))
    }
}

/// NACK poll 快照（由 maintenance tick 填入）。
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct NackPollSnapshot {
    pub sequences_len: usize,
    pub keyframe_escalation_due: bool,
    pub gap_span_too_large: bool,
    pub gap_sequence: Option<u16>,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ReceiveFeedbackArbiterInput {
    pub sparse_idr: SparseIdrRhythm,
    pub nack: NackPollSnapshot,
    pub insert_decision: Option<InsertDecision>,
    pub reference_state: ReferenceChainState,
    pub feedback_target_available: bool,
    pub force_keyframe: bool,
    pub soft_keyframe: bool,
    pub consecutive_pli_without_idr: u8,
    pub fir_after_pli_count: u8,
    pub pli_coalesced: bool,
    pub pli_throttled: bool,
    pub keyframe_required: bool,
    pub keyframe_required_cause: KeyframeRequiredCause,
    pub response_state: &'static str,
    pub terminal_candidate: bool,
    pub ledger_generation: u64,
}

const NACK_SPAN_KEYFRAME_ESCALATION_PACKETS: u32 = 96;
const MAX_RECEIVER_LOCAL_NACK_BATCH: usize = 32;

fn decision_meta(
    input: &ReceiveFeedbackArbiterInput,
) -> (bool, KeyframeRequiredCause, &'static str, bool, u64) {
    (
        input.keyframe_required,
        input.keyframe_required_cause,
        input.response_state,
        input.terminal_candidate,
        input.ledger_generation,
    )
}

fn needs_keyframe(input: &ReceiveFeedbackArbiterInput) -> bool {
    input.keyframe_required
}

pub(crate) fn decide(input: &ReceiveFeedbackArbiterInput) -> ReceiveFeedbackDecision {
    let sparse_active = input.sparse_idr.active || input.keyframe_required;
    let feedback_target_state = if input.feedback_target_available {
        "ready"
    } else {
        "unavailable"
    };
    let nack_packet_count = input.nack.sequences_len.min(u32::MAX as usize) as u32;
    let (
        keyframe_required,
        keyframe_required_cause,
        response_state,
        terminal_candidate,
        ledger_generation,
    ) = decision_meta(input);

    let base_decision = |action, reason, coalescing| ReceiveFeedbackDecision {
        action,
        reason,
        coalescing,
        sparse_active,
        reference_state: input.reference_state,
        feedback_target_state,
        gap_sequence: input.nack.gap_sequence,
        nack_packet_count,
        keyframe_required,
        keyframe_required_cause,
        response_state,
        terminal_candidate,
        ledger_generation,
    };

    if !input.feedback_target_available && needs_rtcp_feedback(input) {
        return keyframe_decision(
            input,
            "feedback-target-unavailable",
            sparse_active,
            feedback_target_state,
            nack_packet_count,
        );
    }

    if input.nack.gap_span_too_large
        || input.nack.sequences_len > 128
        || (input.nack.sequences_len > MAX_RECEIVER_LOCAL_NACK_BATCH
            && gap_span_exceeds_escalation(input.nack.gap_sequence, nack_packet_count))
    {
        return keyframe_decision(
            input,
            "gap-too-large",
            sparse_active,
            feedback_target_state,
            nack_packet_count,
        );
    }

    if input.nack.keyframe_escalation_due || needs_keyframe(input) {
        return keyframe_decision(
            input,
            "need-keyframe",
            sparse_active,
            feedback_target_state,
            nack_packet_count,
        );
    }

    if input.force_keyframe {
        return keyframe_decision(
            input,
            "forced-keyframe",
            sparse_active,
            feedback_target_state,
            nack_packet_count,
        );
    }

    if sparse_active && input.sparse_idr.pli_due {
        return keyframe_decision(
            input,
            "sparse-idr",
            sparse_active,
            feedback_target_state,
            nack_packet_count,
        );
    }

    if input.soft_keyframe {
        return keyframe_decision(
            input,
            "gap-repair",
            sparse_active,
            feedback_target_state,
            nack_packet_count,
        );
    }

    if matches!(input.insert_decision, Some(InsertDecision::HoldRepair))
        && (input.force_keyframe || !input.soft_keyframe)
    {
        return keyframe_decision(
            input,
            "gap-repair",
            sparse_active,
            feedback_target_state,
            nack_packet_count,
        );
    }

    if input.nack.sequences_len > 0 && nack_still_repairable(input) {
        return base_decision(
            ReceiveFeedbackAction::SendNack,
            "gap-repair",
            ReceiveFeedbackCoalescing::NotApplicable,
        );
    }

    base_decision(
        ReceiveFeedbackAction::None,
        "none",
        ReceiveFeedbackCoalescing::NotApplicable,
    )
}

fn needs_rtcp_feedback(input: &ReceiveFeedbackArbiterInput) -> bool {
    input.nack.keyframe_escalation_due
        || input.nack.gap_span_too_large
        || input.nack.sequences_len > 0
        || input.force_keyframe
        || needs_keyframe(input)
}

fn nack_still_repairable(input: &ReceiveFeedbackArbiterInput) -> bool {
    !needs_keyframe(input) && !input.nack.keyframe_escalation_due
}

fn gap_span_exceeds_escalation(gap_sequence: Option<u16>, count: u32) -> bool {
    count >= NACK_SPAN_KEYFRAME_ESCALATION_PACKETS as u32 && gap_sequence.is_some()
}

fn keyframe_action_for_escalation(input: &ReceiveFeedbackArbiterInput) -> ReceiveFeedbackAction {
    if input.consecutive_pli_without_idr >= input.fir_after_pli_count {
        ReceiveFeedbackAction::RequestFir
    } else {
        ReceiveFeedbackAction::RequestPli
    }
}

fn keyframe_decision(
    input: &ReceiveFeedbackArbiterInput,
    reason: &'static str,
    sparse_active: bool,
    feedback_target_state: &'static str,
    nack_packet_count: u32,
) -> ReceiveFeedbackDecision {
    let coalescing = if !input.feedback_target_available {
        ReceiveFeedbackCoalescing::TargetUnavailable
    } else if input.pli_coalesced {
        ReceiveFeedbackCoalescing::SameInterval
    } else if input.pli_throttled && !input.force_keyframe {
        ReceiveFeedbackCoalescing::RateLimited
    } else {
        ReceiveFeedbackCoalescing::FreshSent
    };
    let action = if coalescing == ReceiveFeedbackCoalescing::RateLimited
        || coalescing == ReceiveFeedbackCoalescing::SameInterval
    {
        ReceiveFeedbackAction::None
    } else {
        keyframe_action_for_escalation(input)
    };
    ReceiveFeedbackDecision {
        action,
        reason,
        coalescing,
        sparse_active,
        reference_state: input.reference_state,
        feedback_target_state,
        gap_sequence: input.nack.gap_sequence,
        nack_packet_count,
        keyframe_required: input.keyframe_required,
        keyframe_required_cause: input.keyframe_required_cause,
        response_state: input.response_state,
        terminal_candidate: input.terminal_candidate,
        ledger_generation: input.ledger_generation,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::rtc::recovery::contract::PacketRecoveryActionStage;

    fn base_input() -> ReceiveFeedbackArbiterInput {
        ReceiveFeedbackArbiterInput {
            sparse_idr: SparseIdrRhythm::default(),
            nack: NackPollSnapshot::default(),
            insert_decision: None,
            reference_state: ReferenceChainState::Continuous,
            feedback_target_available: true,
            force_keyframe: false,
            soft_keyframe: false,
            consecutive_pli_without_idr: 0,
            fir_after_pli_count: 2,
            pli_coalesced: false,
            pli_throttled: false,
            keyframe_required: false,
            keyframe_required_cause: KeyframeRequiredCause::None,
            response_state: "no-packet",
            terminal_candidate: false,
            ledger_generation: 0,
        }
    }

    #[test]
    fn keyframe_priority_over_nack() {
        let mut input = base_input();
        input.nack.sequences_len = 4;
        input.nack.keyframe_escalation_due = true;
        let d = decide(&input);
        assert_eq!(d.action, ReceiveFeedbackAction::RequestPli);
        assert_eq!(d.reason, "need-keyframe");
    }

    #[test]
    fn nack_when_gap_repairable() {
        let mut input = base_input();
        input.nack.sequences_len = 2;
        let d = decide(&input);
        assert_eq!(d.action, ReceiveFeedbackAction::SendNack);
        assert_eq!(d.reason, "gap-repair");
    }

    #[test]
    fn gap_too_large_requests_keyframe() {
        let mut input = base_input();
        input.nack.gap_span_too_large = true;
        let d = decide(&input);
        assert_eq!(d.action, ReceiveFeedbackAction::RequestPli);
        assert_eq!(d.reason, "gap-too-large");
    }

    #[test]
    fn sparse_idr_when_due() {
        let mut input = base_input();
        input.sparse_idr = SparseIdrRhythm {
            active: true,
            pli_due: true,
            action_stage: PacketRecoveryActionStage::WaitKeyframe,
            pli_interval_ms: 20.0,
        };
        let d = decide(&input);
        assert_eq!(d.action, ReceiveFeedbackAction::RequestPli);
        assert_eq!(d.reason, "sparse-idr");
        assert!(d.sparse_active);
    }

    #[test]
    fn coalesced_keyframe_yields_none_action() {
        let mut input = base_input();
        input.nack.keyframe_escalation_due = true;
        input.pli_coalesced = true;
        let d = decide(&input);
        assert_eq!(d.action, ReceiveFeedbackAction::None);
        assert_eq!(d.coalescing, ReceiveFeedbackCoalescing::SameInterval);
        assert!(d.should_touch_keyframe_executor());
    }

    #[test]
    fn forced_keyframe_request_sends_without_latching_keyframe_required() {
        let mut input = base_input();
        input.force_keyframe = true;
        input.reference_state = ReferenceChainState::Unknown;

        let d = decide(&input);

        assert_eq!(d.action, ReceiveFeedbackAction::RequestPli);
        assert_eq!(d.reason, "forced-keyframe");
        assert_eq!(d.coalescing, ReceiveFeedbackCoalescing::FreshSent);
        assert!(!d.keyframe_required);
        assert_eq!(d.keyframe_required_cause, KeyframeRequiredCause::None);
        assert!(d.should_touch_keyframe_executor());
    }

    #[test]
    fn soft_keyframe_uses_executor_interval_state() {
        let mut input = base_input();
        input.soft_keyframe = true;

        let d = decide(&input);

        assert_eq!(d.action, ReceiveFeedbackAction::RequestPli);
        assert_eq!(d.reason, "gap-repair");
        assert_eq!(d.coalescing, ReceiveFeedbackCoalescing::FreshSent);
        assert!(d.should_touch_keyframe_executor());
    }

    #[test]
    fn soft_keyframe_rate_limited_still_records_executor_outcome() {
        let mut input = base_input();
        input.soft_keyframe = true;
        input.pli_throttled = true;

        let d = decide(&input);

        assert_eq!(d.action, ReceiveFeedbackAction::None);
        assert_eq!(d.reason, "gap-repair");
        assert_eq!(d.coalescing, ReceiveFeedbackCoalescing::RateLimited);
        assert!(d.should_touch_keyframe_executor());
    }

    #[test]
    fn need_keyframe_blocks_nack() {
        let mut input = base_input();
        input.nack.sequences_len = 3;
        input.keyframe_required = true;
        let d = decide(&input);
        assert_ne!(d.action, ReceiveFeedbackAction::SendNack);
    }
}
