use crate::transport::rtc::recovery::escalation::VideoEscalationDecision;

pub(crate) struct RecoveryPolicyProposal {
    pub(crate) decision: VideoEscalationDecision,
    pub(crate) reason_label: String,
}
