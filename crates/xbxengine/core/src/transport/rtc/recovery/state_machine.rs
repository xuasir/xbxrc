//! 简化的恢复状态机
//!
//! 核心设计：
//! - 5个恢复状态：Healthy → LocalRepair → FrameRecovery → DecoderRecovery → TransportRecovery
//! - 单向状态转换（除了回到Healthy）
//! - 每状态单一超时，无重叠窗口
//! - 仅reconnect有预算限制，IDR/decoder reset通过门控机制防风暴

use std::time::Instant;

use super::policy::RecoveryScenarioProfile;

/// 恢复状态
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RecoveryState {
    /// 健康状态，正常播放
    Healthy,
    /// 本地修复（NACK活跃）
    LocalRepair,
    /// 帧恢复（RFI/IDR已请求）
    FrameRecovery,
    /// 解码恢复（解码器reset进行中）
    DecoderRecovery,
    /// 传输恢复（重连）
    TransportRecovery,
}

/// 恢复预算（仅reconnect有预算限制）
#[derive(Clone, Copy, Debug)]
pub(crate) struct RecoveryBudget {
    /// 当前恢复epoch
    pub(crate) recovery_epoch: u64,
    /// Reconnect已使用预算
    pub(crate) reconnect_used: u8,
    /// Reconnect预算限制（每epoch）
    pub(crate) reconnect_limit: u8,
}

impl RecoveryBudget {
    pub(crate) fn new(recovery_epoch: u64) -> Self {
        Self {
            recovery_epoch,
            reconnect_used: 0,
            reconnect_limit: 1, // 每epoch仅1次重连
        }
    }

    /// 检查reconnect预算是否可用
    pub(crate) fn can_reconnect(&self) -> bool {
        self.reconnect_used < self.reconnect_limit
    }

    /// 消耗reconnect预算
    pub(crate) fn consume_reconnect(&mut self) {
        self.reconnect_used += 1;
    }

    /// 重置预算（新epoch）
    pub(crate) fn reset(&mut self, new_epoch: u64) {
        self.recovery_epoch = new_epoch;
        self.reconnect_used = 0;
    }
}

/// 状态超时配置（从场景化profile读取）
#[derive(Clone, Copy, Debug)]
pub(crate) struct StateTimeouts {
    /// NACK超时（LocalRepair状态）
    pub(crate) nack_timeout_ms: f64,
    /// IDR超时（FrameRecovery状态）
    pub(crate) idr_timeout_ms: f64,
    /// Decoder reset超时（DecoderRecovery状态）
    pub(crate) decoder_reset_timeout_ms: f64,
    /// Reconnect超时（TransportRecovery状态）
    pub(crate) reconnect_timeout_ms: f64,
    /// IDR最小重试间隔（防死锁）
    pub(crate) idr_min_retry_interval_ms: f64,
}

impl StateTimeouts {
    /// 从场景化profile创建超时配置
    pub(crate) fn from_profile(profile: RecoveryScenarioProfile) -> Self {
        // 根据场景调整超时参数
        let (nack_timeout_ms, idr_timeout_ms) = match profile.kind {
            crate::transport::rtc::recovery::policy::ScenarioPolicyProfileKind::CloudGaming => {
                (300.0, 900.0) // Cloud: 更长的超时
            }
            _ => {
                (180.0, 900.0) // Home/Relay: 较短的NACK超时
            }
        };

        Self {
            nack_timeout_ms,
            idr_timeout_ms,
            decoder_reset_timeout_ms: 1200.0,
            reconnect_timeout_ms: 5000.0,
            idr_min_retry_interval_ms: 50.0,
        }
    }

    /// 获取当前状态的超时时间
    pub(crate) fn timeout_for_state(&self, state: RecoveryState) -> Option<f64> {
        match state {
            RecoveryState::Healthy => None,
            RecoveryState::LocalRepair => Some(self.nack_timeout_ms),
            RecoveryState::FrameRecovery => Some(self.idr_timeout_ms),
            RecoveryState::DecoderRecovery => Some(self.decoder_reset_timeout_ms),
            RecoveryState::TransportRecovery => Some(self.reconnect_timeout_ms),
        }
    }
}

/// 恢复状态机
pub(crate) struct RecoveryStateMachine {
    /// 当前状态
    state: RecoveryState,
    /// 状态进入时间
    state_entered_at: Instant,
    /// 预算跟踪
    budget: RecoveryBudget,
    /// 超时配置
    timeouts: StateTimeouts,
    /// 最后一次IDR请求时间（用于最小间隔检查）
    last_idr_request_at: Option<Instant>,
    /// IDR in-flight标志
    idr_in_flight: bool,
    /// Decoder reset in-flight标志
    decoder_reset_in_flight: bool,
    /// Reconnect in-flight标志
    reconnect_in_flight: bool,
}

impl RecoveryStateMachine {
    /// 创建新的状态机
    pub(crate) fn new(profile: RecoveryScenarioProfile, recovery_epoch: u64) -> Self {
        Self {
            state: RecoveryState::Healthy,
            state_entered_at: Instant::now(),
            budget: RecoveryBudget::new(recovery_epoch),
            timeouts: StateTimeouts::from_profile(profile),
            last_idr_request_at: None,
            idr_in_flight: false,
            decoder_reset_in_flight: false,
            reconnect_in_flight: false,
        }
    }

    /// 获取当前状态
    pub(crate) fn current_state(&self) -> RecoveryState {
        self.state
    }

    /// 获取当前预算
    pub(crate) fn current_budget(&self) -> &RecoveryBudget {
        &self.budget
    }

    /// 获取状态停留时间（毫秒）
    pub(crate) fn state_duration_ms(&self) -> f64 {
        self.state_entered_at.elapsed().as_secs_f64() * 1000.0
    }

    /// 检查当前状态是否超时
    pub(crate) fn is_state_timeout(&self) -> bool {
        if let Some(timeout_ms) = self.timeouts.timeout_for_state(self.state) {
            self.state_duration_ms() >= timeout_ms
        } else {
            false
        }
    }

    /// 检查IDR最小重试间隔
    pub(crate) fn can_retry_idr(&self) -> bool {
        if let Some(last_request) = self.last_idr_request_at {
            let elapsed_ms = last_request.elapsed().as_secs_f64() * 1000.0;
            elapsed_ms >= self.timeouts.idr_min_retry_interval_ms
        } else {
            true
        }
    }

    /// 转换到新状态
    fn transition_to(&mut self, new_state: RecoveryState) {
        if self.state != new_state {
            self.state = new_state;
            self.state_entered_at = Instant::now();
        }
    }

    /// 转换到Healthy状态
    pub(crate) fn transition_to_healthy(&mut self) {
        self.transition_to(RecoveryState::Healthy);
        // 清除in-flight标志
        self.idr_in_flight = false;
        self.decoder_reset_in_flight = false;
        self.reconnect_in_flight = false;
    }

    /// 转换到LocalRepair状态
    pub(crate) fn transition_to_local_repair(&mut self) {
        self.transition_to(RecoveryState::LocalRepair);
    }

    /// 转换到FrameRecovery状态
    pub(crate) fn transition_to_frame_recovery(&mut self) {
        self.transition_to(RecoveryState::FrameRecovery);
    }

    /// 转换到DecoderRecovery状态
    pub(crate) fn transition_to_decoder_recovery(&mut self) {
        self.transition_to(RecoveryState::DecoderRecovery);
    }

    /// 转换到TransportRecovery状态
    pub(crate) fn transition_to_transport_recovery(&mut self) {
        self.transition_to(RecoveryState::TransportRecovery);
    }

    /// 标记IDR请求已发送
    pub(crate) fn mark_idr_requested(&mut self) {
        self.last_idr_request_at = Some(Instant::now());
        self.idr_in_flight = true;
    }

    /// 标记IDR已解码完成
    pub(crate) fn mark_idr_decoded(&mut self) {
        self.idr_in_flight = false;
    }

    /// 检查IDR是否in-flight
    pub(crate) fn is_idr_in_flight(&self) -> bool {
        self.idr_in_flight
    }

    /// 标记decoder reset已发送
    pub(crate) fn mark_decoder_reset_requested(&mut self) {
        self.decoder_reset_in_flight = true;
    }

    /// 标记decoder reset已完成
    pub(crate) fn mark_decoder_reset_completed(&mut self) {
        self.decoder_reset_in_flight = false;
    }

    /// 检查decoder reset是否in-flight
    pub(crate) fn is_decoder_reset_in_flight(&self) -> bool {
        self.decoder_reset_in_flight
    }

    /// 标记reconnect已发送
    pub(crate) fn mark_reconnect_requested(&mut self) {
        self.reconnect_in_flight = true;
        self.budget.consume_reconnect();
    }

    /// 标记reconnect已完成
    pub(crate) fn mark_reconnect_completed(&mut self) {
        self.reconnect_in_flight = false;
    }

    /// 检查reconnect是否in-flight
    pub(crate) fn is_reconnect_in_flight(&self) -> bool {
        self.reconnect_in_flight
    }

    /// 更新恢复epoch（重置预算）
    pub(crate) fn update_recovery_epoch(&mut self, new_epoch: u64) {
        if self.budget.recovery_epoch != new_epoch {
            self.budget.reset(new_epoch);
            // 新恢复轮次必须丢弃上一轮的 in-flight / timeout 链，否则旧 epoch 的
            // keyframe / decoder reset / reconnect 会继续压住当前轮次的首拍恢复动作。
            self.state = RecoveryState::Healthy;
            self.state_entered_at = Instant::now();
            self.last_idr_request_at = None;
            self.idr_in_flight = false;
            self.decoder_reset_in_flight = false;
            self.reconnect_in_flight = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_profile() -> RecoveryScenarioProfile {
        crate::transport::rtc::recovery::policy::ScenarioPolicyResolver::resolve_recovery_profile_by_kind(
            crate::transport::rtc::recovery::policy::ScenarioPolicyProfileKind::HomeLanGaming,
        )
    }

    #[test]
    fn test_state_transitions() {
        let mut sm = RecoveryStateMachine::new(test_profile(), 1);

        assert_eq!(sm.current_state(), RecoveryState::Healthy);

        sm.transition_to_local_repair();
        assert_eq!(sm.current_state(), RecoveryState::LocalRepair);

        sm.transition_to_frame_recovery();
        assert_eq!(sm.current_state(), RecoveryState::FrameRecovery);

        sm.transition_to_decoder_recovery();
        assert_eq!(sm.current_state(), RecoveryState::DecoderRecovery);

        sm.transition_to_transport_recovery();
        assert_eq!(sm.current_state(), RecoveryState::TransportRecovery);

        sm.transition_to_healthy();
        assert_eq!(sm.current_state(), RecoveryState::Healthy);
    }

    #[test]
    fn test_budget_tracking() {
        let mut sm = RecoveryStateMachine::new(test_profile(), 1);

        assert!(sm.current_budget().can_reconnect());

        sm.mark_reconnect_requested();
        assert!(!sm.current_budget().can_reconnect());

        // 新epoch重置预算
        sm.update_recovery_epoch(2);
        assert!(sm.current_budget().can_reconnect());
    }

    #[test]
    fn test_epoch_rotation_clears_previous_recovery_chain() {
        let mut sm = RecoveryStateMachine::new(test_profile(), 1);
        sm.transition_to_frame_recovery();
        sm.mark_idr_requested();
        sm.mark_decoder_reset_requested();
        sm.mark_reconnect_requested();

        sm.update_recovery_epoch(2);

        assert_eq!(sm.current_state(), RecoveryState::Healthy);
        assert!(!sm.is_idr_in_flight());
        assert!(!sm.is_decoder_reset_in_flight());
        assert!(!sm.is_reconnect_in_flight());
        assert_eq!(sm.current_budget().recovery_epoch, 2);
        assert_eq!(sm.current_budget().reconnect_used, 0);
    }

    #[test]
    fn test_in_flight_gates() {
        let mut sm = RecoveryStateMachine::new(test_profile(), 1);

        assert!(!sm.is_idr_in_flight());
        sm.mark_idr_requested();
        assert!(sm.is_idr_in_flight());
        sm.mark_idr_decoded();
        assert!(!sm.is_idr_in_flight());

        assert!(!sm.is_decoder_reset_in_flight());
        sm.mark_decoder_reset_requested();
        assert!(sm.is_decoder_reset_in_flight());
        sm.mark_decoder_reset_completed();
        assert!(!sm.is_decoder_reset_in_flight());
    }

    #[test]
    fn test_idr_retry_interval() {
        let mut sm = RecoveryStateMachine::new(test_profile(), 1);

        assert!(sm.can_retry_idr());
        sm.mark_idr_requested();

        // 立即重试应该被阻止（需要等待50ms）
        assert!(!sm.can_retry_idr());

        // 等待后应该可以重试
        std::thread::sleep(std::time::Duration::from_millis(60));
        assert!(sm.can_retry_idr());
    }
}
