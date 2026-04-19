//! 简化的动作协调器
//!
//! 核心职责：
//! - 基于状态的动作分发
//! - 资源效率门控（in-flight门控、状态门控）
//! - 与现有机制集成（repairability评分、transport-await事实模型）

use super::contract::CoalescingMode;
use super::escalation::RecoveryAction;
use super::observation::{RecoveryObservation, RecoverySeverity};
use super::policy::RecoveryScenarioProfile;
use super::state_machine::{RecoveryState, RecoveryStateMachine};

/// 恢复决策结果
#[derive(Clone, Debug)]
pub(crate) struct RecoveryDecision {
    /// 选择的动作
    pub(crate) action: RecoveryAction,
    /// 新状态（如果状态转换）
    pub(crate) new_state: Option<RecoveryState>,
    /// 合并模式（Merge/Refresh/Preempt），None 表示无 in-flight episode
    pub(crate) coalescing_mode: Option<CoalescingMode>,
    /// 解锁原因（如果解锁了 in-flight episode）
    pub(crate) unlock_reason: Option<String>,
    /// 抢占原因（如果抢占了旧 episode）
    pub(crate) preempt_reason: Option<String>,
}

impl RecoveryDecision {
    fn new(action: RecoveryAction, _reason: String) -> Self {
        Self {
            action,
            new_state: None,
            coalescing_mode: None,
            unlock_reason: None,
            preempt_reason: None,
        }
    }

    fn with_state_transition(mut self, new_state: RecoveryState) -> Self {
        self.new_state = Some(new_state);
        self
    }

    fn with_coalescing(mut self, mode: CoalescingMode) -> Self {
        self.coalescing_mode = Some(mode);
        self
    }

    fn with_unlock_reason(mut self, reason: String) -> Self {
        self.unlock_reason = Some(reason);
        self
    }

    fn with_preempt_reason(mut self, reason: String) -> Self {
        self.preempt_reason = Some(reason);
        self
    }
}

/// 简化的动作协调器
pub(crate) struct ActionCoordinator {
    /// 状态机
    state_machine: RecoveryStateMachine,
    /// Repairability阈值（低于此值升级到IDR）
    repairability_threshold: f64,
}

impl ActionCoordinator {
    /// 创建新的协调器
    pub(crate) fn new(profile: RecoveryScenarioProfile, recovery_epoch: u64) -> Self {
        Self {
            state_machine: RecoveryStateMachine::new(profile, recovery_epoch),
            repairability_threshold: 0.45, // 默认阈值
        }
    }

    /// 获取状态机引用
    pub(crate) fn state_machine(&self) -> &RecoveryStateMachine {
        &self.state_machine
    }

    /// 获取当前状态（仅测试断言使用）
    #[cfg(test)]
    pub(crate) fn current_state(&self) -> RecoveryState {
        self.state_machine.current_state()
    }

    /// 获取状态机可变引用
    pub(crate) fn state_machine_mut(&mut self) -> &mut RecoveryStateMachine {
        &mut self.state_machine
    }

    /// 处理恢复观察，返回决策
    pub(crate) fn decide(&mut self, observation: RecoveryObservation) -> RecoveryDecision {
        let current_state = self.state_machine.current_state();

        match current_state {
            RecoveryState::Healthy => self.decide_from_healthy(observation),
            RecoveryState::LocalRepair => self.decide_from_local_repair(observation),
            RecoveryState::FrameRecovery => self.decide_from_frame_recovery(observation),
            RecoveryState::DecoderRecovery => self.decide_from_decoder_recovery(observation),
            RecoveryState::TransportRecovery => self.decide_from_transport_recovery(observation),
        }
    }

    /// 从Healthy状态决策
    fn decide_from_healthy(&mut self, observation: RecoveryObservation) -> RecoveryDecision {
        // 检查是否需要立即升级到更高层级
        if observation.requires_reconnect() {
            self.state_machine.transition_to_transport_recovery();
            self.state_machine.mark_reconnect_requested();
            return RecoveryDecision::new(
                RecoveryAction::RequestReconnectCandidate,
                format!("reconnect required: {}", observation.reason_label),
            )
            .with_state_transition(RecoveryState::TransportRecovery);
        }

        if observation.requires_decoder_reset() {
            self.state_machine.transition_to_decoder_recovery();
            return RecoveryDecision::new(
                RecoveryAction::RequestDecoderReset,
                format!("decoder reset required: {}", observation.reason_label),
            )
            .with_state_transition(RecoveryState::DecoderRecovery);
        }

        if observation.severity >= RecoverySeverity::ChainBroken {
            self.state_machine.transition_to_frame_recovery();
            self.state_machine.mark_idr_requested();
            return RecoveryDecision::new(
                RecoveryAction::RequestKeyframe,
                format!("chain broken: {}", observation.reason_label),
            )
            .with_state_transition(RecoveryState::FrameRecovery);
        }

        if observation.severity >= RecoverySeverity::PacketLoss {
            // 检查repairability决定是NACK还是IDR
            if observation.should_escalate_to_idr(self.repairability_threshold) {
                self.state_machine.transition_to_frame_recovery();
                self.state_machine.mark_idr_requested();
                return RecoveryDecision::new(
                    RecoveryAction::RequestKeyframe,
                    format!("low repairability: {}", observation.reason_label),
                )
                .with_state_transition(RecoveryState::FrameRecovery);
            } else {
                self.state_machine.transition_to_local_repair();
                return RecoveryDecision::new(
                    RecoveryAction::WaitForBurst,
                    format!("packet loss, NACK active: {}", observation.reason_label),
                )
                .with_state_transition(RecoveryState::LocalRepair);
            }
        }

        // 无需恢复
        RecoveryDecision::new(
            RecoveryAction::CooldownSuppressed,
            "healthy, no recovery needed".to_string(),
        )
    }

    /// 从LocalRepair状态决策
    fn decide_from_local_repair(&mut self, observation: RecoveryObservation) -> RecoveryDecision {
        // In-flight门控：如果IDR已在飞行中，coalesce
        if self.state_machine.is_idr_in_flight() {
            return RecoveryDecision::new(
                RecoveryAction::CoalescedKeyframeInFlight,
                "IDR already in flight".to_string(),
            )
            .with_coalescing(CoalescingMode::Merge);
        }

        // 检查NACK是否超时或repairability过低
        let should_escalate = self.state_machine.is_state_timeout()
            || observation.should_escalate_to_idr(self.repairability_threshold);

        if should_escalate {
            self.state_machine.transition_to_frame_recovery();
            self.state_machine.mark_idr_requested();
            return RecoveryDecision::new(
                RecoveryAction::RequestKeyframe,
                format!("NACK failed, escalate to IDR: {}", observation.reason_label),
            )
            .with_state_transition(RecoveryState::FrameRecovery)
            .with_preempt_reason("nack_timeout".to_string());
        }

        // 继续NACK
        RecoveryDecision::new(RecoveryAction::WaitForBurst, "NACK in progress".to_string())
    }

    /// 从FrameRecovery状态决策
    fn decide_from_frame_recovery(&mut self, observation: RecoveryObservation) -> RecoveryDecision {
        // 检查是否需要升级到decoder reset或reconnect
        if observation.requires_reconnect() {
            self.state_machine.transition_to_transport_recovery();
            self.state_machine.mark_reconnect_requested();
            return RecoveryDecision::new(
                RecoveryAction::RequestReconnectCandidate,
                format!("reconnect required: {}", observation.reason_label),
            )
            .with_state_transition(RecoveryState::TransportRecovery);
        }

        if observation.requires_decoder_reset() {
            self.state_machine.transition_to_decoder_recovery();
            self.state_machine.mark_decoder_reset_requested();
            return RecoveryDecision::new(
                RecoveryAction::RequestDecoderReset,
                format!("decoder reset required: {}", observation.reason_label),
            )
            .with_state_transition(RecoveryState::DecoderRecovery);
        }

        // In-flight门控
        if self.state_machine.is_idr_in_flight() {
            // 检查IDR是否超时
            if self.state_machine.is_state_timeout() {
                // 检查最小重试间隔
                if self.state_machine.can_retry_idr() {
                    self.state_machine.mark_idr_requested();
                    return RecoveryDecision::new(
                        RecoveryAction::RequestKeyframe,
                        "IDR timeout, retry immediately".to_string(),
                    )
                    .with_coalescing(CoalescingMode::Refresh)
                    .with_unlock_reason("timeout".to_string());
                } else {
                    return RecoveryDecision::new(
                        RecoveryAction::WaitForBurst,
                        "IDR timeout, waiting for min retry interval".to_string(),
                    );
                }
            } else {
                // IDR仍在飞行中，coalesce
                return RecoveryDecision::new(
                    RecoveryAction::CoalescedKeyframeInFlight,
                    "IDR in flight, coalescing".to_string(),
                )
                .with_coalescing(CoalescingMode::Merge);
            }
        }

        // IDR未在飞行中，发送新的IDR
        self.state_machine.mark_idr_requested();
        RecoveryDecision::new(
            RecoveryAction::RequestKeyframe,
            "request new IDR".to_string(),
        )
    }

    /// 从DecoderRecovery状态决策
    fn decide_from_decoder_recovery(
        &mut self,
        observation: RecoveryObservation,
    ) -> RecoveryDecision {
        // 检查是否需要升级到reconnect
        if observation.requires_reconnect() {
            self.state_machine.transition_to_transport_recovery();
            self.state_machine.mark_reconnect_requested();
            return RecoveryDecision::new(
                RecoveryAction::RequestReconnectCandidate,
                format!("reconnect required: {}", observation.reason_label),
            )
            .with_state_transition(RecoveryState::TransportRecovery);
        }

        // In-flight门控
        if self.state_machine.is_decoder_reset_in_flight() {
            // 检查decoder reset是否超时
            if self.state_machine.is_state_timeout() {
                // 超时，升级到reconnect
                self.state_machine.transition_to_transport_recovery();
                self.state_machine.mark_reconnect_requested();
                return RecoveryDecision::new(
                    RecoveryAction::RequestReconnectCandidate,
                    "decoder reset timeout, escalate to reconnect".to_string(),
                )
                .with_state_transition(RecoveryState::TransportRecovery)
                .with_preempt_reason("decoder_reset_timeout".to_string());
            } else {
                // Decoder reset仍在进行中，coalesce
                return RecoveryDecision::new(
                    RecoveryAction::CoalescedDecoderResetInFlight,
                    "decoder reset in flight, coalescing".to_string(),
                )
                .with_coalescing(CoalescingMode::Merge);
            }
        }

        // Decoder reset未在进行中，发送新的reset
        self.state_machine.mark_decoder_reset_requested();
        RecoveryDecision::new(
            RecoveryAction::RequestDecoderReset,
            "request decoder reset".to_string(),
        )
    }

    /// 从TransportRecovery状态决策
    fn decide_from_transport_recovery(
        &mut self,
        _observation: RecoveryObservation,
    ) -> RecoveryDecision {
        // 检查reconnect预算
        if !self.state_machine.current_budget().can_reconnect() {
            return RecoveryDecision::new(
                RecoveryAction::CooldownSuppressed,
                "reconnect budget exhausted".to_string(),
            );
        }

        // In-flight门控
        if self.state_machine.is_reconnect_in_flight() {
            return RecoveryDecision::new(
                RecoveryAction::CooldownSuppressed,
                "reconnect in flight, waiting".to_string(),
            );
        }

        // 发送reconnect
        self.state_machine.mark_reconnect_requested();
        RecoveryDecision::new(
            RecoveryAction::RequestReconnectCandidate,
            "request reconnect".to_string(),
        )
    }

    /// 检查是否应该回到Healthy状态
    pub(crate) fn check_healthy_transition(
        &mut self,
        has_clean_anchor: bool,
        stable_output: bool,
    ) -> bool {
        if has_clean_anchor && stable_output {
            self.state_machine.transition_to_healthy();
            true
        } else {
            false
        }
    }

    /// 更新恢复epoch
    pub(crate) fn update_recovery_epoch(&mut self, new_epoch: u64) {
        self.state_machine.update_recovery_epoch(new_epoch);
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
    fn test_healthy_to_local_repair() {
        let mut coordinator = ActionCoordinator::new(test_profile(), 1);

        let obs = RecoveryObservation::from_reason(
            VideoEscalationReason::TransportExpiredDeadline,
            "transportExpiredDeadline".to_string(),
            1000.0,
        )
        .with_repairability(0.6); // 高repairability，应该NACK

        let decision = coordinator.decide(obs);
        assert_eq!(decision.action, RecoveryAction::WaitForBurst);
        assert_eq!(coordinator.current_state(), RecoveryState::LocalRepair);
    }

    #[test]
    fn test_healthy_to_frame_recovery() {
        let mut coordinator = ActionCoordinator::new(test_profile(), 1);

        let obs = RecoveryObservation::from_reason(
            VideoEscalationReason::WaitKeyframe,
            "waitKeyframe".to_string(),
            1000.0,
        );

        let decision = coordinator.decide(obs);
        assert_eq!(decision.action, RecoveryAction::RequestKeyframe);
        assert_eq!(coordinator.current_state(), RecoveryState::FrameRecovery);
    }

    #[test]
    fn test_in_flight_coalescing() {
        let mut coordinator = ActionCoordinator::new(test_profile(), 1);

        // 第一次请求IDR
        let obs = RecoveryObservation::from_reason(
            VideoEscalationReason::WaitKeyframe,
            "waitKeyframe".to_string(),
            1000.0,
        );
        let decision = coordinator.decide(obs.clone());
        assert_eq!(decision.action, RecoveryAction::RequestKeyframe);

        // 第二次请求应该被coalesce
        let decision = coordinator.decide(obs);
        assert_eq!(decision.action, RecoveryAction::CoalescedKeyframeInFlight);
    }

    #[test]
    fn test_escalation_path() {
        let mut coordinator = ActionCoordinator::new(test_profile(), 1);

        // Healthy → FrameRecovery
        let obs = RecoveryObservation::from_reason(
            VideoEscalationReason::WaitKeyframe,
            "waitKeyframe".to_string(),
            1000.0,
        );
        coordinator.decide(obs);
        assert_eq!(coordinator.current_state(), RecoveryState::FrameRecovery);

        // FrameRecovery → DecoderRecovery
        let obs = RecoveryObservation::from_reason(
            VideoEscalationReason::DecoderBackendFailure,
            "decoderBackendFailure".to_string(),
            2000.0,
        );
        coordinator.decide(obs);
        assert_eq!(coordinator.current_state(), RecoveryState::DecoderRecovery);

        // DecoderRecovery → TransportRecovery
        let obs = RecoveryObservation::from_reason(
            VideoEscalationReason::LifecycleRecovering,
            "rtcConnectionRecovering".to_string(),
            3000.0,
        );
        coordinator.decide(obs);
        assert_eq!(
            coordinator.current_state(),
            RecoveryState::TransportRecovery
        );
    }

    #[test]
    fn test_reconnect_budget() {
        let mut coordinator = ActionCoordinator::new(test_profile(), 1);

        // 转到TransportRecovery
        coordinator
            .state_machine_mut()
            .transition_to_transport_recovery();

        // 第一次reconnect应该成功
        let obs = RecoveryObservation::from_reason(
            VideoEscalationReason::LifecycleRecovering,
            "rtcConnectionRecovering".to_string(),
            1000.0,
        );
        let decision = coordinator.decide(obs.clone());
        assert_eq!(decision.action, RecoveryAction::RequestReconnectCandidate);

        // 第二次reconnect应该被预算限制
        let decision = coordinator.decide(obs);
        assert_eq!(decision.action, RecoveryAction::CooldownSuppressed);
    }
}
