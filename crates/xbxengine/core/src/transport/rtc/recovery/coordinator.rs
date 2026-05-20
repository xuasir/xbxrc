//! 恢复协调器
//!
//! 统一恢复模型的协调器，负责：
//! - 管理状态驱动的恢复决策
//! - 跟踪 in-flight 状态并自动更新
//! - 提供统一的 CoordinatorProposal 接口

use std::sync::Mutex;

use crate::api::backend::XbxEngineMediaRuntimeStats;
use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::receive::suppress_session_picture_recovery_action;
use crate::transport::rtc::recovery::contract::{CoalescingMode, GapSeverity};
use crate::transport::rtc::recovery::escalation::{
    RecoveryAction, RecoveryActionBudgetState, VideoEscalationController, VideoEscalationDecision,
    VideoEscalationReason,
};
use crate::transport::rtc::recovery::observation::{RecoveryObservation, RecoverySeverity};
use crate::transport::rtc::recovery::runtime_state::resolve_runtime_recovery_profile;
use crate::transport::rtc::recovery::state_coordinator::{
    RecoveryDecision, StateRecoveryCoordinator,
};
use crate::transport::rtc::recovery::timing::{
    advance_recovery_rtt_smoothing, publish_recovery_timing_to_stats,
    resolve_recovery_dynamic_timing,
};

/// Owner 信号
#[derive(Clone, Debug)]
pub(crate) struct RecoveryOwnerSignal {
    pub(crate) reason: VideoEscalationReason,
    pub(crate) reason_label: String,
    pub(crate) observed_at_ms: f64,
    pub(crate) gap_severity: Option<GapSeverity>,
    pub(crate) repairability: Option<f64>,
}

/// Coordinator 提案输出（统一架构）
#[derive(Clone, Debug)]
pub(crate) struct CoordinatorProposal {
    /// 恢复动作决策
    pub(crate) decision: VideoEscalationDecision,

    /// 合并模式（Merge/Refresh/Preempt），None 表示无 in-flight episode
    pub(crate) coalescing_mode: Option<CoalescingMode>,

    /// 解锁原因（如果解锁了 in-flight episode）
    pub(crate) unlock_reason: Option<String>,

    /// 抢占原因（如果抢占了旧 episode）
    pub(crate) preempt_reason: Option<String>,

    /// 决策前的预算状态
    pub(crate) budget_before: RecoveryActionBudgetState,

    /// 决策后的预算状态
    pub(crate) budget_after: RecoveryActionBudgetState,
}

/// 恢复协调器，提供统一的 CoordinatorProposal 接口
pub(crate) struct RecoveryCoordinator {
    coordinator: StateRecoveryCoordinator,
    escalation_controller: VideoEscalationController,
    /// 上次检查的当前图片恢复“远端已响应”时间。
    last_keyframe_response_observed_at_ms: Option<f64>,
    /// 上次检查的当前图片恢复“本地已解码”时间。
    last_keyframe_decoded_at_ms: Option<f64>,
    /// 上次检查的decoder reset时间（用于检测新的reset完成事件）
    last_decoder_reset_at_ms: Option<f64>,
    /// 上次检查的连接状态（用于检测reconnect完成）
    last_connected: Option<bool>,
    /// 已对哪个 transport-await episode.requested_at_ms 执行过“释放旧 keyframe epoch 并重开 PLI”。
    last_transport_await_reopen_requested_at_ms: Option<f64>,
}

impl RecoveryCoordinator {
    pub(crate) fn new(coordinator: StateRecoveryCoordinator) -> Self {
        let mut escalation_controller =
            VideoEscalationController::new(coordinator.current_profile().escalation_config());
        escalation_controller
            .begin_recovery_epoch(coordinator.state_machine().current_budget().recovery_epoch);
        Self {
            escalation_controller,
            coordinator,
            last_keyframe_response_observed_at_ms: None,
            last_keyframe_decoded_at_ms: None,
            last_decoder_reset_at_ms: None,
            last_connected: None,
            last_transport_await_reopen_requested_at_ms: None,
        }
    }

    fn refresh_recovery_timing_snapshot(
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
    ) -> Option<crate::transport::rtc::recovery::timing::RecoveryDynamicTiming> {
        RuntimeStatsSink::update_shared(runtime_stats, |stats| {
            advance_recovery_rtt_smoothing(stats);
            let profile = resolve_runtime_recovery_profile(stats);
            let timing = resolve_recovery_dynamic_timing(stats, profile);
            publish_recovery_timing_to_stats(stats, &timing);
        });
        RuntimeStatsSink::read_shared(runtime_stats, |stats| {
            let profile = resolve_runtime_recovery_profile(stats);
            resolve_recovery_dynamic_timing(stats, profile)
        })
    }

    fn apply_recovery_timing_from_stats(
        &mut self,
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
    ) {
        if let Some(timing) = Self::refresh_recovery_timing_snapshot(runtime_stats) {
            self.coordinator.apply_recovery_dynamic_timing(&timing);
        }
    }

    /// 从owner signal生成恢复提案
    pub(crate) fn propose_from_owner_signal(
        &mut self,
        signal: RecoveryOwnerSignal,
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
    ) -> CoordinatorProposal {
        let recovery_epoch = self.sync_recovery_epoch(runtime_stats);
        // 在生成proposal前，先检查并更新in-flight状态
        self.update_inflight_status(runtime_stats);
        self.apply_recovery_timing_from_stats(runtime_stats);
        if let Some(proposal) = self.maybe_resolve_connectivity_local_repair(&signal) {
            return proposal;
        }
        if let Some(proposal) =
            self.maybe_reopen_stale_transport_await_keyframe(&signal, runtime_stats, recovery_epoch)
        {
            return proposal;
        }
        if signal.reason == VideoEscalationReason::TransportAwaitRecoveryKeyframe {
            let budget = self.snapshot_budget_state();
            return self.convert_escalation_decision_to_proposal(
                VideoEscalationDecision {
                    observation_id: 0,
                    action: RecoveryAction::CooldownSuppressed,
                },
                budget,
                budget,
            );
        }
        if Self::uses_policy_controlled_escalation(signal.reason) {
            let budget_before = self.snapshot_budget_state();
            let mut decision = self.escalation_controller.on_reason_with_epoch_policy(
                signal.reason,
                recovery_epoch,
                true,
                true,
                true,
                true,
            );
            if signal.reason == VideoEscalationReason::LifecycleRecovering
                && decision.action == RecoveryAction::RequestReconnectCandidate
                && self.state_machine().is_reconnect_in_flight()
            {
                decision.action = RecoveryAction::CooldownSuppressed;
            }
            self.sync_connectivity_escalation_state(&decision);
            let budget_after = self.snapshot_budget_state();
            return self.convert_escalation_decision_to_proposal(
                decision,
                budget_before,
                budget_after,
            );
        }

        let budget_before = self.snapshot_budget_state();

        let mut observation = RecoveryObservation::from_reason(
            signal.reason,
            signal.reason_label.clone(),
            signal.observed_at_ms,
        );
        if let Some(gap_severity) = signal.gap_severity {
            observation = observation.with_gap_severity(gap_severity);
        }
        if let Some(repairability) = signal.repairability {
            observation = observation.with_repairability(repairability);
        }

        // 调用新系统
        let decision = self.coordinator.on_observation(observation);
        let budget_after = self.snapshot_budget_state();

        // 转换为统一的CoordinatorProposal格式
        self.convert_to_proposal(decision, budget_before, budget_after)
    }

    fn sync_recovery_epoch(&mut self, runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>) -> u64 {
        let recovery_epoch =
            RuntimeStatsSink::read_shared(runtime_stats, |stats| stats.transport_recovery_epoch)
                .unwrap_or(0);
        if self
            .coordinator
            .state_machine()
            .current_budget()
            .recovery_epoch
            != recovery_epoch
        {
            self.last_transport_await_reopen_requested_at_ms = None;
        }
        self.coordinator.update_recovery_epoch(recovery_epoch);
        self.escalation_controller
            .begin_recovery_epoch(recovery_epoch);
        recovery_epoch
    }

    fn maybe_reopen_stale_transport_await_keyframe(
        &mut self,
        _signal: &RecoveryOwnerSignal,
        _runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        _recovery_epoch: u64,
    ) -> Option<CoordinatorProposal> {
        None
    }

    fn maybe_resolve_connectivity_local_repair(
        &mut self,
        signal: &RecoveryOwnerSignal,
    ) -> Option<CoordinatorProposal> {
        if !matches!(
            signal.reason,
            VideoEscalationReason::TransportExpiredDeadline
                | VideoEscalationReason::TransportSampleLoss
        ) {
            return None;
        }

        let mut observation = RecoveryObservation::from_reason(
            signal.reason,
            signal.reason_label.clone(),
            signal.observed_at_ms,
        );
        if let Some(gap_severity) = signal.gap_severity {
            observation = observation.with_gap_severity(gap_severity);
        }
        if let Some(repairability) = signal.repairability {
            observation = observation.with_repairability(repairability);
        }

        if observation.severity != RecoverySeverity::PacketLoss
            || observation.should_escalate_to_idr(0.45)
        {
            return None;
        }

        let budget_before = self.snapshot_budget_state();
        let decision = self.coordinator.on_observation(observation);
        let budget_after = self.snapshot_budget_state();
        Some(self.convert_to_proposal(decision, budget_before, budget_after))
    }

    /// 基于runtime_stats中的执行事实更新in-flight状态
    fn update_inflight_status(&mut self, runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>) {
        RuntimeStatsSink::read_shared(runtime_stats, |stats| {
            let is_connected =
                stats.transport_state == xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            // 1. 检查关键帧恢复链是否出现“远端响应已到”和“本地解码已完成”新进展
            if self.coordinator.state_machine().is_idr_in_flight() {
                if let Some(response_observed_at_ms) =
                    Self::latest_transport_await_response_observed_at_ms(stats)
                {
                    if self
                        .last_keyframe_response_observed_at_ms
                        .map_or(true, |last| response_observed_at_ms > last)
                    {
                        self.coordinator.on_idr_response_observed();
                        self.last_keyframe_response_observed_at_ms = Some(response_observed_at_ms);
                    }
                }

                if let Some(decoded_at_ms) = Self::latest_transport_await_decoded_at_ms(stats) {
                    if self
                        .last_keyframe_decoded_at_ms
                        .map_or(true, |last| decoded_at_ms > last)
                    {
                        self.coordinator.on_idr_decoded();
                        self.last_keyframe_decoded_at_ms = Some(decoded_at_ms);
                        self.last_keyframe_response_observed_at_ms = Some(
                            self.last_keyframe_response_observed_at_ms
                                .map_or(decoded_at_ms, |last| last.max(decoded_at_ms)),
                        );
                    }
                }
            }

            // 2. 检查decoder reset是否完成
            if self
                .coordinator
                .state_machine()
                .is_decoder_reset_in_flight()
            {
                let should_clear =
                    Self::check_decoder_reset_completed(stats, self.last_decoder_reset_at_ms);

                if should_clear {
                    self.coordinator.on_decoder_reset_completed();
                    // 更新最后检查的时间
                    self.last_decoder_reset_at_ms = stats.latest_video_decoder_reset_time_ms;
                }
            }

            // 3. 检查reconnect是否完成
            if self.coordinator.state_machine().is_reconnect_in_flight() {
                let should_clear = Self::check_reconnect_completed(
                    stats,
                    self.last_connected.unwrap_or(is_connected),
                );

                if should_clear {
                    self.coordinator.on_reconnect_completed();
                }
            }
            self.last_connected = Some(is_connected);
        });
    }

    /// 检查IDR是否完成（基于执行事实）
    #[cfg(test)]
    fn check_idr_completed(
        stats: &XbxEngineMediaRuntimeStats,
        last_checked_at_ms: Option<f64>,
    ) -> bool {
        // 证据1：当前 epoch 已有 clean anchor（最强证据）
        let has_clean_anchor = stats
            .video_anchor_clean_epoch
            .is_some_and(|epoch| epoch == stats.transport_recovery_epoch);

        if has_clean_anchor {
            return true;
        }

        Self::latest_transport_await_decoded_at_ms(stats)
            .is_some_and(|decoded_at| last_checked_at_ms.map_or(true, |last| decoded_at > last))
    }

    fn latest_transport_await_response_observed_at_ms(
        stats: &XbxEngineMediaRuntimeStats,
    ) -> Option<f64> {
        let episode = stats.latest_keyframe_request_episode.as_ref()?;
        if episode.request_reason.as_deref() != Some("receiverWaitingKeyframe") {
            return None;
        }
        if episode.retired_at_ms.is_some() {
            return None;
        }
        episode.first_keyframe_packet_at_ms
    }

    fn latest_transport_await_decoded_at_ms(stats: &XbxEngineMediaRuntimeStats) -> Option<f64> {
        let episode = stats.latest_keyframe_request_episode.as_ref()?;
        if episode.request_reason.as_deref() != Some("receiverWaitingKeyframe") {
            return None;
        }
        if episode.retired_at_ms.is_some() {
            return None;
        }
        episode.first_keyframe_decoded_at_ms.or_else(|| {
            stats
                .video_anchor_clean_epoch
                .filter(|epoch| *epoch == stats.transport_recovery_epoch)
                .and(stats.video_anchor_clean_observed_at_ms)
        })
    }

    /// 检查decoder reset是否完成（基于执行事实）
    fn check_decoder_reset_completed(
        stats: &XbxEngineMediaRuntimeStats,
        last_checked_reset_at_ms: Option<f64>,
    ) -> bool {
        // 获取最近的reset时间
        let reset_at_ms = match stats.latest_video_decoder_reset_time_ms {
            Some(t) => t,
            None => return false, // 没有reset记录，不清除
        };

        // 如果这是我们已经处理过的reset，不重复清除
        if last_checked_reset_at_ms.is_some_and(|last| reset_at_ms <= last) {
            return false;
        }

        // 证据1：reset后有IDR帧解码（最强证据）
        let has_post_reset_idr = stats
            .latest_keyframe_request_episode
            .as_ref()
            .and_then(|ep| ep.first_keyframe_decoded_at_ms)
            .is_some_and(|decoded_at| decoded_at > reset_at_ms);

        if has_post_reset_idr {
            return true;
        }

        // 证据2：reset后有任意帧解码（说明解码器恢复工作）
        let has_post_reset_decode = stats
            .latest_video_decode_ok_time_ms
            .is_some_and(|decode_at| decode_at > reset_at_ms);

        if has_post_reset_decode {
            return true;
        }

        // 证据3：有clean anchor
        let has_clean_anchor = stats
            .video_anchor_clean_epoch
            .is_some_and(|epoch| epoch == stats.transport_recovery_epoch);

        if has_clean_anchor {
            return true;
        }

        false
    }

    /// 检查reconnect是否完成（基于执行事实）
    fn check_reconnect_completed(stats: &XbxEngineMediaRuntimeStats, was_connected: bool) -> bool {
        let is_connected =
            stats.transport_state == xbxengine_protocol::XbxEngineTransportStateDto::Connected;

        // 证据2：有clean anchor（说明恢复成功）
        let has_clean_anchor = stats
            .video_anchor_clean_epoch
            .is_some_and(|epoch| epoch == stats.transport_recovery_epoch);

        if has_clean_anchor {
            return true;
        }

        let episode_opened_at_ms = stats.transport_recovery_episode_opened_at_ms;
        let has_post_reconnect_decode =
            stats
                .latest_video_decode_ok_time_ms
                .is_some_and(|decode_at| {
                    episode_opened_at_ms.map_or(!was_connected && is_connected, |opened_at| {
                        decode_at > opened_at
                    })
                });
        if is_connected && has_post_reconnect_decode {
            return true;
        }

        let has_post_reconnect_present =
            stats
                .latest_video_host_present_time_ms
                .is_some_and(|present_at| {
                    episode_opened_at_ms.map_or(!was_connected && is_connected, |opened_at| {
                        present_at > opened_at
                    })
                });
        if is_connected && has_post_reconnect_present {
            return true;
        }

        false
    }

    /// 获取状态机的不可变引用（用于外部查询）
    fn state_machine(
        &self,
    ) -> &crate::transport::rtc::recovery::state_machine::RecoveryStateMachine {
        self.coordinator.state_machine()
    }

    /// 获取状态机的可变引用（用于标记请求）
    fn state_machine_mut(
        &mut self,
    ) -> &mut crate::transport::rtc::recovery::state_machine::RecoveryStateMachine {
        self.coordinator.state_machine_mut()
    }

    fn uses_policy_controlled_escalation(reason: VideoEscalationReason) -> bool {
        matches!(
            reason,
            VideoEscalationReason::WaitKeyframe
                | VideoEscalationReason::TransportAwaitRecoveryKeyframe
                | VideoEscalationReason::LocalSupplySuspect
                | VideoEscalationReason::DisplaySupplyCritical
                | VideoEscalationReason::AdapterIdleTimeout
                | VideoEscalationReason::AdapterThinStream
                | VideoEscalationReason::Reconfigure
                | VideoEscalationReason::DecoderBackendFailure
                | VideoEscalationReason::LifecycleRecovering
                | VideoEscalationReason::TransportExpiredDeadline
                | VideoEscalationReason::TransportSevereDeadline
                | VideoEscalationReason::TransportRecoveredLate
                | VideoEscalationReason::TransportSampleLoss
        )
    }

    fn sync_connectivity_escalation_state(&mut self, decision: &VideoEscalationDecision) {
        match decision.action {
            RecoveryAction::RequestPli | RecoveryAction::RequestFir => {
                let state_machine = self.state_machine_mut();
                state_machine.transition_to_frame_recovery();
                state_machine.mark_idr_requested();
            }
            RecoveryAction::RequestDecoderReset => {
                let state_machine = self.state_machine_mut();
                state_machine.transition_to_decoder_recovery();
                state_machine.mark_decoder_reset_requested();
            }
            RecoveryAction::RequestReconnectCandidate => {
                let state_machine = self.state_machine_mut();
                state_machine.transition_to_transport_recovery();
            }
            RecoveryAction::CoalescedKeyframeInFlight => {
                let state_machine = self.state_machine_mut();
                state_machine.transition_to_frame_recovery();
            }
            RecoveryAction::CoalescedDecoderResetInFlight => {
                let state_machine = self.state_machine_mut();
                state_machine.transition_to_decoder_recovery();
            }
            RecoveryAction::WaitForBurst
            | RecoveryAction::WaitForDecoderResetBurst
            | RecoveryAction::CooldownSuppressed
            | RecoveryAction::StartupGraceSuppressed => {}
        }
    }

    /// 确认clean anchor
    pub fn acknowledge_clean_anchor(&mut self) {
        self.coordinator.on_clean_anchor(true);
        self.escalation_controller.acknowledge_stable_recovery();
    }

    /// 确认稳定恢复
    pub fn acknowledge_stable_recovery(&mut self) {
        self.coordinator.on_clean_anchor(true);
        self.escalation_controller.acknowledge_stable_recovery();
    }

    /// 转换RecoveryDecision到CoordinatorProposal
    fn convert_to_proposal(
        &self,
        decision: RecoveryDecision,
        budget_before: RecoveryActionBudgetState,
        budget_after: RecoveryActionBudgetState,
    ) -> CoordinatorProposal {
        // 生成observation_id（简单递增）
        static OBSERVATION_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let observation_id = OBSERVATION_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        let action = suppress_session_picture_recovery_action(decision.action);
        CoordinatorProposal {
            decision: VideoEscalationDecision {
                observation_id,
                action,
            },
            coalescing_mode: decision.coalescing_mode,
            unlock_reason: decision.unlock_reason,
            preempt_reason: decision.preempt_reason,
            budget_before,
            budget_after,
        }
    }

    fn convert_escalation_decision_to_proposal(
        &self,
        decision: VideoEscalationDecision,
        budget_before: RecoveryActionBudgetState,
        budget_after: RecoveryActionBudgetState,
    ) -> CoordinatorProposal {
        CoordinatorProposal {
            decision: VideoEscalationDecision {
                observation_id: decision.observation_id,
                action: suppress_session_picture_recovery_action(decision.action),
            },
            coalescing_mode: None,
            unlock_reason: None,
            preempt_reason: None,
            budget_before,
            budget_after,
        }
    }

    fn snapshot_budget_state(&self) -> RecoveryActionBudgetState {
        let budget = self.coordinator.state_machine().current_budget();
        RecoveryActionBudgetState {
            recovery_epoch: budget.recovery_epoch,
            keyframe_budget_used: 0,
            keyframe_budget_limit: 255,
            decoder_reset_budget_used: 0,
            decoder_reset_budget_limit: 255,
            reconnect_budget_used: budget.reconnect_used,
            reconnect_budget_limit: budget.reconnect_limit,
        }
    }

    pub(crate) fn current_budget_state(&self) -> RecoveryActionBudgetState {
        self.snapshot_budget_state()
    }

    pub(crate) fn on_reconnect_candidate_accepted(&mut self) {
        self.coordinator.on_reconnect_requested();
    }

    /// 检查是否有硬恢复证据（简化版本）
    ///
    /// 用于判断 keyframe 请求是否基于明确的媒体问题证据
    pub fn transport_await_has_hard_recovery_evidence(
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        _recovery_epoch: u64,
        now_ms: f64,
    ) -> bool {
        RuntimeStatsSink::read_shared(runtime_stats, |stats| {
            Self::transport_await_has_hard_recovery_evidence_from_stats(stats, now_ms)
        })
        .unwrap_or(false)
    }

    pub(crate) fn transport_await_has_hard_recovery_evidence_from_stats(
        stats: &XbxEngineMediaRuntimeStats,
        now_ms: f64,
    ) -> bool {
        use crate::transport::rtc::recovery::contract::{
            has_current_clean_anchor_from_stats, has_current_transport_await_issue_from_stats,
            recovery_progress_level_from_episode, recovery_progress_missing_anchor,
        };

        // 1. 必须有未解决的 transport await 问题
        if !has_current_transport_await_issue_from_stats(stats) {
            return false;
        }

        // 2. 缺少当前 epoch 的 clean anchor
        if has_current_clean_anchor_from_stats(stats) {
            return false;
        }
        if Self::transport_await_decoded_pending_commit_expired(stats, now_ms) {
            return true;
        }
        let progress = stats
            .latest_keyframe_request_episode
            .as_ref()
            .and_then(|episode| {
                recovery_progress_level_from_episode(
                    episode.status.as_str(),
                    episode.response_verdict.as_deref(),
                    episode.first_video_packet_is_keyframe,
                    episode.first_keyframe_packet_at_ms,
                    episode.first_keyframe_decoded_at_ms,
                    false,
                    false,
                )
            });
        if !recovery_progress_missing_anchor(progress) {
            return false;
        }
        // 3. transport-await 对齐的 keyframe episode 已明确失败/过期，属于硬证据
        let terminal_keyframe_failure = stats
            .latest_keyframe_request_episode
            .as_ref()
            .filter(|episode| episode.request_reason.as_deref() == Some("receiverWaitingKeyframe"))
            .is_some_and(|episode| {
                matches!(
                    episode.status.as_str(),
                    "deferred" | "failed" | "missed" | "expired-unsent" | "late"
                ) || matches!(
                    episode.response_verdict.as_deref(),
                    Some(
                        "transportDeferred"
                            | "transportFailed"
                            | "missed"
                            | "unsentExpired"
                            | "late"
                    )
                )
            });
        if terminal_keyframe_failure {
            return true;
        }

        // 4. keyframe 响应已经明确进入 invalid bootstrap，说明 local recovery 已得到负面证据
        stats
            .latest_h264_inspection_observation
            .as_ref()
            .filter(|inspection| (now_ms - inspection.observed_at_ms).max(0.0) <= 1_500.0)
            .is_some_and(|inspection| {
                !inspection.bootstrap_ready
                    && matches!(
                        inspection.bootstrap_reject_reason.as_deref(),
                        Some(
                            "bootstrapMissingSps"
                                | "bootstrapMissingPps"
                                | "bootstrapInvalidSliceHeader"
                        )
                    )
            })
    }

    /// 检查本地恢复（NACK）是否活跃
    ///
    /// 用于判断是否应该等待 NACK 完成而不是立即升级到 IDR
    pub(crate) fn transport_await_local_recovery_active(
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        _recovery_epoch: u64,
        now_ms: f64,
    ) -> bool {
        RuntimeStatsSink::read_shared(runtime_stats, |stats| {
            if Self::transport_await_decoded_pending_commit_expired(stats, now_ms) {
                return false;
            }
            let Some(nack_obs) = stats.latest_video_nack_observation.as_ref() else {
                return false;
            };
            let has_recent_nack =
                nack_obs.action == "sent" && (now_ms - nack_obs.observed_at_ms) < 500.0;

            // 检查是否有最近的包到达（说明 NACK 可能在工作）
            let has_recent_packets = stats
                .latest_video_packet_arrival_time_ms
                .map_or(false, |t| (now_ms - t) < 200.0);

            has_recent_nack && has_recent_packets
        })
        .unwrap_or(false)
    }

    fn transport_await_decoded_pending_commit_expired(
        stats: &XbxEngineMediaRuntimeStats,
        now_ms: f64,
    ) -> bool {
        let has_current_clean_anchor = stats.video_anchor_clean_epoch
            == Some(stats.transport_recovery_epoch)
            && stats.video_anchor_clean_source_event.as_deref()
                == Some("chain-clean-anchor-submitted")
            && stats.video_anchor_clean_observed_at_ms.is_some();
        if has_current_clean_anchor {
            return false;
        }

        let profile = resolve_runtime_recovery_profile(stats);
        let timing = resolve_recovery_dynamic_timing(stats, profile);
        stats
            .recent_keyframe_request_episodes
            .iter()
            .chain(stats.latest_keyframe_request_episode.iter())
            .filter(|episode| {
                episode.request_reason.as_deref() == Some("receiverWaitingKeyframe")
                    && episode.retired_at_ms.is_none()
            })
            .max_by(|left, right| {
                left.first_keyframe_decoded_at_ms
                    .is_some()
                    .cmp(&right.first_keyframe_decoded_at_ms.is_some())
                    .then_with(|| {
                        left.first_keyframe_packet_at_ms
                            .is_some()
                            .cmp(&right.first_keyframe_packet_at_ms.is_some())
                    })
                    .then_with(|| left.sent_at_ms.is_some().cmp(&right.sent_at_ms.is_some()))
                    .then_with(|| left.requested_at_ms.total_cmp(&right.requested_at_ms))
                    .then_with(|| left.episode_id.cmp(&right.episode_id))
            })
            .and_then(|episode| {
                matches!(
                    episode.status.as_str(),
                    "response-observed" | "packet-seen" | "decoded"
                )
                .then_some(episode)
            })
            .and_then(|episode| episode.first_keyframe_decoded_at_ms)
            .is_some_and(|decoded_at_ms| {
                (now_ms - decoded_at_ms).max(0.0) >= timing.clean_anchor_commit_patience_window_ms
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::rtc::recovery::policy::{
        RecoveryScenarioProfile, ScenarioPolicyProfileKind, ScenarioPolicyResolver,
    };
    use xbxengine_protocol::XbxEngineTransportStateDto;

    fn test_profile() -> RecoveryScenarioProfile {
        ScenarioPolicyResolver::resolve_recovery_profile_by_kind(
            ScenarioPolicyProfileKind::HomeLanGaming,
        )
    }

    fn make_coordinator(recovery_epoch: u64) -> RecoveryCoordinator {
        RecoveryCoordinator::new(StateRecoveryCoordinator::new(
            test_profile(),
            recovery_epoch,
        ))
    }

    #[test]
    fn connected_reconnect_request_does_not_clear_inflight_without_edge() {
        let mut coordinator = make_coordinator(7);
        let shared_stats = Mutex::new(XbxEngineMediaRuntimeStats {
            transport_recovery_epoch: 7,
            transport_state: XbxEngineTransportStateDto::Connected,
            ..Default::default()
        });

        let first = coordinator.propose_from_owner_signal(
            RecoveryOwnerSignal {
                reason: VideoEscalationReason::LifecycleRecovering,
                reason_label: "rtcConnectionRecovering".to_string(),
                observed_at_ms: 1000.0,
                gap_severity: None,
                repairability: None,
            },
            &shared_stats,
        );
        assert_eq!(
            first.decision.action,
            RecoveryAction::RequestReconnectCandidate
        );
        coordinator.on_reconnect_candidate_accepted();
        assert!(coordinator
            .coordinator
            .state_machine()
            .is_reconnect_in_flight());

        let second = coordinator.propose_from_owner_signal(
            RecoveryOwnerSignal {
                reason: VideoEscalationReason::LifecycleRecovering,
                reason_label: "rtcConnectionRecovering".to_string(),
                observed_at_ms: 1100.0,
                gap_severity: None,
                repairability: None,
            },
            &shared_stats,
        );
        assert_eq!(second.decision.action, RecoveryAction::CooldownSuppressed);
        assert!(
            coordinator
                .coordinator
                .state_machine()
                .is_reconnect_in_flight(),
            "仍处于 Connected 且没有成功边沿时，不应清除 reconnect in-flight"
        );
    }

    #[test]
    fn coalesced_keyframe_action_is_preserved_in_proposal() {
        let mut coordinator = make_coordinator(3);
        let shared_stats = Mutex::new(XbxEngineMediaRuntimeStats {
            transport_recovery_epoch: 3,
            ..Default::default()
        });

        let first = coordinator.propose_from_owner_signal(
            RecoveryOwnerSignal {
                reason: VideoEscalationReason::WaitKeyframe,
                reason_label: "waitKeyframe".to_string(),
                observed_at_ms: 1000.0,
                gap_severity: None,
                repairability: None,
            },
            &shared_stats,
        );
        assert_eq!(first.decision.action, RecoveryAction::CooldownSuppressed);

        let second = coordinator.propose_from_owner_signal(
            RecoveryOwnerSignal {
                reason: VideoEscalationReason::WaitKeyframe,
                reason_label: "waitKeyframe".to_string(),
                observed_at_ms: 1010.0,
                gap_severity: None,
                repairability: None,
            },
            &shared_stats,
        );
        assert_eq!(
            second.decision.action,
            RecoveryAction::CoalescedKeyframeInFlight
        );
    }

    #[test]
    fn reconnect_budget_is_not_consumed_during_proposal_generation() {
        let mut coordinator = make_coordinator(11);
        let shared_stats = Mutex::new(XbxEngineMediaRuntimeStats {
            transport_recovery_epoch: 11,
            transport_state: XbxEngineTransportStateDto::Disconnected,
            ..Default::default()
        });

        let proposal = coordinator.propose_from_owner_signal(
            RecoveryOwnerSignal {
                reason: VideoEscalationReason::LifecycleRecovering,
                reason_label: "rtcConnectionDisconnected".to_string(),
                observed_at_ms: 1000.0,
                gap_severity: None,
                repairability: None,
            },
            &shared_stats,
        );

        assert_eq!(proposal.budget_before.recovery_epoch, 11);
        assert_eq!(proposal.budget_before.reconnect_budget_used, 0);
        assert_eq!(proposal.budget_after.reconnect_budget_used, 0);
    }

    #[test]
    fn reconnect_budget_advances_only_after_candidate_is_accepted() {
        let mut coordinator = make_coordinator(12);
        let shared_stats = Mutex::new(XbxEngineMediaRuntimeStats {
            transport_recovery_epoch: 12,
            transport_state: XbxEngineTransportStateDto::Disconnected,
            ..Default::default()
        });

        let proposal = coordinator.propose_from_owner_signal(
            RecoveryOwnerSignal {
                reason: VideoEscalationReason::LifecycleRecovering,
                reason_label: "rtcConnectionDisconnected".to_string(),
                observed_at_ms: 1000.0,
                gap_severity: None,
                repairability: None,
            },
            &shared_stats,
        );

        assert_eq!(
            proposal.decision.action,
            RecoveryAction::RequestReconnectCandidate
        );
        assert_eq!(coordinator.current_budget_state().reconnect_budget_used, 0);

        coordinator.on_reconnect_candidate_accepted();

        assert_eq!(coordinator.current_budget_state().reconnect_budget_used, 1);
    }

    #[test]
    fn owner_signal_local_evidence_reaches_observation_layer() {
        let mut coordinator = make_coordinator(5);
        let shared_stats = Mutex::new(XbxEngineMediaRuntimeStats {
            transport_recovery_epoch: 5,
            ..Default::default()
        });

        let proposal = coordinator.propose_from_owner_signal(
            RecoveryOwnerSignal {
                reason: VideoEscalationReason::TransportExpiredDeadline,
                reason_label: "transportExpiredDeadline".to_string(),
                observed_at_ms: 1000.0,
                gap_severity: Some(GapSeverity::ReferenceGap),
                repairability: Some(0.2),
            },
            &shared_stats,
        );

        assert_eq!(proposal.decision.action, RecoveryAction::CooldownSuppressed);
    }

    #[test]
    fn reconnect_completion_requires_post_episode_decode_or_present_progress() {
        let stats = XbxEngineMediaRuntimeStats {
            transport_recovery_epoch: 13,
            transport_state: XbxEngineTransportStateDto::Connected,
            transport_recovery_episode_opened_at_ms: Some(1_000.0),
            latest_video_packet_arrival_time_ms: Some(1_120.0),
            ..Default::default()
        };

        assert!(
            !RecoveryCoordinator::check_reconnect_completed(&stats, false),
            "仅有 connected 边沿和新包到达不应视为 reconnect 已完成"
        );

        let decode_progress = XbxEngineMediaRuntimeStats {
            latest_video_decode_ok_time_ms: Some(1_180.0),
            ..stats.clone()
        };
        assert!(RecoveryCoordinator::check_reconnect_completed(
            &decode_progress,
            false,
        ));

        let present_progress = XbxEngineMediaRuntimeStats {
            latest_video_host_present_time_ms: Some(1_190.0),
            ..stats
        };
        assert!(RecoveryCoordinator::check_reconnect_completed(
            &present_progress,
            false,
        ));
    }

    #[test]
    fn connectivity_reason_with_high_repairability_stays_in_local_repair() {
        let mut coordinator = make_coordinator(17);
        let shared_stats = Mutex::new(XbxEngineMediaRuntimeStats {
            transport_recovery_epoch: 17,
            ..Default::default()
        });

        let proposal = coordinator.propose_from_owner_signal(
            RecoveryOwnerSignal {
                reason: VideoEscalationReason::TransportExpiredDeadline,
                reason_label: "transportExpiredDeadline".to_string(),
                observed_at_ms: 1000.0,
                gap_severity: Some(GapSeverity::ReferenceGap),
                repairability: Some(0.9),
            },
            &shared_stats,
        );

        assert_eq!(
            proposal.decision.action,
            RecoveryAction::WaitForBurst,
            "高 repairability 的 connectivity 事件应先停留在 local repair，而不是直接请求 keyframe"
        );
    }

    #[test]
    fn skipped_nack_does_not_count_as_active_local_recovery() {
        let now_ms = 2_000.0;
        let stats = Mutex::new(XbxEngineMediaRuntimeStats {
            latest_video_nack_observation: Some(crate::XbxEngineVideoNackObservation {
                observation_id: 1,
                action: "skipped".to_string(),
                source: "sampleLoss".to_string(),
                first_sequence: 10,
                last_sequence: 11,
                packet_count: 2,
                retry_count: 0,
                frame_rtp_timestamp: None,
                frame_is_keyframe: Some(false),
                frame_importance: Some("unknown".to_string()),
                deadline_at_ms: None,
                estimated_recovery_arrival_ms: None,
                nack_disposition: Some("attempted".to_string()),
                frame_playout_deadline_at_ms: None,
                frame_unrecoverable_reason: None,
                frame_budget: None,
                observed_at_ms: now_ms - 20.0,
            }),
            latest_video_packet_arrival_time_ms: Some(now_ms - 10.0),
            ..Default::default()
        });

        assert!(
            !RecoveryCoordinator::transport_await_local_recovery_active(&stats, 0, now_ms),
            "skipped NACK 不是正在进行的本地修复，不应继续压住 reconnect"
        );
    }

    #[test]
    fn decoded_pending_commit_timeout_releases_local_recovery_active() {
        let now_ms = 2_000.0;
        let stats = Mutex::new(XbxEngineMediaRuntimeStats {
            transport_recovery_epoch: 31,
            latest_video_nack_observation: Some(crate::XbxEngineVideoNackObservation {
                observation_id: 1,
                action: "sent".to_string(),
                source: "sampleLoss".to_string(),
                first_sequence: 10,
                last_sequence: 11,
                packet_count: 2,
                retry_count: 0,
                frame_rtp_timestamp: None,
                frame_is_keyframe: Some(false),
                frame_importance: Some("unknown".to_string()),
                deadline_at_ms: None,
                estimated_recovery_arrival_ms: None,
                nack_disposition: Some("attempted".to_string()),
                frame_playout_deadline_at_ms: None,
                frame_unrecoverable_reason: None,
                frame_budget: None,
                observed_at_ms: now_ms - 20.0,
            }),
            latest_video_packet_arrival_time_ms: Some(now_ms - 10.0),
            latest_keyframe_request_episode: Some(
                crate::XbxEngineKeyframeRequestEpisodeObservation {
                    episode_id: 31,
                    request_reason: Some("receiverWaitingKeyframe".to_string()),
                    request_kind: Some("pli".to_string()),
                    status: "decoded".to_string(),
                    status_detail: Some("receiverLocalContinuation".to_string()),
                    requested_at_ms: now_ms - 600.0,
                    sent_at_ms: Some(now_ms - 580.0),
                    deadline_at_ms: Some(now_ms + 100.0),
                    transport_detail: None,
                    first_video_packet_at_ms: Some(now_ms - 540.0),
                    first_video_packet_rtp_timestamp: Some(0x2233_4401),
                    first_video_packet_is_keyframe: Some(false),
                    first_keyframe_packet_at_ms: None,
                    first_keyframe_decoded_at_ms: Some(now_ms - 500.0),
                    response_rtp_timestamp: Some(0x2233_4401),
                    response_frame_seq: Some(41),
                    response_verdict: Some("pending".to_string()),
                    lifecycle_phase: Some("decoded".to_string()),
                    retired_at_ms: None,
                },
            ),
            ..Default::default()
        });

        assert!(
            !RecoveryCoordinator::transport_await_local_recovery_active(&stats, 31, now_ms),
            "decoded 后已跨过 clean-anchor 提交等待窗口时，应释放本地恢复活跃态"
        );
    }

    #[test]
    fn decoded_pending_commit_timeout_counts_as_hard_recovery_evidence() {
        let now_ms = 2_000.0;
        let stats = XbxEngineMediaRuntimeStats {
            transport_recovery_epoch: 31,
            latest_video_timeline_observation: Some(crate::XbxEngineVideoTimelineObservation {
                observation_id: 9,
                source_event: "frame-await-recovery-anchor".to_string(),
                gap: None,
                frame: None,
                chain: crate::XbxEngineVideoTimelineChainSnapshot {
                    state: "recovering".to_string(),
                    reason: Some("receiverWaitingKeyframe".to_string()),
                    chain_break_evidence: None,
                    observed_at_ms: now_ms - 8.0,
                },
                observed_at_ms: now_ms - 8.0,
            }),
            latest_keyframe_request_episode: Some(
                crate::XbxEngineKeyframeRequestEpisodeObservation {
                    episode_id: 31,
                    request_reason: Some("receiverWaitingKeyframe".to_string()),
                    request_kind: Some("pli".to_string()),
                    status: "decoded".to_string(),
                    status_detail: Some("receiverLocalContinuation".to_string()),
                    requested_at_ms: now_ms - 600.0,
                    sent_at_ms: Some(now_ms - 580.0),
                    deadline_at_ms: Some(now_ms + 100.0),
                    transport_detail: None,
                    first_video_packet_at_ms: Some(now_ms - 540.0),
                    first_video_packet_rtp_timestamp: Some(0x2233_4401),
                    first_video_packet_is_keyframe: Some(false),
                    first_keyframe_packet_at_ms: None,
                    first_keyframe_decoded_at_ms: Some(now_ms - 500.0),
                    response_rtp_timestamp: Some(0x2233_4401),
                    response_frame_seq: Some(41),
                    response_verdict: Some("pending".to_string()),
                    lifecycle_phase: Some("decoded".to_string()),
                    retired_at_ms: None,
                },
            ),
            ..Default::default()
        };

        assert!(
            RecoveryCoordinator::transport_await_has_hard_recovery_evidence_from_stats(
                &stats, now_ms
            ),
            "decoded 后迟迟没有 clean anchor 提交，应作为 transport-await 的硬失败证据"
        );
    }

    #[test]
    fn non_idr_during_rfi_grace_window_does_not_count_as_hard_recovery_evidence() {
        let now_ms = 2_000.0;
        let stats = XbxEngineMediaRuntimeStats {
            transport_recovery_epoch: 33,
            transport_state: XbxEngineTransportStateDto::Connected,
            latest_video_timeline_observation: Some(crate::XbxEngineVideoTimelineObservation {
                observation_id: 907,
                source_event: "frame-await-recovery-anchor".to_string(),
                gap: None,
                frame: None,
                chain: crate::XbxEngineVideoTimelineChainSnapshot {
                    state: "recovering".to_string(),
                    reason: Some("awaitingRecoveryAnchor".to_string()),
                    chain_break_evidence: None,
                    observed_at_ms: now_ms - 8.0,
                },
                observed_at_ms: now_ms - 8.0,
            }),
            latest_h264_inspection_observation: Some(crate::XbxEngineH264InspectionObservation {
                observation_id: 908,
                frame_rtp_timestamp: Some(3_333),
                nal_types: vec!["SliceLayerWithoutPartitioningNonIdr".to_string()],
                nal_count: 1,
                vcl_nal_count: 1,
                has_inband_sps: false,
                has_inband_pps: false,
                committed_sps_present: true,
                committed_pps_present: true,
                slice_headers_valid: true,
                delta_continuation_ready: true,
                parameter_sets_changed: false,
                config_changed: false,
                is_idr: false,
                sample_width: Some(1920),
                sample_height: Some(1080),
                bootstrap_ready: false,
                bootstrap_reject_reason: Some("NonIdrVcl".to_string()),
                admission_accepted: true,
                observed_at_ms: now_ms - 6.0,
                ..Default::default()
            }),
            latest_keyframe_request_episode: Some(
                crate::XbxEngineKeyframeRequestEpisodeObservation {
                    episode_id: 909,
                    request_reason: Some("receiverWaitingKeyframe".to_string()),
                    request_kind: Some("pli".to_string()),
                    status: "deferred".to_string(),
                    status_detail: Some("sameFamilyCoalesced:transportStageSuppressed".to_string()),
                    requested_at_ms: now_ms - 25.0,
                    sent_at_ms: None,
                    deadline_at_ms: Some(now_ms + 600.0),
                    transport_detail: Some(
                        "sameFamilyCoalesced:transportStageSuppressed".to_string(),
                    ),
                    first_video_packet_at_ms: Some(now_ms - 6.0),
                    first_video_packet_rtp_timestamp: Some(3_333),
                    first_video_packet_is_keyframe: Some(false),
                    first_keyframe_packet_at_ms: None,
                    first_keyframe_decoded_at_ms: None,
                    response_rtp_timestamp: None,
                    response_frame_seq: None,
                    response_verdict: Some("transportDeferred".to_string()),
                    lifecycle_phase: None,
                    retired_at_ms: None,
                },
            ),
            ..Default::default()
        };

        assert!(
            RecoveryCoordinator::transport_await_has_hard_recovery_evidence_from_stats(
                &stats, now_ms
            )
        );
    }

    #[test]
    fn stale_decoded_episode_does_not_clear_current_idr_inflight() {
        let stats = XbxEngineMediaRuntimeStats {
            transport_recovery_epoch: 7,
            latest_keyframe_request_episode: Some(
                crate::XbxEngineKeyframeRequestEpisodeObservation {
                    episode_id: 70,
                    request_reason: Some("receiverWaitingKeyframe".to_string()),
                    request_kind: Some("pli".to_string()),
                    status: "decoded".to_string(),
                    status_detail: None,
                    requested_at_ms: 100.0,
                    sent_at_ms: Some(110.0),
                    deadline_at_ms: Some(500.0),
                    transport_detail: None,
                    first_video_packet_at_ms: Some(130.0),
                    first_video_packet_rtp_timestamp: Some(1_000),
                    first_video_packet_is_keyframe: Some(true),
                    first_keyframe_packet_at_ms: Some(130.0),
                    first_keyframe_decoded_at_ms: Some(150.0),
                    response_rtp_timestamp: Some(1_000),
                    response_frame_seq: Some(10),
                    response_verdict: Some("on-time".to_string()),
                    lifecycle_phase: Some("decoded".to_string()),
                    retired_at_ms: None,
                },
            ),
            ..Default::default()
        };

        assert!(
            !RecoveryCoordinator::check_idr_completed(&stats, Some(160.0)),
            "旧 decoded 状态不应在没有新响应进展时清掉当前 PLI in-flight"
        );
    }

    #[test]
    fn current_transport_await_keyframe_packet_counts_as_response_observed_only() {
        let stats = XbxEngineMediaRuntimeStats {
            transport_recovery_epoch: 7,
            latest_keyframe_request_episode: Some(
                crate::XbxEngineKeyframeRequestEpisodeObservation {
                    episode_id: 71,
                    request_reason: Some("receiverWaitingKeyframe".to_string()),
                    request_kind: Some("pli".to_string()),
                    status: "response-observed".to_string(),
                    status_detail: Some("pending".to_string()),
                    requested_at_ms: 100.0,
                    sent_at_ms: Some(110.0),
                    deadline_at_ms: Some(500.0),
                    transport_detail: None,
                    first_video_packet_at_ms: Some(170.0),
                    first_video_packet_rtp_timestamp: Some(2_000),
                    first_video_packet_is_keyframe: Some(false),
                    first_keyframe_packet_at_ms: Some(180.0),
                    first_keyframe_decoded_at_ms: None,
                    response_rtp_timestamp: Some(2_000),
                    response_frame_seq: None,
                    response_verdict: Some("pending".to_string()),
                    lifecycle_phase: Some("response-observed".to_string()),
                    retired_at_ms: None,
                },
            ),
            ..Default::default()
        };

        assert_eq!(
            RecoveryCoordinator::latest_transport_await_response_observed_at_ms(&stats),
            Some(180.0),
            "当前 transport-await episode 的新 keyframe 响应应进入 response-observed 层"
        );
        assert!(
            !RecoveryCoordinator::check_idr_completed(&stats, Some(150.0)),
            "packet-seen 只表示远端已响应，decode 完成需要单独证据"
        );
    }

    #[test]
    fn decoded_late_transport_await_episode_does_not_reopen_session_pli() {
        let now_ms = 2_000.0;
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.session_phase = Some("recovering".to_string());
        stats.transport_recovery_epoch = 31;
        stats.transport_state = XbxEngineTransportStateDto::Connected;
        stats.host_no_pending_pressure_level = Some("critical".to_string());
        stats.host_no_pending_streak = 220;
        stats.latest_video_host_present_time_ms = Some(now_ms - 2_100.0);
        stats.latest_video_decode_ok_time_ms = Some(now_ms - 1_700.0);
        stats.latest_video_packet_arrival_time_ms = Some(now_ms - 20.0);
        stats.video_decoder_stalled = Some(false);
        stats.video_renderer_stalled = Some(true);
        let shared_stats = Mutex::new(stats);
        let mut coordinator = make_coordinator(31);

        let first = coordinator.propose_from_owner_signal(
            RecoveryOwnerSignal {
                reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
                reason_label: "receiverWaitingKeyframe".to_string(),
                observed_at_ms: now_ms,
                gap_severity: None,
                repairability: None,
            },
            &shared_stats,
        );
        assert_eq!(first.decision.action, RecoveryAction::CooldownSuppressed);

        RuntimeStatsSink::update_shared(&shared_stats, |stats| {
            stats.latest_video_packet_arrival_time_ms = Some(now_ms + 420.0);
            stats.latest_video_host_present_time_ms = Some(now_ms + 160.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms + 150.0);
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1280),
                video_height: Some(720),
                mime_type: Some("video/H264".to_string()),
                transport_state: XbxEngineTransportStateDto::Connected,
                video_bytes_total: 10_200_000,
                video_packet_count_total: 9_200,
                audio_bytes_total: 124_000,
                observed_at_ms: now_ms + 420.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 13_001,
                    source_event: "gap-expired-skipped".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("receiverWaitingKeyframe".to_string()),
                        chain_break_evidence: None,
                        observed_at_ms: now_ms + 420.0,
                    },
                    observed_at_ms: now_ms + 420.0,
                });
            stats.latest_h264_inspection_observation =
                Some(crate::XbxEngineH264InspectionObservation {
                    observation_id: 13_002,
                    frame_rtp_timestamp: Some(0xD0D0_1002),
                    nal_types: vec!["SliceLayerWithoutPartitioningNonIdr".to_string()],
                    nal_count: 1,
                    vcl_nal_count: 1,
                    has_inband_sps: false,
                    has_inband_pps: false,
                    committed_sps_present: true,
                    committed_pps_present: true,
                    slice_headers_valid: true,
                    delta_continuation_ready: true,
                    parameter_sets_changed: false,
                    config_changed: false,
                    is_idr: false,
                    sample_width: Some(1280),
                    sample_height: Some(720),
                    bootstrap_ready: false,
                    bootstrap_reject_reason: Some("bootstrapMissingIdr".to_string()),
                    continuation_verdict: Some("receiverLocalContinuation".to_string()),
                    admission_accepted: true,
                    bound_episode_id: Some(first.decision.observation_id),
                    bound_episode_status: Some("decoded".to_string()),
                    bound_recovery_epoch: Some(31),
                    bound_response_rtp_timestamp: Some(0xD0D0_1001),
                    bound_as_recovery_response: Some(true),
                    observed_at_ms: now_ms + 420.0,
                    ..Default::default()
                });
            stats.latest_keyframe_request_episode =
                Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                    episode_id: first.decision.observation_id,
                    request_reason: Some("receiverWaitingKeyframe".to_string()),
                    request_kind: Some("pli".to_string()),
                    status: "decoded".to_string(),
                    status_detail: Some("receiverLocalContinuation".to_string()),
                    requested_at_ms: now_ms,
                    sent_at_ms: Some(now_ms + 10.0),
                    deadline_at_ms: Some(now_ms + 960.0),
                    transport_detail: None,
                    first_video_packet_at_ms: Some(now_ms + 120.0),
                    first_video_packet_rtp_timestamp: Some(0xD0D0_1001),
                    first_video_packet_is_keyframe: Some(false),
                    first_keyframe_packet_at_ms: Some(now_ms + 120.0),
                    first_keyframe_decoded_at_ms: Some(now_ms + 140.0),
                    response_rtp_timestamp: Some(0xD0D0_1001),
                    response_frame_seq: Some(176),
                    response_verdict: Some("late".to_string()),
                    lifecycle_phase: Some("decoded".to_string()),
                    retired_at_ms: None,
                });
        });

        std::thread::sleep(std::time::Duration::from_millis(260));

        let second = coordinator.propose_from_owner_signal(
            RecoveryOwnerSignal {
                reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
                reason_label: "receiverWaitingKeyframe".to_string(),
                observed_at_ms: now_ms + 420.0,
                gap_severity: None,
                repairability: None,
            },
            &shared_stats,
        );
        assert_eq!(second.decision.action, RecoveryAction::CooldownSuppressed);
    }
}
