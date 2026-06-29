use crate::transport::rtc::recovery::escalation::VideoEscalationDecision;
use crate::XbxEngineRecoveryReasonDomain;

use super::bwe::BwePolicyProposal;
use super::recovery::RecoveryPolicyProposal;

pub(crate) enum PlannedTransportCommand {
    ExecuteRecoveryAction {
        decision: VideoEscalationDecision,
        reason_label: String,
        reason_domain: XbxEngineRecoveryReasonDomain,
    },
    UpdateTargetRemb {
        target_remb_kbps: u32,
        decision_reason: String,
    },
}

pub(crate) struct PolicyPlanInput {
    pub(crate) recovery: Option<RecoveryPolicyProposal>,
    pub(crate) bwe: Option<BwePolicyProposal>,
}

pub(crate) struct PolicyPlan {
    pub(crate) commands: Vec<PlannedTransportCommand>,
}

/**
 * planner 骨架：
 * - 先固定优先级（recovery > bwe）
 * - 后续再增加去重/冷却/互斥窗口
 */
pub(crate) struct TransportCommandPlanner;

impl TransportCommandPlanner {
    pub(crate) fn new() -> Self {
        Self
    }

    pub(crate) fn plan(&self, input: PolicyPlanInput) -> PolicyPlan {
        let mut commands = Vec::new();

        if let Some(recovery) = input.recovery {
            commands.push(PlannedTransportCommand::ExecuteRecoveryAction {
                decision: recovery.decision,
                reason_label: recovery.reason_label,
                reason_domain: recovery.reason_domain,
            });
        }

        if let Some(bwe) = input.bwe {
            commands.push(PlannedTransportCommand::UpdateTargetRemb {
                target_remb_kbps: bwe.evaluation.target_remb_kbps,
                decision_reason: bwe.evaluation.decision_reason,
            });
        }

        PolicyPlan { commands }
    }
}

#[cfg(test)]
mod tests {
    use super::{PlannedTransportCommand, PolicyPlanInput, TransportCommandPlanner};
    use crate::transport::rtc::policy::recovery::RecoveryPolicyProposal;
    use crate::transport::rtc::recovery::escalation::{
        RecoveryAction, RecoveryActionBudgetState, VideoEscalationDecision, VideoEscalationReason,
    };

    #[test]
    fn recovery_has_higher_priority_than_bwe() {
        let planner = TransportCommandPlanner::new();
        let plan = planner.plan(PolicyPlanInput {
            recovery: Some(RecoveryPolicyProposal {
                decision: VideoEscalationDecision {
                    observation_id: 42,
                    action: RecoveryAction::RequestReconnectCandidate,
                },
                reason: VideoEscalationReason::LifecycleRecovering,
                reason_label: "rtcConnectionRecovering".to_string(),
                reason_domain: crate::XbxEngineRecoveryReasonDomain::ConnectivityTransport,
                reason_domain_before_runtime_resolution: None,
                reason_domain_after_runtime_resolution: None,
                remote_terminal_domain_promoted: false,
                remote_terminal_active: false,
                reconnect_gate_detail: None,
                budget_before: RecoveryActionBudgetState {
                    recovery_epoch: 1,
                    keyframe_budget_used: 0,
                    keyframe_budget_limit: 3,
                    decoder_reset_budget_used: 0,
                    decoder_reset_budget_limit: 2,
                    reconnect_budget_used: 0,
                    reconnect_budget_limit: 1,
                },
                budget_after: RecoveryActionBudgetState {
                    recovery_epoch: 1,
                    keyframe_budget_used: 0,
                    keyframe_budget_limit: 3,
                    decoder_reset_budget_used: 0,
                    decoder_reset_budget_limit: 2,
                    reconnect_budget_used: 1,
                    reconnect_budget_limit: 1,
                },
                coalescing_mode: None,
                unlock_reason: None,
                preempt_reason: None,
            }),
            bwe: None,
        });
        assert_eq!(plan.commands.len(), 1);
        match &plan.commands[0] {
            PlannedTransportCommand::ExecuteRecoveryAction {
                decision,
                reason_domain,
                ..
            } => {
                assert_eq!(decision.observation_id, 42);
                assert_eq!(decision.action, RecoveryAction::RequestReconnectCandidate);
                assert_eq!(
                    *reason_domain,
                    crate::XbxEngineRecoveryReasonDomain::ConnectivityTransport
                );
            }
            _ => panic!("expected recovery command"),
        }
    }

    #[test]
    fn planner_preserves_local_reason_domain_for_reconnect_candidate() {
        let planner = TransportCommandPlanner::new();
        let plan = planner.plan(PolicyPlanInput {
            recovery: Some(RecoveryPolicyProposal {
                decision: VideoEscalationDecision {
                    observation_id: 43,
                    action: RecoveryAction::RequestReconnectCandidate,
                },
                reason: VideoEscalationReason::DisplaySupplyCritical,
                reason_label: "displaySupplyCritical".to_string(),
                reason_domain: crate::XbxEngineRecoveryReasonDomain::Local,
                reason_domain_before_runtime_resolution: None,
                reason_domain_after_runtime_resolution: None,
                remote_terminal_domain_promoted: false,
                remote_terminal_active: false,
                reconnect_gate_detail: None,
                budget_before: RecoveryActionBudgetState {
                    recovery_epoch: 1,
                    keyframe_budget_used: 0,
                    keyframe_budget_limit: 3,
                    decoder_reset_budget_used: 0,
                    decoder_reset_budget_limit: 2,
                    reconnect_budget_used: 0,
                    reconnect_budget_limit: 1,
                },
                budget_after: RecoveryActionBudgetState {
                    recovery_epoch: 1,
                    keyframe_budget_used: 0,
                    keyframe_budget_limit: 3,
                    decoder_reset_budget_used: 0,
                    decoder_reset_budget_limit: 2,
                    reconnect_budget_used: 1,
                    reconnect_budget_limit: 1,
                },
                coalescing_mode: None,
                unlock_reason: None,
                preempt_reason: None,
            }),
            bwe: None,
        });
        match &plan.commands[0] {
            PlannedTransportCommand::ExecuteRecoveryAction {
                reason_label,
                reason_domain,
                ..
            } => {
                assert_eq!(reason_label, "displaySupplyCritical");
                assert_eq!(*reason_domain, crate::XbxEngineRecoveryReasonDomain::Local);
            }
            _ => panic!("expected recovery command"),
        }
    }
}
