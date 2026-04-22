//! 状态驱动的恢复协调器
//!
//! 基于状态机的恢复决策系统，核心特性：
//! - 五状态模型：Healthy → LocalRepair → FrameRecovery → DecoderRecovery → TransportRecovery
//! - In-flight 门控：避免重复请求
//! - 自动状态转换：基于执行事实（clean anchor, decode progress）

use super::action_coordinator::ActionCoordinator;
use super::contract::CoalescingMode;
use super::escalation::RecoveryAction;
use super::observation::RecoveryObservation;
use super::policy::RecoveryScenarioProfile;
#[cfg(test)]
use super::state_machine::RecoveryState;

/// 状态驱动的恢复协调器
pub(crate) struct StateRecoveryCoordinator {
    coordinator: ActionCoordinator,
    profile: RecoveryScenarioProfile,
}

impl StateRecoveryCoordinator {
    /// 创建新的协调器
    pub(crate) fn new(profile: RecoveryScenarioProfile, recovery_epoch: u64) -> Self {
        Self {
            coordinator: ActionCoordinator::new(profile, recovery_epoch),
            profile,
        }
    }

    /// 处理恢复观察，返回决策
    pub(crate) fn on_observation(&mut self, observation: RecoveryObservation) -> RecoveryDecision {
        let decision = self.coordinator.decide(observation);

        RecoveryDecision {
            action: decision.action,
            coalescing_mode: decision.coalescing_mode,
            unlock_reason: decision.unlock_reason,
            preempt_reason: decision.preempt_reason,
        }
    }

    /// 通知clean anchor（可能回到Healthy状态）
    pub(crate) fn on_clean_anchor(&mut self, has_stable_output: bool) {
        self.coordinator
            .check_healthy_transition(true, has_stable_output);
    }

    /// 通知IDR已解码
    pub(crate) fn on_idr_decoded(&mut self) {
        self.coordinator.state_machine_mut().mark_idr_decoded();
    }

    /// 通知decoder reset已完成
    pub(crate) fn on_decoder_reset_completed(&mut self) {
        self.coordinator
            .state_machine_mut()
            .mark_decoder_reset_completed();
    }

    /// 通知reconnect已完成
    pub(crate) fn on_reconnect_completed(&mut self) {
        self.coordinator
            .state_machine_mut()
            .mark_reconnect_completed();
    }

    /// 更新恢复epoch
    pub(crate) fn update_recovery_epoch(&mut self, new_epoch: u64) {
        self.coordinator.update_recovery_epoch(new_epoch);
    }

    /// 更新场景化profile
    pub(crate) fn current_profile(&self) -> RecoveryScenarioProfile {
        self.profile
    }

    /// 获取当前状态（仅测试断言使用）
    #[cfg(test)]
    pub(crate) fn current_state(&self) -> RecoveryState {
        self.coordinator.current_state()
    }

    /// 获取状态机的不可变引用
    pub(crate) fn state_machine(&self) -> &super::state_machine::RecoveryStateMachine {
        self.coordinator.state_machine()
    }

    /// 获取状态机的可变引用
    pub(crate) fn state_machine_mut(&mut self) -> &mut super::state_machine::RecoveryStateMachine {
        self.coordinator.state_machine_mut()
    }
}

/// 简化的恢复决策结果
#[derive(Clone, Debug)]
pub(crate) struct RecoveryDecision {
    /// 选择的动作
    pub(crate) action: RecoveryAction,
    /// 合并模式
    pub(crate) coalescing_mode: Option<CoalescingMode>,
    /// 解锁原因
    pub(crate) unlock_reason: Option<String>,
    /// 抢占原因
    pub(crate) preempt_reason: Option<String>,
}

impl RecoveryDecision {
    /// 是否需要执行动作（非抑制类动作）
    #[cfg(test)]
    pub(crate) fn should_execute(&self) -> bool {
        !matches!(
            self.action,
            RecoveryAction::WaitForBurst
                | RecoveryAction::WaitForDecoderResetBurst
                | RecoveryAction::CooldownSuppressed
                | RecoveryAction::CoalescedKeyframeInFlight
                | RecoveryAction::CoalescedDecoderResetInFlight
                | RecoveryAction::StartupGraceSuppressed
        )
    }

    /// 是否是keyframe请求
    #[cfg(test)]
    pub(crate) fn is_keyframe_request(&self) -> bool {
        matches!(self.action, RecoveryAction::RequestPli | RecoveryAction::RequestFir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::rtc::recovery::escalation::VideoEscalationReason;

    fn test_profile() -> RecoveryScenarioProfile {
        crate::transport::rtc::recovery::policy::ScenarioPolicyResolver::resolve_recovery_profile_by_kind(
            crate::transport::rtc::recovery::policy::ScenarioPolicyProfileKind::HomeLanGaming,
        )
    }

    #[test]
    fn test_basic_flow() {
        let mut coordinator = StateRecoveryCoordinator::new(test_profile(), 1);

        // 发送WaitKeyframe观察
        let observation = RecoveryObservation::from_reason(
            VideoEscalationReason::WaitKeyframe,
            "waitKeyframe".to_string(),
            1000.0,
        );
        let decision = coordinator.on_observation(observation);

        assert_eq!(decision.action, RecoveryAction::RequestPli);
        assert!(decision.should_execute());
        assert!(decision.is_keyframe_request());
        assert_eq!(coordinator.current_state(), RecoveryState::FrameRecovery);
    }

    #[test]
    fn test_clean_anchor_recovery() {
        let mut coordinator = StateRecoveryCoordinator::new(test_profile(), 1);

        // 进入恢复状态
        let observation = RecoveryObservation::from_reason(
            VideoEscalationReason::WaitKeyframe,
            "waitKeyframe".to_string(),
            1000.0,
        );
        coordinator.on_observation(observation);
        assert_eq!(coordinator.current_state(), RecoveryState::FrameRecovery);

        // 通知clean anchor
        coordinator.on_clean_anchor(true);
        assert_eq!(coordinator.current_state(), RecoveryState::Healthy);
    }

    #[test]
    fn test_coalescing() {
        let mut coordinator = StateRecoveryCoordinator::new(test_profile(), 1);

        // 第一次请求
        let observation = RecoveryObservation::from_reason(
            VideoEscalationReason::WaitKeyframe,
            "waitKeyframe".to_string(),
            1000.0,
        );
        let decision = coordinator.on_observation(observation);
        assert!(decision.should_execute());

        // 第二次请求应该被coalesce
        let observation = RecoveryObservation::from_reason(
            VideoEscalationReason::WaitKeyframe,
            "waitKeyframe".to_string(),
            1100.0,
        );
        let decision = coordinator.on_observation(observation);
        assert!(!decision.should_execute());
        assert_eq!(decision.action, RecoveryAction::CoalescedKeyframeInFlight);
    }
}
