use crate::transport::rtc::recovery::escalation::{
    RecoveryActionBudgetState, VideoEscalationDecision, VideoEscalationReason,
};

#[derive(Clone, Debug)]
pub(crate) struct RecoveryPolicyProposal {
    pub(crate) decision: VideoEscalationDecision,
    pub(crate) reason: VideoEscalationReason,
    pub(crate) reason_label: String,
    pub(crate) budget_before: RecoveryActionBudgetState,
    pub(crate) budget_after: RecoveryActionBudgetState,
}
