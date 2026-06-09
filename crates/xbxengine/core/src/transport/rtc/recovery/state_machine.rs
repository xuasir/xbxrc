//! 简化的恢复状态机
//!
//! 核心设计：
//! - 5个恢复状态：Healthy → LocalRepair → FrameRecovery → DecoderRecovery → TransportRecovery
//! - 单向状态转换（除了回到Healthy）
//! - 每状态单一超时，无重叠窗口
//! - 仅reconnect有预算限制，IDR/decoder reset通过门控机制防风暴

use std::time::Instant;

use super::policy::RecoveryScenarioProfile;
use super::timing::{
    default_rtt_ms_for_kind, resolve_recovery_dynamic_timing_with_rtt, RecoveryDynamicTiming,
};

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
        self.reconnect_used = self.reconnect_used.saturating_add(1);
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
    /// PLI refresh 最小间隔（FrameRecovery 状态的动作窗）
    pub(crate) idr_refresh_interval_ms: f64,
    /// IDR响应超时（FrameRecovery 状态的响应/升级等待窗）
    pub(crate) idr_response_timeout_ms: f64,
    /// Decoder reset超时（DecoderRecovery状态）
    pub(crate) decoder_reset_timeout_ms: f64,
    /// Reconnect超时（TransportRecovery状态）
    pub(crate) reconnect_timeout_ms: f64,
}

impl StateTimeouts {
    /// 从场景化 profile 创建超时配置（使用场景默认 RTT，与动态解析器一致）。
    pub(crate) fn from_profile(profile: RecoveryScenarioProfile) -> Self {
        let timing = resolve_recovery_dynamic_timing_with_rtt(
            default_rtt_ms_for_kind(profile.kind),
            profile,
        );
        Self::from_recovery_dynamic_timing(&timing)
    }

    pub(crate) fn from_recovery_dynamic_timing(timing: &RecoveryDynamicTiming) -> Self {
        Self {
            nack_timeout_ms: timing.nack_timeout_ms,
            idr_refresh_interval_ms: timing.pli_refresh_interval_ms,
            idr_response_timeout_ms: 900.0,
            decoder_reset_timeout_ms: 1200.0,
            reconnect_timeout_ms: 5000.0,
        }
    }

    /// 获取当前状态的超时时间
    pub(crate) fn timeout_for_state(&self, state: RecoveryState) -> Option<f64> {
        match state {
            RecoveryState::Healthy => None,
            RecoveryState::LocalRepair => Some(self.nack_timeout_ms),
            RecoveryState::FrameRecovery => Some(self.idr_response_timeout_ms),
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
    /// 关键帧请求仍在等待远端响应
    keyframe_request_in_flight: bool,
    /// 已观察到关键帧/owner 响应，正在等待 decode 落地
    keyframe_decode_pending: bool,
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
            keyframe_request_in_flight: false,
            keyframe_decode_pending: false,
            decoder_reset_in_flight: false,
            reconnect_in_flight: false,
        }
    }

    /// 每拍刷新 LocalRepair / FrameRecovery 相关超时，使 NACK/PLI 间隔随 RTT 变化。
    pub(crate) fn apply_recovery_dynamic_timing(&mut self, timing: &RecoveryDynamicTiming) {
        let next = StateTimeouts::from_recovery_dynamic_timing(timing);
        self.timeouts.nack_timeout_ms = next.nack_timeout_ms;
        self.timeouts.idr_refresh_interval_ms = next.idr_refresh_interval_ms;
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
            elapsed_ms >= self.timeouts.idr_refresh_interval_ms
        } else {
            true
        }
    }

    pub(crate) fn idr_response_timeout_elapsed(&self) -> bool {
        self.state == RecoveryState::FrameRecovery && self.is_state_timeout()
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
        self.keyframe_request_in_flight = false;
        self.keyframe_decode_pending = false;
        self.decoder_reset_in_flight = false;
        self.reconnect_in_flight = false;
    }

    /// 转换到LocalRepair状态
    pub(crate) fn transition_to_local_repair(&mut self) {
        self.transition_to(RecoveryState::LocalRepair);
    }

    /// 转换到FrameRecovery状态
    #[cfg(test)]
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
    #[cfg(test)]
    pub(crate) fn mark_idr_requested(&mut self) {
        self.last_idr_request_at = Some(Instant::now());
        self.keyframe_request_in_flight = true;
        self.keyframe_decode_pending = false;
    }

    /// 标记已经观察到关键帧响应，等待 decode 完成
    pub(crate) fn mark_idr_response_observed(&mut self) {
        self.keyframe_request_in_flight = false;
        self.keyframe_decode_pending = true;
    }

    /// 标记IDR已解码完成
    pub(crate) fn mark_idr_decoded(&mut self) {
        self.keyframe_request_in_flight = false;
        self.keyframe_decode_pending = false;
    }

    /// 检查关键帧请求是否仍在等待远端响应
    pub(crate) fn is_keyframe_request_in_flight(&self) -> bool {
        self.keyframe_request_in_flight
    }

    /// 检查是否已经看到 owner/anchor 响应但仍等待 decode 完成
    pub(crate) fn is_keyframe_decode_pending(&self) -> bool {
        self.keyframe_decode_pending
    }

    /// 兼容旧调用口径：只表示“关键帧恢复链仍未完成”
    pub(crate) fn is_idr_in_flight(&self) -> bool {
        self.keyframe_request_in_flight || self.keyframe_decode_pending
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
            self.keyframe_request_in_flight = false;
            self.keyframe_decode_pending = false;
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
    fn test_reconnect_budget_consumption_is_saturating() {
        let mut sm = RecoveryStateMachine::new(test_profile(), 1);
        sm.budget.reconnect_used = u8::MAX;

        sm.mark_reconnect_requested();

        assert_eq!(sm.current_budget().reconnect_used, u8::MAX);
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
        assert!(sm.is_keyframe_request_in_flight());
        assert!(!sm.is_keyframe_decode_pending());
        sm.mark_idr_response_observed();
        assert!(!sm.is_keyframe_request_in_flight());
        assert!(sm.is_keyframe_decode_pending());
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

        // 立即重试应该被阻止（需要等待短 refresh 窗）
        assert!(!sm.can_retry_idr());

        // 等待后应该可以重试
        std::thread::sleep(std::time::Duration::from_millis(120));
        assert!(sm.can_retry_idr());
    }

    #[test]
    fn test_idr_response_timeout_is_longer_than_refresh_interval() {
        let mut sm = RecoveryStateMachine::new(test_profile(), 1);
        sm.transition_to_frame_recovery();
        sm.mark_idr_requested();

        std::thread::sleep(std::time::Duration::from_millis(120));
        assert!(sm.can_retry_idr());
        assert!(!sm.idr_response_timeout_elapsed());
    }
}
