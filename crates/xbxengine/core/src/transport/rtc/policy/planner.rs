use crate::transport::rtc::recovery::escalation::VideoEscalationDecision;

use super::bwe::BwePolicyProposal;
use super::reconnect::ReconnectPolicyProposal;
use super::recovery::RecoveryPolicyProposal;

pub(crate) enum PlannedTransportCommand {
    ExecuteRecoveryAction {
        decision: VideoEscalationDecision,
        reason_label: String,
    },
    RequestReconnectCandidate {
        observation_id: u64,
        reason: String,
    },
    UpdateTargetRemb {
        target_remb_kbps: u32,
        decision_reason: String,
    },
}

pub(crate) struct PolicyPlanInput {
    pub(crate) recovery: Option<RecoveryPolicyProposal>,
    pub(crate) reconnect: Option<ReconnectPolicyProposal>,
    pub(crate) bwe: Option<BwePolicyProposal>,
}

pub(crate) struct PolicyPlan {
    pub(crate) commands: Vec<PlannedTransportCommand>,
}

/**
 * planner 骨架：
 * - 先固定优先级（reconnect > recovery > bwe）
 * - 后续再增加去重/冷却/互斥窗口
 */
pub(crate) struct TransportCommandPlanner;

impl TransportCommandPlanner {
    pub(crate) fn new() -> Self {
        Self
    }

    pub(crate) fn plan(&self, input: PolicyPlanInput) -> PolicyPlan {
        let mut commands = Vec::new();

        if let Some(reconnect) = input.reconnect {
            commands.push(PlannedTransportCommand::RequestReconnectCandidate {
                observation_id: reconnect.observation_id,
                reason: reconnect.reason,
            });
            return PolicyPlan { commands };
        }

        if let Some(recovery) = input.recovery {
            commands.push(PlannedTransportCommand::ExecuteRecoveryAction {
                decision: recovery.decision,
                reason_label: recovery.reason_label,
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
    use crate::transport::rtc::policy::reconnect::ReconnectPolicyProposal;

    #[test]
    fn reconnect_has_higher_priority_than_other_proposals() {
        let planner = TransportCommandPlanner::new();
        let plan = planner.plan(PolicyPlanInput {
            recovery: None,
            reconnect: Some(ReconnectPolicyProposal {
                observation_id: 42,
                reason: "peer-failed".to_string(),
            }),
            bwe: None,
        });
        assert_eq!(plan.commands.len(), 1);
        match &plan.commands[0] {
            PlannedTransportCommand::RequestReconnectCandidate { observation_id, .. } => {
                assert_eq!(*observation_id, 42);
            }
            _ => panic!("expected reconnect command"),
        }
    }
}
