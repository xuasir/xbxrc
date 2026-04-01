use crate::transport::rtc::facts::TransportCommand;
use crate::transport::rtc::policy::bwe::BwePolicyProposal;
use crate::transport::rtc::policy::planner::{
    PlannedTransportCommand, PolicyPlanInput, TransportCommandPlanner,
};
use crate::transport::rtc::policy::recovery::RecoveryPolicyProposal;
use crate::transport::rtc::policy::video_scheduling_owner::{
    VideoHealthContract, VideoSchedulingOwnerState,
};
use crate::transport::rtc::recovery::escalation::RecoveryAction;

/// 统一调度控制面：
/// - 只负责接收 proposal 并输出命令计划
/// - 通过 owner state / owner health 对 BWE 更新做硬门控
/// - 不在 transport/media 层分散下发恢复动作
pub(crate) struct SchedulingPolicyEngine {
    planner: TransportCommandPlanner,
}

pub(crate) struct SchedulingPolicyInput {
    pub(crate) owner_state: VideoSchedulingOwnerState,
    pub(crate) owner_health: VideoHealthContract,
    pub(crate) recovery: Option<RecoveryPolicyProposal>,
    pub(crate) bwe: Option<BwePolicyProposal>,
}

impl SchedulingPolicyEngine {
    pub(crate) fn new() -> Self {
        Self {
            planner: TransportCommandPlanner::new(),
        }
    }

    pub(crate) fn plan(&self, input: SchedulingPolicyInput) -> Vec<PlannedTransportCommand> {
        let bwe = if allows_bwe_update(input.owner_state, input.owner_health) {
            input.bwe
        } else {
            None
        };
        self.planner
            .plan(PolicyPlanInput {
                recovery: input.recovery,
                bwe,
            })
            .commands
    }
}

fn allows_bwe_update(
    owner_state: VideoSchedulingOwnerState,
    owner_health: VideoHealthContract,
) -> bool {
    matches!(
        (owner_state, owner_health),
        (
            VideoSchedulingOwnerState::SeekingAnchor,
            VideoHealthContract::Startup
        ) | (
            VideoSchedulingOwnerState::Priming,
            VideoHealthContract::Startup
        ) | (
            VideoSchedulingOwnerState::StableServing,
            VideoHealthContract::Stable
        )
    )
}

pub(crate) fn map_planned_command_to_transport_commands(
    command: PlannedTransportCommand,
    bwe_observation_id: u64,
) -> Vec<TransportCommand> {
    match command {
        PlannedTransportCommand::ExecuteRecoveryAction {
            decision,
            reason_label,
        } => map_recovery_action_to_transport_commands(
            decision.action,
            reason_label,
            decision.observation_id,
        ),
        PlannedTransportCommand::UpdateTargetRemb {
            target_remb_kbps,
            decision_reason,
        } => vec![TransportCommand::SetTargetRembKbps {
            target_kbps: target_remb_kbps,
            reason: decision_reason,
            observation_id: bwe_observation_id,
        }],
    }
}

fn map_recovery_action_to_transport_commands(
    action: RecoveryAction,
    reason: String,
    observation_id: u64,
) -> Vec<TransportCommand> {
    match action {
        RecoveryAction::RequestKeyframe => vec![TransportCommand::RequestKeyframe {
            reason,
            observation_id,
        }],
        RecoveryAction::RequestDecoderReset => vec![TransportCommand::RequestDecoderReset {
            reason,
            observation_id,
        }],
        RecoveryAction::RequestReconnectCandidate => {
            vec![TransportCommand::RequestReconnectCandidate {
                reason,
                observation_id,
            }]
        }
        RecoveryAction::RequestKeyframeAndDecoderReset | RecoveryAction::StartupLowQualityRetry => {
            vec![
                TransportCommand::RequestKeyframe {
                    reason: reason.clone(),
                    observation_id,
                },
                TransportCommand::RequestDecoderReset {
                    reason,
                    observation_id,
                },
            ]
        }
        RecoveryAction::WaitForBurst
        | RecoveryAction::WaitForDecoderResetBurst
        | RecoveryAction::CooldownSuppressed
        | RecoveryAction::StartupGraceSuppressed => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::{SchedulingPolicyEngine, SchedulingPolicyInput};
    use crate::transport::rtc::bwe::evaluator::RtcBweEvaluation;
    use crate::transport::rtc::policy::bwe::BwePolicyProposal;
    use crate::transport::rtc::policy::recovery::RecoveryPolicyProposal;
    use crate::transport::rtc::policy::video_scheduling_owner::{
        VideoHealthContract, VideoSchedulingOwnerState,
    };
    use crate::transport::rtc::recovery::escalation::{
        RecoveryAction, RecoveryActionBudgetState, VideoEscalationDecision, VideoEscalationReason,
    };

    fn build_recovery_proposal(
        action: RecoveryAction,
        observation_id: u64,
    ) -> RecoveryPolicyProposal {
        RecoveryPolicyProposal {
            decision: VideoEscalationDecision {
                observation_id,
                action,
            },
            reason: VideoEscalationReason::AdapterIdleTimeout,
            reason_label: "adapterIdleTimeout".to_string(),
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
                reconnect_budget_used: 0,
                reconnect_budget_limit: 1,
            },
        }
    }

    #[test]
    fn high_no_pending_pressure_does_not_override_recovery_budget_action() {
        let engine = SchedulingPolicyEngine::new();
        let commands = engine.plan(SchedulingPolicyInput {
            owner_state: VideoSchedulingOwnerState::SupplyStarved,
            owner_health: VideoHealthContract::Starved,
            recovery: Some(build_recovery_proposal(
                RecoveryAction::CooldownSuppressed,
                9,
            )),
            bwe: None,
        });
        assert_eq!(commands.len(), 1);
        match &commands[0] {
            crate::transport::rtc::policy::planner::PlannedTransportCommand::ExecuteRecoveryAction {
                decision,
                ..
            } => assert_eq!(decision.action, RecoveryAction::CooldownSuppressed),
            _ => panic!("unexpected command kind"),
        }
    }

    #[test]
    fn planner_mapping_remains_stable_for_soft_actions() {
        let engine = SchedulingPolicyEngine::new();
        let commands = engine.plan(SchedulingPolicyInput {
            owner_state: VideoSchedulingOwnerState::RebuildingSupply,
            owner_health: VideoHealthContract::Recovering,
            recovery: Some(build_recovery_proposal(RecoveryAction::WaitForBurst, 10)),
            bwe: None,
        });
        assert_eq!(commands.len(), 1);
        match commands.first() {
            Some(
                crate::transport::rtc::policy::planner::PlannedTransportCommand::ExecuteRecoveryAction {
                    decision,
                    ..
                },
            ) => assert_eq!(decision.action, RecoveryAction::WaitForBurst),
            _ => panic!("unexpected command kind"),
        }
    }

    #[test]
    fn reconnect_is_emitted_through_recovery_action_mapping() {
        let engine = SchedulingPolicyEngine::new();
        let commands = engine.plan(SchedulingPolicyInput {
            owner_state: VideoSchedulingOwnerState::RebuildingSupply,
            owner_health: VideoHealthContract::Recovering,
            recovery: Some(RecoveryPolicyProposal {
                decision: VideoEscalationDecision {
                    observation_id: 99,
                    action: RecoveryAction::RequestReconnectCandidate,
                },
                reason: VideoEscalationReason::LifecycleRecovering,
                reason_label: "rtcConnectionRecovering".to_string(),
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
            }),
            bwe: None,
        });
        assert_eq!(commands.len(), 1);
        match commands.first() {
            Some(
                crate::transport::rtc::policy::planner::PlannedTransportCommand::ExecuteRecoveryAction {
                    decision,
                    ..
                },
            ) => {
                assert_eq!(decision.observation_id, 99);
                assert_eq!(decision.action, RecoveryAction::RequestReconnectCandidate);
            }
            _ => panic!("unexpected command kind"),
        }
    }

    #[test]
    fn rebuilding_supply_recovering_blocks_bwe_but_keeps_recovery() {
        let engine = SchedulingPolicyEngine::new();
        let commands = engine.plan(SchedulingPolicyInput {
            owner_state: VideoSchedulingOwnerState::RebuildingSupply,
            owner_health: VideoHealthContract::Recovering,
            recovery: Some(RecoveryPolicyProposal {
                decision: VideoEscalationDecision {
                    observation_id: 7,
                    action: RecoveryAction::RequestKeyframe,
                },
                reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
                reason_label: "transportAwaitRecoveryKeyframe".to_string(),
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
                    keyframe_budget_used: 1,
                    keyframe_budget_limit: 3,
                    decoder_reset_budget_used: 0,
                    decoder_reset_budget_limit: 2,
                    reconnect_budget_used: 0,
                    reconnect_budget_limit: 1,
                },
            }),
            bwe: Some(BwePolicyProposal {
                evaluation: RtcBweEvaluation {
                    target_remb_kbps: 18_000,
                    decision_reason: "twcc-gcc-cloud-hold".to_string(),
                    observation_id: 123,
                },
            }),
        });

        assert_eq!(commands.len(), 1);
        match commands.first() {
            Some(
                crate::transport::rtc::policy::planner::PlannedTransportCommand::ExecuteRecoveryAction {
                    decision,
                    ..
                },
            ) => assert_eq!(decision.action, RecoveryAction::RequestKeyframe),
            _ => panic!("unexpected command kind"),
        }
    }

    #[test]
    fn stable_serving_stable_keeps_bwe_update() {
        let engine = SchedulingPolicyEngine::new();
        let commands = engine.plan(SchedulingPolicyInput {
            owner_state: VideoSchedulingOwnerState::StableServing,
            owner_health: VideoHealthContract::Stable,
            recovery: None,
            bwe: Some(BwePolicyProposal {
                evaluation: RtcBweEvaluation {
                    target_remb_kbps: 24_000,
                    decision_reason: "twcc-gcc-cloud-ramp".to_string(),
                    observation_id: 321,
                },
            }),
        });

        assert_eq!(commands.len(), 1);
        match commands.first() {
            Some(
                crate::transport::rtc::policy::planner::PlannedTransportCommand::UpdateTargetRemb {
                    target_remb_kbps,
                    decision_reason,
                },
            ) => {
                assert_eq!(*target_remb_kbps, 24_000);
                assert_eq!(decision_reason, "twcc-gcc-cloud-ramp");
            }
            _ => panic!("unexpected command kind"),
        }
    }
}
