use crate::transport::rtc::recovery::escalation::{
    RecoveryActionBudgetState, VideoEscalationDecision, VideoEscalationReason,
};
use crate::XbxEngineRecoveryReasonDomain;

#[derive(Clone, Debug)]
pub(crate) struct RecoveryPolicyProposal {
    pub(crate) decision: VideoEscalationDecision,
    pub(crate) reason: VideoEscalationReason,
    pub(crate) reason_label: String,
    pub(crate) reason_domain: XbxEngineRecoveryReasonDomain,
    pub(crate) budget_before: RecoveryActionBudgetState,
    pub(crate) budget_after: RecoveryActionBudgetState,
}
