use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::recovery::decoder_backend_failure::{
    resolve_decoder_backend_failure_recovery, DecoderBackendFailureResolution,
};
use crate::transport::rtc::recovery::escalation::{
    RecoveryAction, RecoveryActionBudgetState, VideoEscalationController, VideoEscalationDecision,
    VideoEscalationReason,
};
use crate::transport::rtc::recovery::hard_stall::resolve_persistent_stall_recovery;
use crate::transport::rtc::recovery::nack_outcome::{
    resolve_recent_nack_outcome, CloudStartupExpiredDeadlineBudget, RecentNackOutcomeResolution,
};
use crate::transport::rtc::recovery::policy::RecoveryScenarioProfile;
use crate::transport::rtc::recovery::repeat_suppression::resolve_recent_repeat_suppression;
use crate::transport::rtc::recovery::runtime_state::{
    has_fresh_media_output, resolve_recovery_profile, unix_now_ms,
};
#[cfg(test)]
use crate::transport::rtc::recovery::runtime_state::{
    runtime_state_for_diagnosis as build_runtime_state_for_diagnosis, RecoveryRuntimeState,
};
use crate::transport::rtc::recovery::startup::{
    resolve_session_phase, should_fast_reset_startup_recovery, should_suppress_startup_escalation,
    SessionPhase, StartupRecoveryProbe,
};
use crate::{
    XbxEngineAnchorCandidateLedger, XbxEngineAnchorCandidateState, XbxEngineMediaRuntimeStats,
    XbxEngineVideoTimelineObservation,
};

#[derive(Clone, Debug)]
pub struct RecoveryOwnerSignal {
    pub reason: VideoEscalationReason,
    pub reason_label: String,
    pub observed_at_ms: f64,
}

#[derive(Clone, Debug)]
pub struct RecoveryCoordinatorProposal {
    pub signal: RecoveryOwnerSignal,
    pub decision: VideoEscalationDecision,
    pub budget_before: RecoveryActionBudgetState,
    pub budget_after: RecoveryActionBudgetState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RecoverySignalDomain {
    Connectivity,
    MediaRecovery,
}

/**
 * 统一承接 startup/recovery 的局部状态：
 * - `stack` 只负责喂事件和执行动作
 * - startup grace / fast-reset / low-quality probe 不再散落在事件循环里
 * - owner signal -> coordinator -> planner/command 的恢复链路只经过这里
 */
pub struct RecoveryCoordinator {
    escalation_controller: VideoEscalationController,
    startup_probe: StartupRecoveryProbe,
    stream_started_at: Instant,
    startup_grace: Duration,
    cloud_startup_nack_budget: CloudStartupExpiredDeadlineBudget,
    await_recovery_keyframe_streak: u16,
    await_recovery_keyframe_last_seen_at_ms: Option<f64>,
    await_recovery_keyframe_streak_started_at_ms: Option<f64>,
    await_recovery_hard_fallback_started_at_ms: Option<f64>,
}

const TRANSPORT_AWAIT_RECOVERY_KEYFRAME_STREAK_WINDOW_MS: f64 = 3_500.0;
const TRANSPORT_AWAIT_CONNECTED_INGRESS_EVIDENCE_MAX_AGE_MS: f64 = 4_000.0;

impl RecoveryCoordinator {
    pub fn new(
        escalation_controller: VideoEscalationController,
        stream_started_at: Instant,
        startup_grace: Duration,
    ) -> Self {
        Self {
            escalation_controller,
            startup_probe: StartupRecoveryProbe::default(),
            stream_started_at,
            startup_grace,
            cloud_startup_nack_budget: CloudStartupExpiredDeadlineBudget::default(),
            await_recovery_keyframe_streak: 0,
            await_recovery_keyframe_last_seen_at_ms: None,
            await_recovery_keyframe_streak_started_at_ms: None,
            await_recovery_hard_fallback_started_at_ms: None,
        }
    }

    pub fn on_reason_with_runtime_stats(
        &mut self,
        reason: VideoEscalationReason,
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
    ) -> VideoEscalationDecision {
        self.propose_from_owner_signal(
            RecoveryOwnerSignal {
                reason,
                reason_label: reason.label().to_string(),
                observed_at_ms: unix_now_ms(),
            },
            runtime_stats,
        )
        .decision
    }

    pub fn propose_from_owner_signal(
        &mut self,
        signal: RecoveryOwnerSignal,
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
    ) -> RecoveryCoordinatorProposal {
        let recovery_epoch =
            RuntimeStatsSink::read_shared(runtime_stats, |stats| stats.transport_recovery_epoch)
                .unwrap_or(0);
        self.escalation_controller
            .begin_recovery_epoch(recovery_epoch);
        let budget_before = self.escalation_controller.budget_state();
        self.track_await_recovery_keyframe_streak(signal.reason, signal.observed_at_ms);
        if let Some(decision) =
            self.resolve_decoder_backend_failure_recovery(runtime_stats, &signal.reason)
        {
            return RecoveryCoordinatorProposal {
                signal,
                decision,
                budget_before,
                budget_after: self.escalation_controller.budget_state(),
            };
        }
        if let Some(decision) =
            self.resolve_persistent_stall_recovery(runtime_stats, &signal.reason)
        {
            return RecoveryCoordinatorProposal {
                signal,
                decision,
                budget_before,
                budget_after: self.escalation_controller.budget_state(),
            };
        }
        if let Some(decision) =
            self.resolve_recent_repeat_suppression(runtime_stats, &signal.reason)
        {
            return RecoveryCoordinatorProposal {
                signal,
                decision,
                budget_before,
                budget_after: self.escalation_controller.budget_state(),
            };
        }
        if let Some(decision) = self.resolve_recent_nack_outcome(runtime_stats, &signal.reason) {
            return RecoveryCoordinatorProposal {
                signal,
                decision,
                budget_before,
                budget_after: self.escalation_controller.budget_state(),
            };
        }
        if matches!(signal.reason, VideoEscalationReason::AdapterIdleTimeout)
            && RuntimeStatsSink::read_shared(runtime_stats, |stats| {
                has_fresh_media_output(stats, unix_now_ms())
                    && !stats.video_decoder_stalled.unwrap_or(false)
                    && !stats.video_renderer_stalled.unwrap_or(false)
            })
            .unwrap_or(false)
        {
            return RecoveryCoordinatorProposal {
                signal,
                decision: self
                    .escalation_controller
                    .suppressed(RecoveryAction::CooldownSuppressed),
                budget_before,
                budget_after: self.escalation_controller.budget_state(),
            };
        }
        let phase =
            resolve_session_phase(runtime_stats, self.stream_started_at, self.startup_grace);
        let profile = resolve_recovery_profile(runtime_stats);
        let decision = self.on_reason_with_policy(
            signal.reason,
            phase,
            profile,
            recovery_epoch,
            runtime_stats,
            signal.observed_at_ms,
        );
        RecoveryCoordinatorProposal {
            signal,
            decision,
            budget_before,
            budget_after: self.escalation_controller.budget_state(),
        }
    }

    pub fn propose_lifecycle_reconnect(
        &mut self,
        reason_label: String,
        observed_at_ms: f64,
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
    ) -> RecoveryCoordinatorProposal {
        // 兼容入口：lifecycle recovering 也统一走 owner signal 主链，避免形成双轨恢复路径。
        self.propose_from_owner_signal(
            RecoveryOwnerSignal {
                reason: VideoEscalationReason::LifecycleRecovering,
                reason_label,
                observed_at_ms,
            },
            runtime_stats,
        )
    }

    pub fn acknowledge_clean_anchor(&mut self) {
        self.await_recovery_keyframe_streak = 0;
        self.await_recovery_keyframe_last_seen_at_ms = None;
        self.await_recovery_keyframe_streak_started_at_ms = None;
        // 手动确认 clean anchor 只清理 keyframe 侧的阶段性噪声。
        // hard-fallback 计时的内部起点要保留，避免短暂健康帧把既有坏窗彻底打散。
        self.escalation_controller.reset_keyframe_epoch();
    }

    fn on_reason_with_policy(
        &mut self,
        reason: VideoEscalationReason,
        phase: SessionPhase,
        profile: RecoveryScenarioProfile,
        recovery_epoch: u64,
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        observed_at_ms: f64,
    ) -> VideoEscalationDecision {
        let signal_domain = classify_signal_domain(reason);
        let startup_fast_reset = profile.startup_fast_reset_enabled
            && phase == SessionPhase::Startup
            && should_fast_reset_startup_recovery(
                &reason,
                self.stream_started_at,
                self.startup_grace,
            );
        let mut escalation_decision = if phase == SessionPhase::Startup
            && should_suppress_startup_escalation(
                &reason,
                self.stream_started_at,
                self.startup_grace,
            ) {
            self.escalation_controller
                .suppressed(RecoveryAction::StartupGraceSuppressed)
        } else {
            self.escalation_controller.on_reason_with_epoch_policy(
                reason,
                recovery_epoch,
                signal_domain == RecoverySignalDomain::Connectivity,
            )
        };
        if reason == VideoEscalationReason::TransportAwaitRecoveryKeyframe
            && Self::transport_await_soft_reentry_is_recent_and_healthy(
                runtime_stats,
                recovery_epoch,
                observed_at_ms,
            )
            && escalation_decision.action != RecoveryAction::CooldownSuppressed
        {
            escalation_decision = self
                .escalation_controller
                .suppressed(RecoveryAction::CooldownSuppressed);
        }
        if let Some(forced_stage_decision) = self.maybe_force_transport_await_stage_upgrade(
            runtime_stats,
            reason,
            profile,
            recovery_epoch,
            observed_at_ms,
            escalation_decision.action,
        ) {
            // owner 连续上报 awaitingRecoveryKeyframe，且当前恢复动作已被压成 cooldown 时，
            // 在明确 stall 证据出现后更早推进 staged recovery，缩短 Connected 后坏窗。
            escalation_decision = forced_stage_decision;
        }
        if let Some(hard_fallback_decision) = self.resolve_transport_await_hard_fallback(
            reason,
            recovery_epoch,
            profile,
            runtime_stats,
            observed_at_ms,
        ) {
            escalation_decision = hard_fallback_decision;
        }
        let action = if startup_fast_reset
            && escalation_decision.action == RecoveryAction::RequestKeyframe
        {
            RecoveryAction::RequestKeyframeAndDecoderReset
        } else {
            escalation_decision.action
        };
        if startup_fast_reset && action == RecoveryAction::RequestKeyframeAndDecoderReset {
            self.startup_probe.arm(Instant::now());
        }
        VideoEscalationDecision {
            observation_id: escalation_decision.observation_id,
            action,
        }
    }

    fn resolve_transport_await_hard_fallback(
        &mut self,
        reason: VideoEscalationReason,
        recovery_epoch: u64,
        profile: RecoveryScenarioProfile,
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        observed_at_ms: f64,
    ) -> Option<VideoEscalationDecision> {
        if reason != VideoEscalationReason::TransportAwaitRecoveryKeyframe {
            return None;
        }
        let explicit_healthy_with_clean_anchor =
            Self::transport_await_soft_reentry_is_recent_and_healthy(
                runtime_stats,
                recovery_epoch,
                observed_at_ms,
            );
        if explicit_healthy_with_clean_anchor {
            self.reset_transport_await_hard_fallback(runtime_stats, "explicitHealthyCleanAnchor");
            return None;
        }
        let has_evidence = Self::has_transport_await_hard_fallback_evidence(
            runtime_stats,
            observed_at_ms,
            profile,
        );
        if self.await_recovery_hard_fallback_started_at_ms.is_none() && !has_evidence {
            RuntimeStatsSink::update_shared(runtime_stats, |stats| {
                stats.recovery_hard_fallback_timer_ms = None;
                stats.recovery_hard_fallback_trigger_reason = None;
            });
            return None;
        }
        let started_at_ms = *self
            .await_recovery_hard_fallback_started_at_ms
            .get_or_insert(observed_at_ms);
        let timer_ms = (observed_at_ms - started_at_ms).max(0.0);
        RuntimeStatsSink::update_shared(runtime_stats, |stats| {
            stats.recovery_hard_fallback_timer_ms = Some(timer_ms);
            stats.recovery_hard_fallback_timer_reset_reason = None;
        });
        if timer_ms < profile.hard_fallback_transport_await_timeout_ms as f64 {
            return None;
        }
        if !Self::has_transport_await_decoder_reset_attempt_since(runtime_stats, started_at_ms) {
            // staged recovery: hard fallback 超时后，若还能确认 Connected + ingress 仍在推进，
            // 就允许直接升到 reconnect；否则继续要求先走 decoder reset 尝试。
            if !Self::has_transport_await_connected_ingress_evidence(runtime_stats, observed_at_ms)
            {
                return None;
            }
        }
        let decision = self.escalation_controller.on_reason_with_epoch_policy(
            VideoEscalationReason::LifecycleRecovering,
            recovery_epoch,
            true,
        );
        RuntimeStatsSink::update_shared(runtime_stats, |stats| {
            stats.recovery_hard_fallback_trigger_reason =
                Some("transportAwaitRecoveryKeyframeTimeout".to_string());
        });
        Some(decision)
    }

    fn maybe_force_transport_await_stage_upgrade(
        &mut self,
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        reason: VideoEscalationReason,
        profile: RecoveryScenarioProfile,
        recovery_epoch: u64,
        observed_at_ms: f64,
        current_action: RecoveryAction,
    ) -> Option<VideoEscalationDecision> {
        const TRANSPORT_AWAIT_CONNECTED_BAD_WINDOW_STAGE_MIN_MS: f64 = 120.0;
        if reason != VideoEscalationReason::TransportAwaitRecoveryKeyframe
            || current_action != RecoveryAction::CooldownSuppressed
        {
            return None;
        }
        if Self::transport_await_soft_reentry_is_recent_and_healthy(
            runtime_stats,
            recovery_epoch,
            observed_at_ms,
        ) {
            return None;
        }
        let has_stall_evidence = Self::has_transport_await_hard_fallback_evidence(
            runtime_stats,
            observed_at_ms,
            profile,
        );
        if has_stall_evidence
            && self.await_recovery_keyframe_streak >= 2
            && self
                .await_recovery_keyframe_streak_started_at_ms
                .is_some_and(|started_at_ms| {
                    (observed_at_ms - started_at_ms).max(0.0)
                        >= TRANSPORT_AWAIT_CONNECTED_BAD_WINDOW_STAGE_MIN_MS
                })
        {
            self.escalation_controller
                .register_action_applied(RecoveryAction::RequestDecoderReset);
            return Some(
                self.escalation_controller
                    .suppressed(RecoveryAction::RequestDecoderReset),
            );
        }
        let streak_threshold = if has_stall_evidence { 2 } else { 3 };
        if self.await_recovery_keyframe_streak >= streak_threshold {
            Some(
                self.escalation_controller
                    .on_reason_with_epoch(reason, recovery_epoch),
            )
        } else {
            None
        }
    }

    fn has_transport_await_hard_fallback_evidence(
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        now_ms: f64,
        profile: RecoveryScenarioProfile,
    ) -> bool {
        RuntimeStatsSink::read_shared(runtime_stats, |stats| {
            let no_fresh_output = !has_fresh_media_output(stats, now_ms);
            let present_age_ms = stats
                .latest_video_present_time_ms
                .map(|at_ms| (now_ms - at_ms).max(0.0));
            let present_expired = present_age_ms.is_some_and(|age_ms| {
                age_ms >= profile.display_supply_thresholds.critical_present_age_ms
            });
            let no_pending_critical = matches!(
                stats.host_no_pending_pressure_level.as_deref(),
                Some("high" | "critical")
            ) && stats.host_no_pending_streak
                >= profile.display_supply_thresholds.critical_no_pending_streak;
            stats.video_renderer_stalled.unwrap_or(false)
                || stats.video_decoder_stalled.unwrap_or(false)
                || no_fresh_output
                || present_expired
                || no_pending_critical
        })
        .unwrap_or(false)
    }

    fn reset_transport_await_hard_fallback(
        &mut self,
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        reset_reason: &str,
    ) {
        RuntimeStatsSink::update_shared(runtime_stats, |stats| {
            stats.recovery_hard_fallback_timer_ms = None;
            stats.recovery_hard_fallback_trigger_reason = None;
            stats.recovery_hard_fallback_timer_reset_reason = Some(reset_reason.to_string());
        });
    }

    fn has_transport_await_decoder_reset_attempt_since(
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        started_at_ms: f64,
    ) -> bool {
        RuntimeStatsSink::read_shared(runtime_stats, |stats| {
            let decoder_reset_applied = stats
                .latest_video_decoder_reset_time_ms
                .is_some_and(|at_ms| at_ms >= started_at_ms);
            let decoder_reset_requested = stats
                .latest_video_escalation_observation
                .as_ref()
                .is_some_and(|observation| {
                    observation.action == "requestDecoderReset"
                        && observation.observed_at_ms >= started_at_ms
                });
            decoder_reset_applied || decoder_reset_requested
        })
        .unwrap_or(false)
    }

    fn transport_await_decoder_reset_budget_exhausted(&self) -> bool {
        let budget = self.escalation_controller.budget_state();
        budget.decoder_reset_budget_used >= budget.decoder_reset_budget_limit
    }

    fn has_transport_await_connected_ingress_evidence(
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        now_ms: f64,
    ) -> bool {
        RuntimeStatsSink::read_shared(runtime_stats, |stats| {
            if stats.transport_state != xbxengine_protocol::XbxEngineTransportStateDto::Connected {
                return false;
            }
            let track_attached_and_recent =
                stats
                    .latest_video_track_status
                    .as_ref()
                    .is_some_and(|track| {
                        track.state == "remoteTrackAttached"
                            && track.video_bytes_total > 0
                            && (now_ms - track.observed_at_ms).max(0.0)
                                <= TRANSPORT_AWAIT_CONNECTED_INGRESS_EVIDENCE_MAX_AGE_MS
                    });
            track_attached_and_recent && stats.inbound_primary_video_bytes_total > 0
        })
        .unwrap_or(false)
    }

    fn transport_await_soft_reentry_is_recent_and_healthy(
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        recovery_epoch: u64,
        now_ms: f64,
    ) -> bool {
        RuntimeStatsSink::read_shared(runtime_stats, |stats| {
            let chain_healthy = stats
                .latest_video_timeline_observation
                .as_ref()
                .is_some_and(|observation: &XbxEngineVideoTimelineObservation| {
                    observation.chain.state == "healthy"
                });
            chain_healthy
                && Self::has_recent_clean_anchor_evidence(
                    stats.video_anchor_clean_epoch,
                    stats.video_anchor_clean_observed_at_ms,
                    stats.video_anchor_clean_source_event.as_deref(),
                    stats.latest_anchor_candidate_ledger.as_ref(),
                    recovery_epoch,
                    now_ms,
                )
        })
        .unwrap_or(false)
    }

    fn has_recent_clean_anchor_evidence(
        clean_anchor_epoch: Option<u64>,
        clean_anchor_observed_at_ms: Option<f64>,
        clean_anchor_source_event: Option<&str>,
        latest_anchor_candidate_ledger: Option<&XbxEngineAnchorCandidateLedger>,
        recovery_epoch: u64,
        now_ms: f64,
    ) -> bool {
        let explicit_clean_anchor = clean_anchor_epoch.is_some_and(|epoch| {
            Self::clean_anchor_epoch_is_usable(
                epoch,
                clean_anchor_observed_at_ms,
                recovery_epoch,
                now_ms,
            )
        }) && clean_anchor_source_event
            == Some("chain-clean-keyframe-submitted");
        if explicit_clean_anchor {
            return true;
        }
        latest_anchor_candidate_ledger.is_some_and(|candidate| {
            candidate.state == XbxEngineAnchorCandidateState::SubmittedCleanAnchor
                && candidate.source_event == "chain-clean-keyframe-submitted"
                && Self::clean_anchor_epoch_is_usable(
                    candidate.recovery_epoch,
                    Some(candidate.observed_at_ms),
                    recovery_epoch,
                    now_ms,
                )
        })
    }

    fn clean_anchor_epoch_is_usable(
        anchor_epoch: u64,
        anchor_observed_at_ms: Option<f64>,
        current_recovery_epoch: u64,
        now_ms: f64,
    ) -> bool {
        const CLEAN_ANCHOR_EPOCH_GRACE_MAX_DELTA: u64 = 1;
        const CLEAN_ANCHOR_EPOCH_GRACE_WINDOW_MS: f64 = 1_500.0;
        if anchor_epoch == current_recovery_epoch {
            return true;
        }
        if current_recovery_epoch < anchor_epoch {
            return false;
        }
        let epoch_delta = current_recovery_epoch - anchor_epoch;
        if epoch_delta > CLEAN_ANCHOR_EPOCH_GRACE_MAX_DELTA {
            return false;
        }
        anchor_observed_at_ms.is_some_and(|anchor_ms| {
            (now_ms - anchor_ms).max(0.0) <= CLEAN_ANCHOR_EPOCH_GRACE_WINDOW_MS
        })
    }

    fn track_await_recovery_keyframe_streak(
        &mut self,
        reason: VideoEscalationReason,
        observed_at_ms: f64,
    ) {
        if reason != VideoEscalationReason::TransportAwaitRecoveryKeyframe {
            self.await_recovery_keyframe_streak = 0;
            self.await_recovery_keyframe_last_seen_at_ms = Some(observed_at_ms);
            self.await_recovery_keyframe_streak_started_at_ms = None;
            return;
        }
        let within_window = self
            .await_recovery_keyframe_last_seen_at_ms
            .map(|last| {
                (observed_at_ms - last).max(0.0)
                    <= TRANSPORT_AWAIT_RECOVERY_KEYFRAME_STREAK_WINDOW_MS
            })
            .unwrap_or(false);
        if within_window {
            self.await_recovery_keyframe_streak =
                self.await_recovery_keyframe_streak.saturating_add(1);
        } else {
            self.await_recovery_keyframe_streak = 1;
            self.await_recovery_keyframe_streak_started_at_ms = Some(observed_at_ms);
        }
        if self.await_recovery_keyframe_streak_started_at_ms.is_none() {
            self.await_recovery_keyframe_streak_started_at_ms = Some(observed_at_ms);
        }
        self.await_recovery_keyframe_last_seen_at_ms = Some(observed_at_ms);
    }

    pub fn poll_startup_retry(
        &mut self,
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
    ) -> Option<VideoEscalationDecision> {
        let phase =
            resolve_session_phase(runtime_stats, self.stream_started_at, self.startup_grace);
        let profile = resolve_recovery_profile(runtime_stats);
        if phase != SessionPhase::Startup {
            self.startup_probe.clear();
            return None;
        }
        if !self.startup_probe.should_retry_low_quality(
            runtime_stats,
            self.stream_started_at,
            self.startup_grace,
            Duration::from_millis(profile.startup_low_quality_retry_delay_ms),
            profile.startup_low_quality_floor_kbps,
            profile.startup_low_quality_recovered_kbps,
        ) {
            return None;
        }

        Some(
            self.escalation_controller
                .suppressed(RecoveryAction::StartupLowQualityRetry),
        )
    }

    #[cfg(test)]
    pub(crate) fn runtime_state_for_diagnosis(
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        diagnosis_label: &str,
        stream_started_at: Instant,
        startup_grace: Duration,
    ) -> RecoveryRuntimeState {
        build_runtime_state_for_diagnosis(
            runtime_stats,
            diagnosis_label,
            stream_started_at,
            startup_grace,
        )
    }

    fn resolve_decoder_backend_failure_recovery(
        &mut self,
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        reason: &VideoEscalationReason,
    ) -> Option<VideoEscalationDecision> {
        match resolve_decoder_backend_failure_recovery(runtime_stats, reason)? {
            DecoderBackendFailureResolution::Suppress(action) => {
                Some(self.escalation_controller.suppressed(action))
            }
            DecoderBackendFailureResolution::Escalate(profile) => {
                let phase = resolve_session_phase(
                    runtime_stats,
                    self.stream_started_at,
                    self.startup_grace,
                );
                Some(
                    self.on_reason_with_policy(
                        VideoEscalationReason::DecoderBackendFailure,
                        phase,
                        profile,
                        RuntimeStatsSink::read_shared(runtime_stats, |stats| {
                            stats.transport_recovery_epoch
                        })
                        .unwrap_or(0),
                        runtime_stats,
                        unix_now_ms(),
                    ),
                )
            }
        }
    }

    // 优先消费最近一次 NACK outcome：
    // - 已追回：不要立刻把恢复继续升级
    // - 已过期：只对关键/reference 帧升级；delta 直接放弃，避免拖慢后续帧
    fn resolve_recent_nack_outcome(
        &mut self,
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        reason: &VideoEscalationReason,
    ) -> Option<VideoEscalationDecision> {
        match resolve_recent_nack_outcome(
            runtime_stats,
            reason,
            self.stream_started_at,
            self.startup_grace,
            &mut self.cloud_startup_nack_budget,
        )? {
            RecentNackOutcomeResolution::Suppress(action) => {
                Some(self.escalation_controller.suppressed(action))
            }
            RecentNackOutcomeResolution::Escalate(reason) => {
                let phase = resolve_session_phase(
                    runtime_stats,
                    self.stream_started_at,
                    self.startup_grace,
                );
                let profile = resolve_recovery_profile(runtime_stats);
                let recovery_epoch = RuntimeStatsSink::read_shared(runtime_stats, |stats| {
                    stats.transport_recovery_epoch
                })
                .unwrap_or(0);
                Some(self.on_reason_with_policy(
                    reason,
                    phase,
                    profile,
                    recovery_epoch,
                    runtime_stats,
                    unix_now_ms(),
                ))
            }
        }
    }

    // 已经进入同一轮恢复时，短窗口内抑制重复 reason：
    // - WaitKeyframe 在刚发过 keyframe/reset 后，先别继续一帧一帧推高恢复动作
    // - AdapterIdleTimeout 在刚发过 decoder reset 后，先观察这一轮恢复是否生效
    fn resolve_recent_repeat_suppression(
        &mut self,
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        reason: &VideoEscalationReason,
    ) -> Option<VideoEscalationDecision> {
        resolve_recent_repeat_suppression(runtime_stats, reason)
            .map(|action| self.escalation_controller.suppressed(action))
    }

    // 当视频已经长时间 0kbps 且没有任何新呈现时，不允许一直停在 cooldownSuppressed。
    // 这条链只依赖“当前已进入硬停滞事实”，不能再绑定单个 diagnosis label，
    // 否则 transportExpiredDeadline / severe deadline 会卡在 cooldownSuppressed 而无法升级。
    fn resolve_persistent_stall_recovery(
        &mut self,
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        reason: &VideoEscalationReason,
    ) -> Option<VideoEscalationDecision> {
        resolve_persistent_stall_recovery(runtime_stats, reason)
    }
}

fn classify_signal_domain(reason: VideoEscalationReason) -> RecoverySignalDomain {
    match reason {
        VideoEscalationReason::LifecycleRecovering => RecoverySignalDomain::Connectivity,
        VideoEscalationReason::WaitKeyframe
        | VideoEscalationReason::TransportAwaitRecoveryKeyframe
        | VideoEscalationReason::Reconfigure
        | VideoEscalationReason::DecoderBackendFailure
        | VideoEscalationReason::AdapterIdleTimeout
        | VideoEscalationReason::AdapterThinStream
        | VideoEscalationReason::TransportExpiredDeadline
        | VideoEscalationReason::TransportSevereDeadline
        | VideoEscalationReason::TransportRecoveredLate
        | VideoEscalationReason::TransportSampleLoss => RecoverySignalDomain::MediaRecovery,
    }
}

#[cfg(test)]
mod tests {
    use super::{RecoveryCoordinator, RecoveryOwnerSignal};
    use crate::runtime_stats_sink::RuntimeStatsSink;
    use crate::transport::rtc::recovery::escalation::{
        RecoveryAction, VideoEscalationConfig, VideoEscalationController, VideoEscalationReason,
    };
    use crate::transport::rtc::recovery::runtime_state::{
        resolve_recovery_coupling_state, resolve_recovery_profile, unix_now_ms,
        RecoveryCouplingMode,
    };
    use crate::transport::rtc::recovery::startup::SessionPhase;
    use crate::XbxEngineMediaRuntimeStats;
    use crate::{
        XbxEngineVideoNackObservation, XbxEngineVideoTrackStatus, XbxEngineVideoTwccObservation,
    };
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    use xbxengine_protocol::{XbxEngineTargetTypeDto, XbxEngineTransportStateDto};

    fn test_escalation_controller(
        cooldown_ms: u64,
        keyframe_burst_threshold: u8,
        decoder_reset_burst_threshold: u8,
    ) -> VideoEscalationController {
        VideoEscalationController::new(VideoEscalationConfig {
            cooldown_ms,
            keyframe_burst_threshold,
            decoder_reset_burst_threshold,
            keyframe_min_interval_ms: cooldown_ms,
            escalation_window_ms: cooldown_ms.saturating_mul(3),
            keyframe_upgrade_min_delay_ms: (cooldown_ms / 2).max(40),
        })
    }

    #[test]
    fn home_lan_uses_aggressive_startup_recovery_profile() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.session_target_type = Some(XbxEngineTargetTypeDto::Home);
        stats.transport_path = Some("Direct (host->host)".to_string());
        let profile = resolve_recovery_profile(&Mutex::new(stats));
        assert!(profile.startup_fast_reset_enabled);
        assert_eq!(profile.startup_low_quality_retry_delay_ms, 320);
        assert_eq!(profile.startup_low_quality_floor_kbps, 8_000.0);
        assert_eq!(profile.startup_low_quality_recovered_kbps, 12_000.0);
        assert_eq!(profile.escalation_cooldown_ms, 260);
        assert_eq!(profile.escalation_keyframe_min_interval_ms, 260);
    }

    #[test]
    fn relay_home_uses_conservative_startup_recovery_profile() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.session_target_type = Some(XbxEngineTargetTypeDto::Home);
        stats.transport_path = Some("Relay".to_string());
        let profile = resolve_recovery_profile(&Mutex::new(stats));
        assert!(!profile.startup_fast_reset_enabled);
        assert_eq!(profile.startup_low_quality_retry_delay_ms, 650);
        assert_eq!(profile.startup_low_quality_floor_kbps, 6_000.0);
        assert_eq!(profile.startup_low_quality_recovered_kbps, 10_000.0);
        assert_eq!(profile.escalation_cooldown_ms, 360);
        assert_eq!(profile.escalation_keyframe_min_interval_ms, 360);
    }

    #[test]
    fn cloud_uses_relaxed_startup_recovery_profile() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.session_target_type = Some(XbxEngineTargetTypeDto::Cloud);
        stats.transport_path = Some("Direct (host->host)".to_string());
        let profile = resolve_recovery_profile(&Mutex::new(stats));
        assert!(!profile.startup_fast_reset_enabled);
        assert_eq!(profile.startup_low_quality_retry_delay_ms, 650);
        assert_eq!(profile.startup_low_quality_floor_kbps, 14_000.0);
        assert_eq!(profile.startup_low_quality_recovered_kbps, 20_000.0);
        assert_eq!(profile.escalation_cooldown_ms, 650);
        assert_eq!(profile.escalation_keyframe_min_interval_ms, 650);
        assert_eq!(profile.escalation_upgrade_window_ms, 2_600);
        assert_eq!(profile.escalation_keyframe_upgrade_min_delay_ms, 550);
        assert_eq!(profile.hard_fallback_transport_await_timeout_ms, 6_500);
        assert_eq!(
            profile.display_supply_thresholds.degraded_no_pending_streak,
            64
        );
        assert_eq!(
            profile.display_supply_thresholds.critical_no_pending_streak,
            128
        );
    }

    fn healthy_twcc_observation(now_ms: f64) -> XbxEngineVideoTwccObservation {
        XbxEngineVideoTwccObservation {
            observation_id: 1,
            source: "local-feedback".to_string(),
            feedback_packet_count: 20,
            covered_sequence_start: 10,
            covered_sequence_end: 29,
            covered_sequence_span: 20,
            observed_packet_count: 20,
            observed_byte_count: 32_000,
            coverage_ratio: None,
            ledger_hit_ratio: None,
            feedback_interval_ms: Some(100.0),
            arrival_span_ms: Some(95.0),
            receive_bitrate_kbps: Some(18_000.0),
            twcc_sample_valid: true,

            twcc_invalid_reason: None,

            quality: crate::XbxEngineTwccObservationQuality::Stable,
            delivery_ratio: 0.99,
            packet_loss_ratio: 0.01,
            observed_at_ms: now_ms,
        }
    }

    fn make_test_nack_observation(
        action: &str,
        frame_importance: &str,
        retry_count: u8,
        observed_at_ms: f64,
    ) -> XbxEngineVideoNackObservation {
        XbxEngineVideoNackObservation {
            observation_id: 1,
            action: action.to_string(),
            source: "sampleLoss".to_string(),
            first_sequence: 1,
            last_sequence: 2,
            packet_count: 2,
            retry_count,
            frame_rtp_timestamp: Some(1),
            frame_is_keyframe: Some(frame_importance == "keyframe"),
            frame_importance: Some(frame_importance.to_string()),
            deadline_at_ms: None,
            estimated_recovery_arrival_ms: None,
            nack_disposition: Some("attempted".to_string()),
            frame_playout_deadline_at_ms: None,
            frame_unrecoverable_reason: None,
            observed_at_ms,
        }
    }

    #[test]
    fn recovered_nack_suppresses_transport_sample_loss_escalation() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.latest_video_nack_observation = Some(make_test_nack_observation(
            "recovered",
            "delta",
            0,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as f64,
        ));
        let mut coordinator = RecoveryCoordinator::new(
            test_escalation_controller(250, 2, 2),
            Instant::now(),
            Duration::from_millis(800),
        );
        let decision = coordinator.on_reason_with_runtime_stats(
            VideoEscalationReason::TransportSampleLoss,
            &Mutex::new(stats),
        );
        assert_eq!(decision.action, RecoveryAction::CooldownSuppressed);
    }

    #[test]
    fn expired_delta_nack_stays_suppressed_without_stall_signal() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Home);
        stats.latest_video_nack_observation = Some(make_test_nack_observation(
            "expiredDeadline",
            "delta",
            2,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as f64,
        ));
        let mut coordinator = RecoveryCoordinator::new(
            test_escalation_controller(250, 2, 2),
            Instant::now(),
            Duration::from_millis(800),
        );
        let decision = coordinator.on_reason_with_runtime_stats(
            VideoEscalationReason::TransportSampleLoss,
            &Mutex::new(stats),
        );
        assert_eq!(decision.action, RecoveryAction::CooldownSuppressed);
    }

    #[test]
    fn expired_delta_nack_in_cloud_requires_continuous_budget_before_keyframe() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud);
        let observed_at_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as f64;
        let mut coordinator = RecoveryCoordinator::new(
            test_escalation_controller(250, 1, 1),
            Instant::now(),
            Duration::from_secs(2),
        );

        stats.latest_video_nack_observation = Some(make_test_nack_observation(
            "expiredDeadline",
            "delta",
            2,
            observed_at_ms,
        ));
        let shared_stats = Mutex::new(stats);
        let decision = coordinator.on_reason_with_runtime_stats(
            VideoEscalationReason::TransportExpiredDeadline,
            &shared_stats,
        );
        assert_eq!(decision.action, RecoveryAction::CooldownSuppressed);

        RuntimeStatsSink::update_shared(&shared_stats, |stats| {
            if let Some(observation) = stats.latest_video_nack_observation.as_mut() {
                observation.observation_id = 2;
                observation.observed_at_ms += 180.0;
            }
        });
        let decision = coordinator.on_reason_with_runtime_stats(
            VideoEscalationReason::TransportExpiredDeadline,
            &shared_stats,
        );
        assert_eq!(decision.action, RecoveryAction::CooldownSuppressed);

        RuntimeStatsSink::update_shared(&shared_stats, |stats| {
            if let Some(observation) = stats.latest_video_nack_observation.as_mut() {
                observation.observation_id = 3;
                observation.observed_at_ms += 180.0;
            }
        });
        let decision = coordinator.on_reason_with_runtime_stats(
            VideoEscalationReason::TransportExpiredDeadline,
            &shared_stats,
        );
        assert_eq!(decision.action, RecoveryAction::RequestKeyframe);
    }

    #[test]
    fn expired_delta_nack_requests_keyframe_when_pipeline_is_stalled() {
        let now_ms = unix_now_ms();
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.video_renderer_stalled = Some(true);
        stats.latest_video_present_time_ms = Some(now_ms - 2_000.0);
        stats.latest_video_packet_arrival_time_ms = Some(now_ms - 40.0);
        stats.latest_video_nack_observation = Some(make_test_nack_observation(
            "expiredDeadline",
            "delta",
            2,
            now_ms,
        ));
        let mut coordinator = RecoveryCoordinator::new(
            test_escalation_controller(250, 1, 1),
            Instant::now() - Duration::from_secs(5),
            Duration::from_millis(800),
        );
        let decision = coordinator.on_reason_with_runtime_stats(
            VideoEscalationReason::TransportExpiredDeadline,
            &Mutex::new(stats),
        );
        assert_eq!(decision.action, RecoveryAction::RequestKeyframe);
    }

    #[test]
    fn decoder_backend_failure_prioritizes_decoder_reset_over_transport_suppression() {
        let now_ms = unix_now_ms();
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.transport_state = XbxEngineTransportStateDto::Connected;
        stats.latest_video_packet_arrival_time_ms = Some(now_ms - 30.0);
        stats.latest_video_decode_ok_time_ms = Some(now_ms - 1_800.0);
        stats.latest_video_present_time_ms = Some(now_ms - 1_800.0);
        stats.video_renderer_stalled = Some(true);
        stats.latest_video_twcc_observation = Some(healthy_twcc_observation(now_ms - 20.0));
        stats.video_decoder_hardware_failure_streak = 4;
        stats.latest_video_decoder_hardware_failure_time_ms = Some(now_ms - 25.0);
        stats.latest_video_decoder_reset_time_ms = Some(now_ms - 2_500.0);
        stats.latest_video_nack_observation = Some(make_test_nack_observation(
            "expiredDeadline",
            "delta",
            2,
            now_ms,
        ));

        let mut coordinator = RecoveryCoordinator::new(
            test_escalation_controller(250, 2, 2),
            Instant::now() - Duration::from_secs(5),
            Duration::from_millis(800),
        );
        let decision = coordinator.on_reason_with_runtime_stats(
            VideoEscalationReason::TransportExpiredDeadline,
            &Mutex::new(stats),
        );
        assert_eq!(decision.action, RecoveryAction::RequestDecoderReset);
    }

    #[test]
    fn decoder_backend_failure_respects_reset_spacing_cooldown() {
        let now_ms = unix_now_ms();
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.transport_state = XbxEngineTransportStateDto::Connected;
        stats.latest_video_packet_arrival_time_ms = Some(now_ms - 30.0);
        stats.latest_video_decode_ok_time_ms = Some(now_ms - 1_800.0);
        stats.latest_video_present_time_ms = Some(now_ms - 1_800.0);
        stats.video_renderer_stalled = Some(true);
        stats.latest_video_twcc_observation = Some(healthy_twcc_observation(now_ms - 20.0));
        stats.video_decoder_hardware_failure_streak = 5;
        stats.latest_video_decoder_hardware_failure_time_ms = Some(now_ms - 15.0);
        stats.latest_video_decoder_reset_time_ms = Some(now_ms - 100.0);

        let mut coordinator = RecoveryCoordinator::new(
            test_escalation_controller(250, 2, 2),
            Instant::now() - Duration::from_secs(5),
            Duration::from_millis(800),
        );
        let decision = coordinator.on_reason_with_runtime_stats(
            VideoEscalationReason::TransportExpiredDeadline,
            &Mutex::new(stats),
        );
        assert_eq!(decision.action, RecoveryAction::CooldownSuppressed);
    }

    #[test]
    fn runtime_state_overrides_transport_diagnosis_to_decoder_backend_failure() {
        let now_ms = unix_now_ms();
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.transport_state = XbxEngineTransportStateDto::Connected;
        stats.latest_video_packet_arrival_time_ms = Some(now_ms - 20.0);
        stats.latest_video_decode_ok_time_ms = Some(now_ms - 1_800.0);
        stats.latest_video_present_time_ms = Some(now_ms - 1_800.0);
        stats.video_renderer_stalled = Some(true);
        stats.latest_video_twcc_observation = Some(healthy_twcc_observation(now_ms - 20.0));
        stats.video_decoder_hardware_failure_streak = 3;
        stats.latest_video_decoder_hardware_failure_time_ms = Some(now_ms - 20.0);

        let state = RecoveryCoordinator::runtime_state_for_diagnosis(
            &Mutex::new(stats),
            "transportExpiredDeadline",
            Instant::now() - Duration::from_secs(3),
            Duration::from_millis(800),
        );
        assert_eq!(state.phase, SessionPhase::Recovering);
        assert_eq!(state.recovery_policy_profile, "homeLanGaming");
        assert_eq!(
            state.coupling.mode,
            RecoveryCouplingMode::RecoveringReferenceChain
        );
        assert_eq!(state.diagnosis_label, "decoderBackendFailure");
    }

    #[test]
    fn runtime_state_keeps_transport_diagnosis_when_pipeline_is_still_advancing() {
        let now_ms = unix_now_ms();
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.transport_state = XbxEngineTransportStateDto::Connected;
        stats.latest_video_packet_arrival_time_ms = Some(now_ms - 20.0);
        stats.latest_video_decode_ok_time_ms = Some(now_ms - 60.0);
        stats.latest_video_present_time_ms = Some(now_ms - 60.0);
        stats.video_renderer_stalled = Some(false);
        stats.latest_video_twcc_observation = Some(healthy_twcc_observation(now_ms - 20.0));
        stats.video_decoder_hardware_failure_streak = 4;
        stats.latest_video_decoder_hardware_failure_time_ms = Some(now_ms - 20.0);

        let state = RecoveryCoordinator::runtime_state_for_diagnosis(
            &Mutex::new(stats),
            "transportExpiredDeadline",
            Instant::now() - Duration::from_secs(3),
            Duration::from_millis(800),
        );
        assert_eq!(state.phase, SessionPhase::Steady);
        assert_eq!(state.recovery_policy_profile, "homeLanGaming");
        assert_eq!(state.coupling.mode, RecoveryCouplingMode::Healthy);
        assert_eq!(state.diagnosis_label, "transportExpiredDeadline");
    }

    #[test]
    fn recovered_reference_nack_waits_for_burst_in_wait_keyframe_chain() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        let mut observation = make_test_nack_observation(
            "recovered",
            "reference",
            0,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as f64,
        );
        observation.frame_is_keyframe = Some(true);
        stats.latest_video_nack_observation = Some(observation);
        let mut coordinator = RecoveryCoordinator::new(
            test_escalation_controller(250, 2, 2),
            Instant::now(),
            Duration::from_millis(800),
        );
        let decision = coordinator
            .on_reason_with_runtime_stats(VideoEscalationReason::WaitKeyframe, &Mutex::new(stats));
        assert_eq!(decision.action, RecoveryAction::WaitForBurst);
    }

    #[test]
    fn expired_reference_nack_pushes_idle_timeout_into_recovery_chain() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        let mut observation = make_test_nack_observation(
            "expiredDeadline",
            "reference",
            2,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as f64,
        );
        observation.frame_is_keyframe = Some(true);
        stats.latest_video_nack_observation = Some(observation);
        let mut coordinator = RecoveryCoordinator::new(
            test_escalation_controller(250, 1, 1),
            Instant::now() - Duration::from_secs(5),
            Duration::from_millis(800),
        );
        let decision = coordinator.on_reason_with_runtime_stats(
            VideoEscalationReason::AdapterIdleTimeout,
            &Mutex::new(stats),
        );
        assert_eq!(decision.action, RecoveryAction::RequestKeyframe);
    }

    #[test]
    fn recent_wait_keyframe_recovery_suppresses_repeat_wait_keyframe() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.transport_recovery_epoch = 4;
        stats.transport_recovery_epoch_at_last_escalation = 4;
        stats.latest_video_escalation_observation =
            Some(crate::XbxEngineVideoEscalationObservation {
                observation_id: 7,
                reason: "ingressWaitKeyframe".to_string(),
                action: "requestKeyframe".to_string(),
                observed_at_ms: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as f64,
            });
        let mut coordinator = RecoveryCoordinator::new(
            test_escalation_controller(250, 2, 2),
            Instant::now(),
            Duration::from_millis(800),
        );
        let decision = coordinator
            .on_reason_with_runtime_stats(VideoEscalationReason::WaitKeyframe, &Mutex::new(stats));
        assert_eq!(decision.action, RecoveryAction::CooldownSuppressed);
    }

    #[test]
    fn new_transport_recovery_epoch_breaks_wait_keyframe_suppression() {
        let now_ms = unix_now_ms();
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.transport_recovery_epoch = 5;
        stats.transport_recovery_epoch_at_last_escalation = 4;
        stats.latest_video_escalation_observation =
            Some(crate::XbxEngineVideoEscalationObservation {
                observation_id: 17,
                reason: "ingressWaitKeyframe".to_string(),
                action: "requestKeyframe".to_string(),
                observed_at_ms: now_ms - 40.0,
            });
        let mut coordinator = RecoveryCoordinator::new(
            test_escalation_controller(250, 2, 2),
            Instant::now(),
            Duration::from_millis(800),
        );
        let decision = coordinator
            .on_reason_with_runtime_stats(VideoEscalationReason::WaitKeyframe, &Mutex::new(stats));
        assert_ne!(decision.action, RecoveryAction::CooldownSuppressed);
    }

    #[test]
    fn cooldown_suppressed_observation_does_not_self_lock_wait_keyframe_chain() {
        let now_ms = unix_now_ms();
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.latest_video_escalation_observation =
            Some(crate::XbxEngineVideoEscalationObservation {
                observation_id: 9,
                reason: "ingressWaitKeyframe".to_string(),
                action: "cooldownSuppressed".to_string(),
                observed_at_ms: now_ms - 50.0,
            });
        let mut coordinator = RecoveryCoordinator::new(
            test_escalation_controller(250, 2, 2),
            Instant::now() - Duration::from_secs(3),
            Duration::from_millis(800),
        );
        let decision = coordinator
            .on_reason_with_runtime_stats(VideoEscalationReason::WaitKeyframe, &Mutex::new(stats));
        assert_ne!(decision.action, RecoveryAction::CooldownSuppressed);
    }

    #[test]
    fn recent_idle_timeout_decoder_reset_suppresses_repeat_idle_timeout() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.transport_recovery_epoch = 3;
        stats.transport_recovery_epoch_at_last_escalation = 3;
        stats.latest_video_escalation_observation =
            Some(crate::XbxEngineVideoEscalationObservation {
                observation_id: 8,
                reason: "adapterIdleTimeout".to_string(),
                action: "requestDecoderReset".to_string(),
                observed_at_ms: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as f64,
            });
        let mut coordinator = RecoveryCoordinator::new(
            test_escalation_controller(250, 2, 2),
            Instant::now(),
            Duration::from_millis(800),
        );
        let decision = coordinator.on_reason_with_runtime_stats(
            VideoEscalationReason::AdapterIdleTimeout,
            &Mutex::new(stats),
        );
        assert_eq!(decision.action, RecoveryAction::CooldownSuppressed);
    }

    #[test]
    fn hard_paused_stream_retries_decoder_reset_after_timeout() {
        let now_ms = unix_now_ms();
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.transport_state = XbxEngineTransportStateDto::Connected;
        stats.inbound_video_bitrate_kbps = Some(0.0);
        stats.direct_gaming_bitrate_band = Some("paused".to_string());
        stats.video_present_fps = 0.0;
        stats.latest_video_present_time_ms = Some(now_ms - 1_600.0);
        stats.latest_video_packet_arrival_time_ms = Some(now_ms - 1_600.0);
        stats.latest_video_decoder_reset_time_ms = Some(now_ms - 1_400.0);
        let mut coordinator = RecoveryCoordinator::new(
            test_escalation_controller(250, 2, 2),
            Instant::now(),
            Duration::from_millis(800),
        );
        let decision = coordinator.on_reason_with_runtime_stats(
            VideoEscalationReason::AdapterIdleTimeout,
            &Mutex::new(stats),
        );
        assert_eq!(decision.action, RecoveryAction::RequestDecoderReset);
    }

    #[test]
    fn hard_paused_stream_prefers_decoder_reset_over_reconnect_after_long_stall() {
        let now_ms = unix_now_ms();
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.transport_state = XbxEngineTransportStateDto::Connected;
        stats.inbound_video_bitrate_kbps = Some(0.0);
        stats.direct_gaming_bitrate_band = Some("paused".to_string());
        stats.video_present_fps = 0.0;
        stats.latest_video_present_time_ms = Some(now_ms - 3_600.0);
        stats.latest_video_packet_arrival_time_ms = Some(now_ms - 3_600.0);
        stats.latest_video_decoder_reset_time_ms = Some(now_ms - 2_200.0);
        stats.latest_video_escalation_observation =
            Some(crate::XbxEngineVideoEscalationObservation {
                observation_id: 11,
                reason: "adapterIdleTimeout".to_string(),
                action: "cooldownSuppressed".to_string(),
                observed_at_ms: now_ms - 200.0,
            });
        let mut coordinator = RecoveryCoordinator::new(
            test_escalation_controller(250, 2, 2),
            Instant::now(),
            Duration::from_millis(800),
        );
        let decision = coordinator.on_reason_with_runtime_stats(
            VideoEscalationReason::AdapterIdleTimeout,
            &Mutex::new(stats),
        );
        assert_eq!(decision.action, RecoveryAction::RequestDecoderReset);
    }

    #[test]
    fn hard_paused_stream_with_renderer_stall_prefers_decoder_reset_over_reconnect() {
        let now_ms = unix_now_ms();
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.transport_state = XbxEngineTransportStateDto::Connected;
        stats.inbound_video_bitrate_kbps = Some(0.0);
        stats.direct_gaming_bitrate_band = Some("paused".to_string());
        stats.video_present_fps = 30.0;
        stats.video_renderer_stalled = Some(true);
        stats.latest_video_present_time_ms = Some(now_ms - 3_600.0);
        stats.latest_video_packet_arrival_time_ms = Some(now_ms - 3_600.0);
        stats.latest_video_decoder_reset_time_ms = Some(now_ms - 2_200.0);
        stats.latest_video_escalation_observation =
            Some(crate::XbxEngineVideoEscalationObservation {
                observation_id: 12,
                reason: "adapterIdleTimeout".to_string(),
                action: "cooldownSuppressed".to_string(),
                observed_at_ms: now_ms - 200.0,
            });
        let mut coordinator = RecoveryCoordinator::new(
            test_escalation_controller(250, 2, 2),
            Instant::now(),
            Duration::from_millis(800),
        );
        let decision = coordinator.on_reason_with_runtime_stats(
            VideoEscalationReason::AdapterIdleTimeout,
            &Mutex::new(stats),
        );
        assert_eq!(decision.action, RecoveryAction::RequestDecoderReset);
    }

    #[test]
    fn transport_expired_deadline_hard_pause_prefers_decoder_reset_over_reconnect() {
        let now_ms = unix_now_ms();
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.transport_state = XbxEngineTransportStateDto::Connected;
        stats.inbound_video_bitrate_kbps = Some(0.0);
        stats.direct_gaming_bitrate_band = Some("paused".to_string());
        stats.video_present_fps = 0.0;
        stats.latest_video_present_time_ms = Some(now_ms - 3_600.0);
        stats.latest_video_packet_arrival_time_ms = Some(now_ms - 3_600.0);
        stats.latest_video_decoder_reset_time_ms = Some(now_ms - 2_200.0);
        stats.latest_video_escalation_observation =
            Some(crate::XbxEngineVideoEscalationObservation {
                observation_id: 13,
                reason: "transportExpiredDeadline".to_string(),
                action: "cooldownSuppressed".to_string(),
                observed_at_ms: now_ms - 200.0,
            });
        let mut coordinator = RecoveryCoordinator::new(
            test_escalation_controller(250, 2, 2),
            Instant::now(),
            Duration::from_millis(800),
        );
        let decision = coordinator.on_reason_with_runtime_stats(
            VideoEscalationReason::TransportExpiredDeadline,
            &Mutex::new(stats),
        );
        assert_eq!(decision.action, RecoveryAction::RequestDecoderReset);
    }

    #[test]
    fn startup_low_quality_maps_to_coupled_hold_mode() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.session_phase = Some("startup".to_string());
        stats.direct_gaming_bitrate_band = Some("startupLow".to_string());
        let state = resolve_recovery_coupling_state(&Mutex::new(stats), SessionPhase::Startup);
        assert_eq!(state.mode, RecoveryCouplingMode::StartupLowQuality);
        assert!(state.suppress_ramp_up);
        assert!(state.prefer_hold);
        assert!(!state.allow_peak_range);
    }

    #[test]
    fn adapter_idle_timeout_maps_to_stalled_coupling() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.recovery_diagnosis = Some("adapterIdleTimeout".to_string());
        let state = resolve_recovery_coupling_state(&Mutex::new(stats), SessionPhase::Recovering);
        assert_eq!(state.mode, RecoveryCouplingMode::Stalled);
        assert!(state.suppress_ramp_up);
        assert!(state.prefer_hold);
        assert!(!state.allow_peak_range);
    }

    #[test]
    fn adapter_idle_timeout_is_ignored_when_output_is_fresh() {
        let now_ms = unix_now_ms();
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.recovery_diagnosis = Some("adapterIdleTimeout".to_string());
        stats.latest_video_present_time_ms = Some(now_ms - 40.0);
        stats.latest_video_decode_ok_time_ms = Some(now_ms - 40.0);
        stats.video_present_fps = 58.0;
        let state = resolve_recovery_coupling_state(&Mutex::new(stats), SessionPhase::Steady);
        assert_eq!(state.mode, RecoveryCouplingMode::Healthy);
        assert!(!state.suppress_ramp_up);
        assert!(state.allow_peak_range);
    }

    #[test]
    fn adapter_idle_timeout_is_downgraded_when_audio_only_and_recovery_recent() {
        let now_ms = unix_now_ms();
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.recovery_diagnosis = Some("adapterIdleTimeout".to_string());
        stats.latest_video_escalation_observation =
            Some(crate::XbxEngineVideoEscalationObservation {
                observation_id: 1,
                reason: "adapterIdleTimeout".to_string(),
                action: "requestDecoderReset".to_string(),
                observed_at_ms: now_ms - 500.0,
            });
        stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
            state: "audioOnly".to_string(),
            video_width: None,
            video_height: None,
            mime_type: None,
            transport_state: XbxEngineTransportStateDto::Connected,
            video_bytes_total: 0,
            video_packet_count_total: 0,
            audio_bytes_total: 42,
            observed_at_ms: now_ms,
        });

        let state = RecoveryCoordinator::runtime_state_for_diagnosis(
            &Mutex::new(stats),
            "adapterIdleTimeout",
            Instant::now(),
            Duration::from_millis(800),
        );
        assert_eq!(state.phase, SessionPhase::Startup);
        assert_eq!(state.recovery_policy_profile, "homeLanGaming");
        assert_eq!(state.coupling.mode, RecoveryCouplingMode::Stalled);
        assert_eq!(state.diagnosis_label, "healthy");
    }

    #[test]
    fn steady_healthy_output_exits_recovery_coupling() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.inbound_video_bitrate_kbps = Some(16_500.0);
        stats.video_present_fps = 58.0;
        let state = resolve_recovery_coupling_state(&Mutex::new(stats), SessionPhase::Steady);
        assert_eq!(state.mode, RecoveryCouplingMode::Healthy);
        assert!(!state.suppress_ramp_up);
        assert!(state.allow_peak_range);
    }

    #[test]
    fn owner_signal_is_preserved_through_coordinator_proposal() {
        let shared_stats = Mutex::new(XbxEngineMediaRuntimeStats::default());
        let mut coordinator = RecoveryCoordinator::new(
            test_escalation_controller(250, 2, 2),
            Instant::now(),
            Duration::from_millis(800),
        );
        let signal = RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportSampleLoss,
            reason_label: "displaySupplyNoPending".to_string(),
            observed_at_ms: unix_now_ms(),
        };
        let proposal = coordinator.propose_from_owner_signal(signal, &shared_stats);
        assert_eq!(
            proposal.signal.reason,
            VideoEscalationReason::TransportSampleLoss
        );
        assert_eq!(proposal.signal.reason_label, "displaySupplyNoPending");
    }

    #[test]
    fn wait_keyframe_escalation_budget_is_released_after_new_epoch() {
        let now_ms = unix_now_ms();
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.transport_recovery_epoch = 8;
        let shared_stats = Mutex::new(stats);
        let mut coordinator = RecoveryCoordinator::new(
            test_escalation_controller(120, 1, 1),
            Instant::now() - Duration::from_secs(3),
            Duration::from_millis(800),
        );

        let first = coordinator
            .on_reason_with_runtime_stats(VideoEscalationReason::WaitKeyframe, &shared_stats);
        assert_eq!(first.action, RecoveryAction::RequestKeyframe);

        let second = coordinator
            .on_reason_with_runtime_stats(VideoEscalationReason::WaitKeyframe, &shared_stats);
        assert_eq!(second.action, RecoveryAction::CooldownSuppressed);

        RuntimeStatsSink::update_shared(&shared_stats, |stats| {
            stats.transport_recovery_epoch = 9;
            stats.transport_recovery_epoch_at_last_escalation = 8;
            stats.latest_video_escalation_observation =
                Some(crate::XbxEngineVideoEscalationObservation {
                    observation_id: 200,
                    reason: "waitKeyframe".to_string(),
                    action: "requestKeyframe".to_string(),
                    observed_at_ms: now_ms - 50.0,
                });
        });
        std::thread::sleep(Duration::from_millis(130));

        let third = coordinator
            .on_reason_with_runtime_stats(VideoEscalationReason::WaitKeyframe, &shared_stats);
        assert_ne!(third.action, RecoveryAction::CooldownSuppressed);
    }

    #[test]
    fn coordinator_staged_recovery_avoids_single_keyframe_hang_for_transport_await() {
        let now_ms = unix_now_ms();
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.transport_recovery_epoch = 10;
        let shared_stats = Mutex::new(stats);
        let mut coordinator = RecoveryCoordinator::new(
            test_escalation_controller(120, 1, 1),
            Instant::now() - Duration::from_secs(3),
            Duration::from_millis(800),
        );

        let first = coordinator.propose_from_owner_signal(
            RecoveryOwnerSignal {
                reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
                reason_label: "transportAwaitRecoveryKeyframe".to_string(),
                observed_at_ms: now_ms,
            },
            &shared_stats,
        );
        assert_eq!(first.decision.action, RecoveryAction::RequestKeyframe);

        let second = coordinator.propose_from_owner_signal(
            RecoveryOwnerSignal {
                reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
                reason_label: "transportAwaitRecoveryKeyframe".to_string(),
                observed_at_ms: now_ms + 80.0,
            },
            &shared_stats,
        );
        assert_eq!(second.decision.action, RecoveryAction::CooldownSuppressed);

        std::thread::sleep(Duration::from_millis(420));
        let third = coordinator.propose_from_owner_signal(
            RecoveryOwnerSignal {
                reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
                reason_label: "transportAwaitRecoveryKeyframe".to_string(),
                observed_at_ms: now_ms + 420.0,
            },
            &shared_stats,
        );
        assert_eq!(third.decision.action, RecoveryAction::RequestDecoderReset);
    }

    #[test]
    fn transport_await_with_connected_stall_evidence_escalates_on_second_post_cooldown_tick() {
        let now_ms = unix_now_ms();
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.transport_recovery_epoch = 13;
        stats.transport_state = XbxEngineTransportStateDto::Connected;
        stats.video_renderer_stalled = Some(true);
        stats.latest_video_present_time_ms = Some(now_ms - 2_000.0);
        let shared_stats = Mutex::new(stats);
        let mut coordinator = RecoveryCoordinator::new(
            test_escalation_controller(120, 1, 1),
            Instant::now() - Duration::from_secs(3),
            Duration::from_millis(800),
        );

        let first = coordinator.propose_from_owner_signal(
            RecoveryOwnerSignal {
                reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
                reason_label: "transportAwaitRecoveryKeyframe".to_string(),
                observed_at_ms: now_ms,
            },
            &shared_stats,
        );
        assert_eq!(first.decision.action, RecoveryAction::RequestKeyframe);

        std::thread::sleep(Duration::from_millis(220));
        let second = coordinator.propose_from_owner_signal(
            RecoveryOwnerSignal {
                reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
                reason_label: "transportAwaitRecoveryKeyframe".to_string(),
                observed_at_ms: now_ms + 220.0,
            },
            &shared_stats,
        );
        assert_eq!(second.decision.action, RecoveryAction::RequestDecoderReset);
    }

    #[test]
    fn coordinator_staged_recovery_handles_sparse_transport_await_signals() {
        let now_ms = unix_now_ms();
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.transport_recovery_epoch = 14;
        let shared_stats = Mutex::new(stats);
        let mut coordinator = RecoveryCoordinator::new(
            test_escalation_controller(120, 1, 1),
            Instant::now() - Duration::from_secs(3),
            Duration::from_millis(800),
        );

        let first = coordinator.propose_from_owner_signal(
            RecoveryOwnerSignal {
                reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
                reason_label: "transportAwaitRecoveryKeyframe".to_string(),
                observed_at_ms: now_ms,
            },
            &shared_stats,
        );
        assert_eq!(first.decision.action, RecoveryAction::RequestKeyframe);

        let second = coordinator.propose_from_owner_signal(
            RecoveryOwnerSignal {
                reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
                reason_label: "transportAwaitRecoveryKeyframe".to_string(),
                observed_at_ms: now_ms + 1_600.0,
            },
            &shared_stats,
        );
        assert_eq!(second.decision.action, RecoveryAction::RequestDecoderReset);
    }

    #[test]
    fn recent_clean_anchor_keeps_transport_await_recovery_keyframe_from_forcing_hard_escalation() {
        let now_ms = unix_now_ms();
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.transport_recovery_epoch = 12;
        stats.video_anchor_clean_epoch = Some(12);
        stats.video_anchor_clean_observed_at_ms = Some(now_ms - 180.0);
        stats.video_anchor_clean_source_event = Some("chain-clean-keyframe-submitted".to_string());
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 42,
            source_event: "frame-observed".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "healthy".to_string(),
                reason: None,
                observed_at_ms: now_ms - 30.0,
            },
            observed_at_ms: now_ms - 30.0,
        });
        let shared_stats = Mutex::new(stats);
        let mut coordinator = RecoveryCoordinator::new(
            test_escalation_controller(120, 1, 1),
            Instant::now() - Duration::from_secs(3),
            Duration::from_millis(800),
        );

        let _ = coordinator.propose_from_owner_signal(
            RecoveryOwnerSignal {
                reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
                reason_label: "transportAwaitRecoveryKeyframe".to_string(),
                observed_at_ms: now_ms,
            },
            &shared_stats,
        );
        let _ = coordinator.propose_from_owner_signal(
            RecoveryOwnerSignal {
                reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
                reason_label: "transportAwaitRecoveryKeyframe".to_string(),
                observed_at_ms: now_ms + 60.0,
            },
            &shared_stats,
        );
        let third = coordinator.propose_from_owner_signal(
            RecoveryOwnerSignal {
                reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
                reason_label: "transportAwaitRecoveryKeyframe".to_string(),
                observed_at_ms: now_ms + 120.0,
            },
            &shared_stats,
        );
        assert_eq!(third.decision.action, RecoveryAction::CooldownSuppressed);
    }

    #[test]
    fn clean_anchor_acknowledgement_resets_transport_await_streak() {
        let now_ms = unix_now_ms();
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.transport_recovery_epoch = 11;
        let shared_stats = Mutex::new(stats);
        let mut coordinator = RecoveryCoordinator::new(
            test_escalation_controller(120, 1, 1),
            Instant::now() - Duration::from_secs(3),
            Duration::from_millis(800),
        );

        let _ = coordinator.propose_from_owner_signal(
            RecoveryOwnerSignal {
                reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
                reason_label: "transportAwaitRecoveryKeyframe".to_string(),
                observed_at_ms: now_ms,
            },
            &shared_stats,
        );
        let _ = coordinator.propose_from_owner_signal(
            RecoveryOwnerSignal {
                reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
                reason_label: "transportAwaitRecoveryKeyframe".to_string(),
                observed_at_ms: now_ms + 120.0,
            },
            &shared_stats,
        );

        coordinator.acknowledge_clean_anchor();

        let after_clean_anchor = coordinator.propose_from_owner_signal(
            RecoveryOwnerSignal {
                reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
                reason_label: "transportAwaitRecoveryKeyframe".to_string(),
                observed_at_ms: now_ms + 240.0,
            },
            &shared_stats,
        );
        assert_ne!(
            after_clean_anchor.decision.action,
            RecoveryAction::RequestDecoderReset
        );
        assert_ne!(
            after_clean_anchor.decision.action,
            RecoveryAction::RequestReconnectCandidate
        );
    }

    #[test]
    fn transport_await_hard_fallback_timeout_persists_across_recovery_epoch() {
        let now_ms = unix_now_ms();
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.transport_recovery_epoch = 20;
        stats.video_renderer_stalled = Some(true);
        stats.latest_video_present_time_ms = Some(now_ms - 6_000.0);
        let shared_stats = Mutex::new(stats);
        let mut coordinator = RecoveryCoordinator::new(
            test_escalation_controller(120, 1, 1),
            Instant::now() - Duration::from_secs(3),
            Duration::from_millis(800),
        );

        let _ = coordinator.propose_from_owner_signal(
            RecoveryOwnerSignal {
                reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
                reason_label: "transportAwaitRecoveryKeyframe".to_string(),
                observed_at_ms: now_ms,
            },
            &shared_stats,
        );
        RuntimeStatsSink::update_shared(&shared_stats, |stats| {
            stats.transport_recovery_epoch = 21;
        });
        let _ = coordinator.propose_from_owner_signal(
            RecoveryOwnerSignal {
                reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
                reason_label: "transportAwaitRecoveryKeyframe".to_string(),
                observed_at_ms: now_ms + 1_600.0,
            },
            &shared_stats,
        );
        RuntimeStatsSink::update_shared(&shared_stats, |stats| {
            stats.latest_video_escalation_observation =
                Some(crate::XbxEngineVideoEscalationObservation {
                    observation_id: 901,
                    reason: "transportAwaitRecoveryKeyframe".to_string(),
                    action: "requestDecoderReset".to_string(),
                    observed_at_ms: now_ms + 1_650.0,
                });
            stats.latest_video_decoder_reset_time_ms = Some(now_ms + 1_700.0);
        });
        let timeout = coordinator.propose_from_owner_signal(
            RecoveryOwnerSignal {
                reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
                reason_label: "transportAwaitRecoveryKeyframe".to_string(),
                observed_at_ms: now_ms + 2_700.0,
            },
            &shared_stats,
        );
        assert_eq!(
            timeout.decision.action,
            RecoveryAction::RequestReconnectCandidate
        );
        let fallback = RuntimeStatsSink::read_shared(&shared_stats, |stats| {
            (
                stats.recovery_hard_fallback_timer_ms,
                stats.recovery_hard_fallback_trigger_reason.clone(),
                stats.recovery_hard_fallback_timer_reset_reason.clone(),
            )
        })
        .unwrap_or((None, None, None));
        assert!(fallback.0.is_some_and(|timer_ms| timer_ms >= 2_400.0));
        assert_eq!(
            fallback.1.as_deref(),
            Some("transportAwaitRecoveryKeyframeTimeout")
        );
        assert!(fallback.2.is_none());
    }

    #[test]
    fn transport_await_hard_fallback_timer_resets_on_healthy_clean_anchor() {
        let now_ms = unix_now_ms();
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.transport_recovery_epoch = 30;
        stats.video_renderer_stalled = Some(true);
        stats.latest_video_present_time_ms = Some(now_ms - 6_000.0);
        let shared_stats = Mutex::new(stats);
        let mut coordinator = RecoveryCoordinator::new(
            test_escalation_controller(120, 1, 1),
            Instant::now() - Duration::from_secs(3),
            Duration::from_millis(800),
        );
        let _ = coordinator.propose_from_owner_signal(
            RecoveryOwnerSignal {
                reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
                reason_label: "transportAwaitRecoveryKeyframe".to_string(),
                observed_at_ms: now_ms,
            },
            &shared_stats,
        );
        RuntimeStatsSink::update_shared(&shared_stats, |stats| {
            stats.video_anchor_clean_epoch = Some(30);
            stats.video_anchor_clean_observed_at_ms = Some(now_ms + 100.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-keyframe-submitted".to_string());
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 77,
                    source_event: "frame-observed".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        observed_at_ms: now_ms + 100.0,
                    },
                    observed_at_ms: now_ms + 100.0,
                });
            stats.video_renderer_stalled = Some(false);
            stats.latest_video_present_time_ms = Some(now_ms + 100.0);
        });
        let _ = coordinator.propose_from_owner_signal(
            RecoveryOwnerSignal {
                reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
                reason_label: "transportAwaitRecoveryKeyframe".to_string(),
                observed_at_ms: now_ms + 120.0,
            },
            &shared_stats,
        );
        let fallback = RuntimeStatsSink::read_shared(&shared_stats, |stats| {
            (
                stats.recovery_hard_fallback_timer_ms,
                stats.recovery_hard_fallback_trigger_reason.clone(),
                stats.recovery_hard_fallback_timer_reset_reason.clone(),
            )
        })
        .unwrap_or((None, None, None));
        assert!(fallback.0.is_none());
        assert!(fallback.1.is_none());
        assert_eq!(fallback.2.as_deref(), Some("explicitHealthyCleanAnchor"));
    }

    #[test]
    fn transport_await_hard_fallback_requires_decoder_reset_attempt_before_reconnect() {
        let now_ms = unix_now_ms();
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.transport_recovery_epoch = 32;
        stats.video_renderer_stalled = Some(true);
        stats.latest_video_present_time_ms = Some(now_ms - 6_000.0);
        let shared_stats = Mutex::new(stats);
        let mut coordinator = RecoveryCoordinator::new(
            test_escalation_controller(120, 1, 1),
            Instant::now() - Duration::from_secs(3),
            Duration::from_millis(800),
        );

        let _ = coordinator.propose_from_owner_signal(
            RecoveryOwnerSignal {
                reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
                reason_label: "transportAwaitRecoveryKeyframe".to_string(),
                observed_at_ms: now_ms,
            },
            &shared_stats,
        );
        let _ = coordinator.propose_from_owner_signal(
            RecoveryOwnerSignal {
                reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
                reason_label: "transportAwaitRecoveryKeyframe".to_string(),
                observed_at_ms: now_ms + 1_600.0,
            },
            &shared_stats,
        );
        let timeout = coordinator.propose_from_owner_signal(
            RecoveryOwnerSignal {
                reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
                reason_label: "transportAwaitRecoveryKeyframe".to_string(),
                observed_at_ms: now_ms + 2_700.0,
            },
            &shared_stats,
        );
        assert_ne!(
            timeout.decision.action,
            RecoveryAction::RequestReconnectCandidate
        );
        assert_ne!(
            timeout.decision.action,
            RecoveryAction::RequestReconnectCandidate
        );
        assert!(matches!(
            timeout.decision.action,
            RecoveryAction::RequestDecoderReset | RecoveryAction::CooldownSuppressed
        ));
    }

    #[test]
    fn transport_await_hard_fallback_uses_connected_ingress_when_decoder_reset_path_exhausted() {
        let now_ms = unix_now_ms();
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.session_target_type = Some(XbxEngineTargetTypeDto::Cloud);
        stats.transport_recovery_epoch = 41;
        stats.transport_state = XbxEngineTransportStateDto::Connected;
        stats.video_renderer_stalled = Some(true);
        stats.latest_video_present_time_ms = Some(now_ms - 9_000.0);
        stats.host_no_pending_pressure_level = Some("critical".to_string());
        stats.host_no_pending_streak = 320;
        stats.inbound_primary_video_bytes_total = 12_000;
        stats.latest_video_track_status = Some(XbxEngineVideoTrackStatus {
            state: "remoteTrackAttached".to_string(),
            video_width: Some(2560),
            video_height: Some(1440),
            mime_type: Some("video/H264".to_string()),
            transport_state: XbxEngineTransportStateDto::Connected,
            video_bytes_total: 42_000,
            video_packet_count_total: 120,
            audio_bytes_total: 2_100,
            observed_at_ms: now_ms,
        });
        let shared_stats = Mutex::new(stats);
        let mut coordinator = RecoveryCoordinator::new(
            test_escalation_controller(120, 1, 1),
            Instant::now() - Duration::from_secs(3),
            Duration::from_millis(800),
        );

        // 先把 decoder reset 预算耗尽。
        let _ = coordinator.propose_from_owner_signal(
            RecoveryOwnerSignal {
                reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
                reason_label: "transportAwaitRecoveryKeyframe".to_string(),
                observed_at_ms: now_ms,
            },
            &shared_stats,
        );
        let _ = coordinator.propose_from_owner_signal(
            RecoveryOwnerSignal {
                reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
                reason_label: "transportAwaitRecoveryKeyframe".to_string(),
                observed_at_ms: now_ms + 200.0,
            },
            &shared_stats,
        );

        // 用一次显式 healthy clean anchor 把 hard fallback 计时窗口重置，
        // 让后续 timeout 窗口内“不存在 decoder reset 尝试”。
        RuntimeStatsSink::update_shared(&shared_stats, |stats| {
            stats.video_anchor_clean_epoch = Some(41);
            stats.video_anchor_clean_observed_at_ms = Some(now_ms + 260.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-keyframe-submitted".to_string());
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1201,
                    source_event: "frame-observed".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        observed_at_ms: now_ms + 260.0,
                    },
                    observed_at_ms: now_ms + 260.0,
                });
            stats.video_renderer_stalled = Some(false);
            stats.latest_video_present_time_ms = Some(now_ms + 260.0);
        });
        let _ = coordinator.propose_from_owner_signal(
            RecoveryOwnerSignal {
                reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
                reason_label: "transportAwaitRecoveryKeyframe".to_string(),
                observed_at_ms: now_ms + 300.0,
            },
            &shared_stats,
        );

        // 回到 Connected + ingress 持续，但显示链路仍坏窗。
        RuntimeStatsSink::update_shared(&shared_stats, |stats| {
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1202,
                    source_event: "frame-await-recovery-keyframe".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("streamThinStall".to_string()),
                        observed_at_ms: now_ms + 6_900.0,
                    },
                    observed_at_ms: now_ms + 6_900.0,
                });
            stats.video_renderer_stalled = Some(true);
            stats.latest_video_present_time_ms = Some(now_ms - 10_000.0);
            stats.latest_video_track_status = Some(XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(2560),
                video_height: Some(1440),
                mime_type: Some("video/H264".to_string()),
                transport_state: XbxEngineTransportStateDto::Connected,
                video_bytes_total: 88_000,
                video_packet_count_total: 300,
                audio_bytes_total: 4_200,
                observed_at_ms: now_ms + 6_900.0,
            });
            stats.inbound_primary_video_bytes_total = 48_000;
        });
        let timeout = coordinator.propose_from_owner_signal(
            RecoveryOwnerSignal {
                reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
                reason_label: "transportAwaitRecoveryKeyframe".to_string(),
                observed_at_ms: now_ms + 7_000.0,
            },
            &shared_stats,
        );
        assert_eq!(
            timeout.decision.action,
            RecoveryAction::RequestReconnectCandidate
        );
    }

    #[test]
    fn transport_await_hard_fallback_does_not_reset_on_non_await_reason() {
        let now_ms = unix_now_ms();
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.transport_recovery_epoch = 31;
        stats.video_renderer_stalled = Some(true);
        stats.latest_video_present_time_ms = Some(now_ms - 6_000.0);
        let shared_stats = Mutex::new(stats);
        let mut coordinator = RecoveryCoordinator::new(
            test_escalation_controller(120, 1, 1),
            Instant::now() - Duration::from_secs(3),
            Duration::from_millis(800),
        );

        let _ = coordinator.propose_from_owner_signal(
            RecoveryOwnerSignal {
                reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
                reason_label: "transportAwaitRecoveryKeyframe".to_string(),
                observed_at_ms: now_ms,
            },
            &shared_stats,
        );
        let _ = coordinator.propose_from_owner_signal(
            RecoveryOwnerSignal {
                reason: VideoEscalationReason::AdapterThinStream,
                reason_label: "adapterThinStream".to_string(),
                observed_at_ms: now_ms + 1_200.0,
            },
            &shared_stats,
        );
        let _ = coordinator.propose_from_owner_signal(
            RecoveryOwnerSignal {
                reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
                reason_label: "transportAwaitRecoveryKeyframe".to_string(),
                observed_at_ms: now_ms + 2_800.0,
            },
            &shared_stats,
        );
        let fallback = RuntimeStatsSink::read_shared(&shared_stats, |stats| {
            (
                stats.recovery_hard_fallback_timer_ms,
                stats.recovery_hard_fallback_timer_reset_reason.clone(),
            )
        })
        .unwrap_or((None, None));
        assert!(fallback.0.is_some_and(|timer_ms| timer_ms >= 2_700.0));
        assert!(fallback.1.is_none());
    }
}
