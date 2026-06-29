use crate::api::backend::XbxEngineRecoveryDecisionLedgerObservation;
use crate::transport::rtc::recovery::contract::CoalescingMode;
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
    pub(crate) reason_domain_before_runtime_resolution: Option<XbxEngineRecoveryReasonDomain>,
    pub(crate) reason_domain_after_runtime_resolution: Option<XbxEngineRecoveryReasonDomain>,
    pub(crate) remote_terminal_domain_promoted: bool,
    pub(crate) remote_terminal_active: bool,
    pub(crate) reconnect_gate_detail: Option<String>,
    pub(crate) budget_before: RecoveryActionBudgetState,
    pub(crate) budget_after: RecoveryActionBudgetState,
    // Coalescing 语义（从 CoordinatorProposal 传递）
    pub(crate) coalescing_mode: Option<CoalescingMode>,
    pub(crate) unlock_reason: Option<String>,
    pub(crate) preempt_reason: Option<String>,
}

impl RecoveryPolicyProposal {
    pub(crate) fn with_runtime_reason_domain(mut self) -> Self {
        let before = self.reason_domain;
        let after = self.runtime_reason_domain();
        self.reason_domain_before_runtime_resolution = Some(before);
        self.reason_domain_after_runtime_resolution = Some(after);
        self.reason_domain = after;
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

/// `latest_recovery_decision_ledger` 的「待 command 回填」仅适用于会下发 `TransportCommand` 的动作；
/// 抑制/等待/合并占位等不会走 `transport_session::update_recovery_decision_command_result`。
#[allow(dead_code)]
pub(crate) fn ledger_action_selected_expects_command_result(action_selected: &str) -> bool {
    matches!(
        action_selected,
        "requestDecoderReset" | "requestReconnectCandidate"
    )
}

#[allow(dead_code)]
pub(crate) fn recovery_decision_ledger_has_pending_transport_command(
    ledger: &XbxEngineRecoveryDecisionLedgerObservation,
) -> bool {
    ledger.command_result.is_none()
        && ledger_action_selected_expects_command_result(ledger.action_selected.as_str())
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
        VideoEscalationReason::TransportLowValueDeadline
        | VideoEscalationReason::TransportRepairableDeadline
        | VideoEscalationReason::WaitKeyframe
        | VideoEscalationReason::TransportAwaitRecoveryKeyframe
        | VideoEscalationReason::LocalSupplySuspect
        | VideoEscalationReason::DisplaySupplyCritical
        | VideoEscalationReason::Reconfigure
        | VideoEscalationReason::DecoderBackendFailure
        | VideoEscalationReason::AdapterIdleTimeout
        | VideoEscalationReason::AdapterThinStream => XbxEngineRecoveryReasonDomain::Local,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ledger_action_selected_expects_command_result,
        recovery_decision_ledger_has_pending_transport_command,
        resolve_runtime_reconnect_reason_domain,
    };
    use crate::api::backend::XbxEngineRecoveryDecisionLedgerObservation;
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
                RecoveryAction::RequestPli,
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

    #[test]
    fn ledger_expects_command_result_only_for_transport_owner_actions() {
        assert!(!ledger_action_selected_expects_command_result(
            RecoveryAction::RequestPli.label()
        ));
        assert!(!ledger_action_selected_expects_command_result(
            RecoveryAction::RequestFir.label()
        ));
        assert!(ledger_action_selected_expects_command_result(
            RecoveryAction::RequestDecoderReset.label()
        ));
        assert!(ledger_action_selected_expects_command_result(
            RecoveryAction::RequestReconnectCandidate.label()
        ));
        assert!(!ledger_action_selected_expects_command_result(
            RecoveryAction::CooldownSuppressed.label()
        ));
        assert!(!ledger_action_selected_expects_command_result(
            RecoveryAction::WaitForBurst.label()
        ));
    }

    #[test]
    fn recovery_decision_ledger_pending_matches_transport_command_semantics() {
        let suppressed = XbxEngineRecoveryDecisionLedgerObservation {
            decision_id: 1,
            state_before: "stable".to_string(),
            state_after: "stable".to_string(),
            input_signal: "x".to_string(),
            gate_result: "suppressed:cooldownSuppressed".to_string(),
            action_selected: RecoveryAction::CooldownSuppressed.label().to_string(),
            frame_value: None,
            gap_severity: None,
            repairability: None,
            recovery_episode_stage: None,
            recovery_episode_progress_at_ms: None,
            coalescing_mode: None,
            unlock_reason: None,
            preempt_reason: None,
            recovery_primary_action: None,
            owner_surface_state: None,
            anchor_evidence: None,
            keyframe_episode_health: None,
            escalation_basis: None,
            budget_before: None,
            budget_after: None,
            trigger_observation_label: None,
            trigger_observation_summary: None,
            command_result: None,
            command_detail: None,
            observed_at_ms: 0.0,
            ..Default::default()
        };
        assert!(
            !recovery_decision_ledger_has_pending_transport_command(&suppressed),
            "抑制类动作不应被当作等待 TransportCommand 回填"
        );

        let mut issued = suppressed.clone();
        issued.action_selected = RecoveryAction::RequestDecoderReset.label().to_string();
        assert!(recovery_decision_ledger_has_pending_transport_command(
            &issued
        ));

        issued.command_result = Some("succeeded".to_string());
        assert!(!recovery_decision_ledger_has_pending_transport_command(
            &issued
        ));
    }
}
