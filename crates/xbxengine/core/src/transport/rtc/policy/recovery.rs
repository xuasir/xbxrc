use crate::transport::rtc::recovery::escalation::{
    RecoveryAction, RecoveryActionBudgetState, VideoEscalationController, VideoEscalationDecision,
    VideoEscalationReason,
};
use crate::XbxEngineRecoveryReasonDomain;

#[derive(Clone, Debug)]
pub(crate) struct RecoveryPolicyProposal {
    pub(crate) decision: VideoEscalationDecision,
    pub(crate) reason: VideoEscalationReason,
    pub(crate) reason_label: String,
    pub(crate) reason_domain: XbxEngineRecoveryReasonDomain,
    pub(crate) reconnect_gate_detail: Option<String>,
    pub(crate) budget_before: RecoveryActionBudgetState,
    pub(crate) budget_after: RecoveryActionBudgetState,
}

impl RecoveryPolicyProposal {
    pub(crate) fn with_runtime_reason_domain(mut self) -> Self {
        self.reason_domain = self.runtime_reason_domain();
        self
    }

    pub(crate) fn runtime_reason_domain(&self) -> XbxEngineRecoveryReasonDomain {
        resolve_runtime_reconnect_reason_domain(self.reason, self.decision.action)
    }

    pub(crate) fn ledger_gate_result(
        &self,
        failed_terminal_reason: Option<&str>,
        local_probe_only: bool,
    ) -> String {
        if let Some(reason) = failed_terminal_reason {
            return format!("terminal:{reason}");
        }
        if local_probe_only {
            return "pass:localProbe".to_string();
        }
        if let Some(detail) = self.reconnect_gate_detail.as_deref() {
            if self.decision.action == RecoveryAction::RequestReconnectCandidate {
                return format!("pass:{detail}");
            }
            return format!("suppressed:{detail}");
        }
        if matches!(
            self.decision.action,
            RecoveryAction::CoalescedKeyframeInFlight
                | RecoveryAction::CoalescedDecoderResetInFlight
        ) {
            return self.decision.action.label().to_string();
        }
        let contract = VideoEscalationController::action_contract(self.decision.action);
        if contract.owner.is_some() {
            return "pass".to_string();
        }
        format!("suppressed:{}", self.decision.action.label())
    }

    pub(crate) fn ledger_action_selected(&self, failed_terminal_reason: Option<&str>) -> String {
        if failed_terminal_reason.is_some() {
            return "failed-terminal".to_string();
        }
        self.decision.action.label().to_string()
    }
}

pub(crate) fn resolve_runtime_reconnect_reason_domain(
    reason: VideoEscalationReason,
    action: RecoveryAction,
) -> XbxEngineRecoveryReasonDomain {
    if action != RecoveryAction::RequestReconnectCandidate {
        // 非 reconnect 动作属于本地维护域，不再借道连接域语义。
        return XbxEngineRecoveryReasonDomain::Local;
    }
    match reason {
        VideoEscalationReason::LifecycleRecovering
        | VideoEscalationReason::TransportExpiredDeadline
        | VideoEscalationReason::TransportSevereDeadline
        | VideoEscalationReason::TransportRecoveredLate
        | VideoEscalationReason::TransportSampleLoss => {
            XbxEngineRecoveryReasonDomain::ConnectivityTransport
        }
        VideoEscalationReason::WaitKeyframe
        | VideoEscalationReason::TransportAwaitRecoveryKeyframe
        | VideoEscalationReason::DisplaySupplyCritical
        | VideoEscalationReason::Reconfigure
        | VideoEscalationReason::DecoderBackendFailure
        | VideoEscalationReason::AdapterIdleTimeout
        | VideoEscalationReason::AdapterThinStream => XbxEngineRecoveryReasonDomain::Local,
    }
}

#[cfg(test)]
mod tests {
    use super::resolve_runtime_reconnect_reason_domain;
    use crate::transport::rtc::recovery::escalation::{RecoveryAction, VideoEscalationReason};

    #[test]
    fn non_reconnect_actions_always_resolve_to_local_domain() {
        assert_eq!(
            resolve_runtime_reconnect_reason_domain(
                VideoEscalationReason::TransportExpiredDeadline,
                RecoveryAction::RequestDecoderReset,
            ),
            crate::XbxEngineRecoveryReasonDomain::Local
        );
        assert_eq!(
            resolve_runtime_reconnect_reason_domain(
                VideoEscalationReason::TransportSevereDeadline,
                RecoveryAction::RequestKeyframe,
            ),
            crate::XbxEngineRecoveryReasonDomain::Local
        );
    }

    #[test]
    fn reconnect_candidate_keeps_transport_domain_for_deadline_paths() {
        assert_eq!(
            resolve_runtime_reconnect_reason_domain(
                VideoEscalationReason::TransportExpiredDeadline,
                RecoveryAction::RequestReconnectCandidate,
            ),
            crate::XbxEngineRecoveryReasonDomain::ConnectivityTransport
        );
    }
}
