use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::runtime_stats_sink::expire_latest_keyframe_request_episode_if_unsent;
use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::recovery::decoder_backend_failure::{
    resolve_decoder_backend_failure_recovery, DecoderBackendFailureResolution,
};
use crate::transport::rtc::recovery::escalation::{
    KeyframeTransportFeedback, RecoveryAction, RecoveryActionBudgetState,
    VideoEscalationController, VideoEscalationDecision, VideoEscalationReason,
};
use crate::transport::rtc::recovery::hard_stall::resolve_persistent_stall_recovery;
use crate::transport::rtc::recovery::nack_outcome::{
    resolve_recent_nack_outcome, CloudStartupExpiredDeadlineBudget, RecentNackOutcomeResolution,
};
use crate::transport::rtc::recovery::policy::RecoveryScenarioProfile;
use crate::transport::rtc::recovery::repeat_suppression::resolve_recent_repeat_suppression;
use crate::transport::rtc::recovery::runtime_state::{
    has_fresh_media_output, recovery_stage_label_from_stats, resolve_recovery_profile, unix_now_ms,
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
    XbxEngineAnchorCandidateFailureReason, XbxEngineAnchorCandidateLedger,
    XbxEngineAnchorCandidateState, XbxEngineMediaRuntimeStats, XbxEngineVideoTimelineObservation,
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
    Local,
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
    await_recovery_hard_fallback_epoch: Option<u64>,
    last_synced_decoder_reset_observation_id: Option<u64>,
    last_synced_reconnect_observation_id: Option<u64>,
}

const TRANSPORT_AWAIT_RECOVERY_KEYFRAME_STREAK_WINDOW_MS: f64 = 3_500.0;
const TRANSPORT_AWAIT_CONNECTED_INGRESS_EVIDENCE_MAX_AGE_MS: f64 = 4_000.0;
const CLEAN_ANCHOR_EPOCH_GRACE_MAX_DELTA: u64 = 1;
const CLEAN_ANCHOR_EPOCH_GRACE_WINDOW_MS: f64 = 1_500.0;
const UNSENT_KEYFRAME_REQUEST_GRACE_MS: f64 = 220.0;

impl RecoveryCoordinator {
    pub(crate) fn transport_await_has_hard_recovery_evidence(
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        recovery_epoch: u64,
        now_ms: f64,
    ) -> bool {
        Self::has_transport_await_stage_escalation_failure_evidence(
            runtime_stats,
            recovery_epoch,
            now_ms,
        )
    }

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
            await_recovery_hard_fallback_epoch: None,
            last_synced_decoder_reset_observation_id: None,
            last_synced_reconnect_observation_id: None,
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
        self.sync_reconnect_transport_success(runtime_stats, recovery_epoch, signal.observed_at_ms);
        self.sync_decoder_reset_transport_success(
            runtime_stats,
            recovery_epoch,
            signal.observed_at_ms,
        );
        self.sync_keyframe_transport_feedback(runtime_stats, signal.observed_at_ms);
        let budget_before = self.escalation_controller.budget_state();
        self.track_await_recovery_keyframe_streak(
            signal.reason,
            runtime_stats,
            recovery_epoch,
            signal.observed_at_ms,
        );
        if signal.reason != VideoEscalationReason::TransportAwaitRecoveryKeyframe {
            self.clear_transport_await_hard_fallback("nonAwaitReason");
        }
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
        if matches!(
            signal.reason,
            VideoEscalationReason::AdapterIdleTimeout
                | VideoEscalationReason::DisplaySupplyCritical
        ) && RuntimeStatsSink::read_shared(runtime_stats, |stats| {
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

    fn sync_keyframe_transport_feedback(
        &mut self,
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        observed_at_ms: f64,
    ) {
        let (feedback, expire_unsent_episode) =
            RuntimeStatsSink::read_shared(runtime_stats, |stats| {
                let Some(episode) = stats.latest_keyframe_request_episode.as_ref() else {
                    return (KeyframeTransportFeedback::None, false);
                };
                let pending_verdict =
                    matches!(episode.response_verdict.as_deref(), None | Some("pending"));
                if !pending_verdict {
                    return (KeyframeTransportFeedback::Terminal, false);
                }
                let within_window = episode
                    .deadline_at_ms
                    .map(|deadline_at_ms| observed_at_ms <= deadline_at_ms)
                    .unwrap_or(true);
                if !within_window {
                    return (KeyframeTransportFeedback::Terminal, false);
                }
                if episode.sent_at_ms.is_some()
                    && matches!(episode.status.as_str(), "requested" | "sent")
                {
                    (KeyframeTransportFeedback::SentPending, false)
                } else if episode.sent_at_ms.is_none() && episode.status == "requested" {
                    let unsent_age_ms = (observed_at_ms - episode.requested_at_ms).max(0.0);
                    if unsent_age_ms <= UNSENT_KEYFRAME_REQUEST_GRACE_MS {
                        (KeyframeTransportFeedback::UnsentPending, false)
                    } else {
                        (KeyframeTransportFeedback::Terminal, true)
                    }
                } else {
                    (KeyframeTransportFeedback::Terminal, false)
                }
            })
            .unwrap_or((KeyframeTransportFeedback::None, false));
        if expire_unsent_episode {
            if let Ok(mut stats) = runtime_stats.lock() {
                expire_latest_keyframe_request_episode_if_unsent(&mut stats, observed_at_ms);
            }
        }
        self.escalation_controller
            .reconcile_keyframe_transport_feedback(feedback);
    }

    fn sync_decoder_reset_transport_success(
        &mut self,
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        recovery_epoch: u64,
        observed_at_ms: f64,
    ) {
        let latest_decoder_reset = RuntimeStatsSink::read_shared(runtime_stats, |stats| {
            let observation = stats.latest_video_escalation_observation.as_ref()?;
            if stats.transport_recovery_epoch_at_last_escalation != recovery_epoch {
                return None;
            }
            if observation.observed_at_ms > observed_at_ms {
                return None;
            }
            if !matches!(
                observation.action.as_str(),
                "requestDecoderReset"
                    | "requestKeyframe+decoderReset"
                    | "requestKeyframe+decoderReset(startupLowQualityRetry)"
            ) {
                return None;
            }
            Some(observation.observation_id)
        })
        .flatten();
        let Some(observation_id) = latest_decoder_reset else {
            return;
        };
        if self.last_synced_decoder_reset_observation_id == Some(observation_id) {
            return;
        }
        self.escalation_controller.register_decoder_reset_started();
        self.last_synced_decoder_reset_observation_id = Some(observation_id);
    }

    fn sync_reconnect_transport_success(
        &mut self,
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        recovery_epoch: u64,
        observed_at_ms: f64,
    ) {
        let latest_reconnect = RuntimeStatsSink::read_shared(runtime_stats, |stats| {
            let observation = stats.latest_video_escalation_observation.as_ref()?;
            if stats.transport_recovery_epoch_at_last_escalation != recovery_epoch {
                return None;
            }
            if observation.observed_at_ms > observed_at_ms {
                return None;
            }
            if observation.action != "requestReconnectCandidate" {
                return None;
            }
            Some(observation.observation_id)
        })
        .flatten();
        let Some(observation_id) = latest_reconnect else {
            return;
        };
        if self.last_synced_reconnect_observation_id == Some(observation_id) {
            return;
        }
        self.escalation_controller.register_reconnect_started();
        self.last_synced_reconnect_observation_id = Some(observation_id);
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
        // clean anchor 代表当前 recovery epoch 已经拿到明确健康证据；
        // lingering hard-fallback 计时不能跨过这类恢复成功信号继续累积。
        self.clear_transport_await_hard_fallback("cleanAnchorAcknowledged");
    }

    pub fn acknowledge_stable_recovery(&mut self) {
        self.acknowledge_clean_anchor();
        self.escalation_controller.acknowledge_stable_recovery();
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
        let transport_await_hard_evidence = reason
            != VideoEscalationReason::TransportAwaitRecoveryKeyframe
            || Self::has_transport_await_stage_escalation_failure_evidence(
                runtime_stats,
                recovery_epoch,
                observed_at_ms,
            );
        let allow_transport_await_stage_escalation = transport_await_hard_evidence;
        let allow_wait_keyframe_stage_escalation = reason != VideoEscalationReason::WaitKeyframe
            || Self::has_wait_keyframe_stage_escalation_failure_evidence(
                runtime_stats,
                recovery_epoch,
                observed_at_ms,
            );
        let allow_reconfigure_stage_escalation = reason != VideoEscalationReason::Reconfigure
            || Self::has_reconfigure_stage_escalation_failure_evidence(
                runtime_stats,
                recovery_epoch,
                observed_at_ms,
            );
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
                allow_transport_await_stage_escalation,
                allow_wait_keyframe_stage_escalation,
                allow_reconfigure_stage_escalation,
            )
        };
        if reason == VideoEscalationReason::TransportAwaitRecoveryKeyframe
            && !transport_await_hard_evidence
            && Self::transport_await_startup_bootstrap_probation_active(
                runtime_stats,
                observed_at_ms,
            )
            && Self::transport_await_action_leaves_local_probe_domain(escalation_decision.action)
        {
            escalation_decision = self
                .escalation_controller
                .suppressed(RecoveryAction::WaitForBurst);
        }
        if reason == VideoEscalationReason::TransportAwaitRecoveryKeyframe
            && !transport_await_hard_evidence
            && Self::has_recent_transport_await_keyframe_attempt(
                runtime_stats,
                recovery_epoch,
                observed_at_ms,
            )
            && !Self::is_non_executing_recovery_action(escalation_decision.action)
        {
            escalation_decision = self
                .escalation_controller
                .suppressed(RecoveryAction::WaitForBurst);
        }
        if reason == VideoEscalationReason::TransportAwaitRecoveryKeyframe
            && Self::transport_await_soft_reentry_is_recent_and_healthy(
                runtime_stats,
                recovery_epoch,
                observed_at_ms,
            )
            && !Self::is_non_executing_recovery_action(escalation_decision.action)
        {
            escalation_decision = self
                .escalation_controller
                .suppressed(RecoveryAction::CooldownSuppressed);
        }
        if reason == VideoEscalationReason::TransportAwaitRecoveryKeyframe
            && matches!(
                escalation_decision.action,
                RecoveryAction::CoalescedKeyframeInFlight | RecoveryAction::CooldownSuppressed
            )
            && Self::has_keyframe_stage_escalation_failure_evidence(
                runtime_stats,
                recovery_epoch,
                observed_at_ms,
            )
        {
            let decoder_reset_in_flight = self
                .await_recovery_keyframe_streak_started_at_ms
                .is_some_and(|started_at_ms| {
                    Self::has_transport_await_decoder_reset_attempt_since(
                        runtime_stats,
                        started_at_ms,
                    )
                });
            escalation_decision = self.escalation_controller.suppressed(
                if decoder_reset_in_flight {
                    RecoveryAction::CoalescedDecoderResetInFlight
                } else {
                    RecoveryAction::RequestDecoderReset
                },
            );
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
        if self
            .await_recovery_hard_fallback_epoch
            .is_some_and(|epoch| epoch != recovery_epoch)
        {
            self.reset_transport_await_hard_fallback(runtime_stats, "recoveryEpochAdvanced", true);
        }
        let explicit_healthy_with_clean_anchor =
            Self::transport_await_soft_reentry_is_recent_and_healthy(
                runtime_stats,
                recovery_epoch,
                observed_at_ms,
            );
        if explicit_healthy_with_clean_anchor {
            self.reset_transport_await_hard_fallback(
                runtime_stats,
                "explicitHealthyCleanAnchor",
                true,
            );
            return None;
        }
        let has_evidence = Self::has_transport_await_hard_fallback_evidence(
            runtime_stats,
            observed_at_ms,
            profile,
        );
        if !has_evidence {
            self.reset_transport_await_hard_fallback(runtime_stats, "stallEvidenceCleared", true);
            return None;
        }
        let started_at_ms = *self
            .await_recovery_hard_fallback_started_at_ms
            .get_or_insert(observed_at_ms);
        self.await_recovery_hard_fallback_epoch = Some(recovery_epoch);
        let timer_ms = (observed_at_ms - started_at_ms).max(0.0);
        RuntimeStatsSink::update_shared(runtime_stats, |stats| {
            stats.recovery_hard_fallback_timer_ms = Some(timer_ms);
            stats.recovery_hard_fallback_timer_reset_reason = None;
        });
        if timer_ms < profile.hard_fallback_transport_await_timeout_ms as f64 {
            return None;
        }
        let recovery_stage = RuntimeStatsSink::read_shared(runtime_stats, |stats| {
            recovery_stage_label_from_stats(stats)
        })
        .unwrap_or("steady");
        let connected_ingress_evidence =
            Self::has_transport_await_connected_ingress_evidence(runtime_stats, observed_at_ms);
        if connected_ingress_evidence {
            return Some(
                self.escalation_controller
                    .suppressed(RecoveryAction::CooldownSuppressed),
            );
        }
        if !Self::has_transport_await_decoder_reset_attempt_since(runtime_stats, started_at_ms) {
            // hard fallback 只接受“持续坏窗 + 已经失去本地恢复进展”的升级。
            // 如果还没有 decoder reset 尝试，除 reconnecting 以外都先留在本地恢复链。
            if recovery_stage != "reconnecting" {
                return None;
            }
        }
        let decision = self.escalation_controller.on_reason_with_epoch_policy(
            VideoEscalationReason::LifecycleRecovering,
            recovery_epoch,
            true,
            true,
            true,
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
            || !matches!(
                current_action,
                RecoveryAction::CooldownSuppressed | RecoveryAction::CoalescedKeyframeInFlight
            )
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
        if !Self::has_transport_await_stage_escalation_failure_evidence(
            runtime_stats,
            recovery_epoch,
            observed_at_ms,
        ) {
            return None;
        }
        if Self::has_keyframe_stage_escalation_failure_evidence(
            runtime_stats,
            recovery_epoch,
            observed_at_ms,
        ) {
            if self
                .await_recovery_keyframe_streak_started_at_ms
                .is_some_and(|started_at_ms| {
                    Self::has_transport_await_decoder_reset_attempt_since(
                        runtime_stats,
                        started_at_ms,
                    )
                })
            {
                return Some(
                    self.escalation_controller
                        .suppressed(RecoveryAction::CoalescedDecoderResetInFlight),
                );
            }
            return Some(
                self.escalation_controller
                    .suppressed(RecoveryAction::RequestDecoderReset),
            );
        }
        let recovery_stage = RuntimeStatsSink::read_shared(runtime_stats, |stats| {
            recovery_stage_label_from_stats(stats)
        })
        .unwrap_or("steady");
        let has_stall_evidence = Self::has_transport_await_hard_fallback_evidence(
            runtime_stats,
            observed_at_ms,
            profile,
        );
        let has_stage_upgrade_pressure =
            Self::has_transport_await_stage_upgrade_pressure(runtime_stats, observed_at_ms);
        if has_stall_evidence
            && self.await_recovery_keyframe_streak >= 2
            && self
                .await_recovery_keyframe_streak_started_at_ms
                .is_some_and(|started_at_ms| {
                    (observed_at_ms - started_at_ms).max(0.0)
                        >= TRANSPORT_AWAIT_CONNECTED_BAD_WINDOW_STAGE_MIN_MS
                })
        {
            if self
                .await_recovery_keyframe_streak_started_at_ms
                .is_some_and(|started_at_ms| {
                    Self::has_transport_await_decoder_reset_attempt_since(
                        runtime_stats,
                        started_at_ms,
                    )
                })
            {
                return Some(
                    self.escalation_controller
                        .suppressed(RecoveryAction::CoalescedDecoderResetInFlight),
                );
            }
            return Some(
                self.escalation_controller
                    .suppressed(RecoveryAction::RequestDecoderReset),
            );
        }
        let streak_threshold = match (
            recovery_stage,
            has_stall_evidence,
            has_stage_upgrade_pressure,
        ) {
            (_, true, true) => 1,
            ("rebuilding-supply", true, false) => 1,
            ("priming", true, false) => 3,
            (_, true, false) => 2,
            ("priming", false, false) => 4,
            _ => 3,
        };
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
                .latest_video_host_present_time_ms
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

    fn has_transport_await_stage_upgrade_pressure(
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        now_ms: f64,
    ) -> bool {
        RuntimeStatsSink::read_shared(runtime_stats, |stats| {
            let thin_stall_timeline = stats
                .latest_video_timeline_observation
                .as_ref()
                .is_some_and(|timeline| {
                    (now_ms - timeline.observed_at_ms).max(0.0) <= 1_500.0
                        && (matches!(timeline.chain.reason.as_deref(), Some("streamThinStall"))
                            || matches!(
                                timeline.source_event.as_str(),
                                "timeout-stream-thin-stall"
                            ))
                });
            let expired_deadline =
                stats
                    .latest_video_nack_observation
                    .as_ref()
                    .is_some_and(|nack| {
                        (now_ms - nack.observed_at_ms).max(0.0) <= 1_500.0
                            && nack.action == "expiredDeadline"
                    });
            let unsent_keyframe_request = stats
                .latest_keyframe_request_episode
                .as_ref()
                .is_some_and(|episode| {
                    matches!(episode.response_verdict.as_deref(), None | Some("pending"))
                        && episode.status == "requested"
                        && episode.sent_at_ms.is_none()
                        && (now_ms - episode.requested_at_ms).max(0.0) <= 1_500.0
                });
            let recent_rtcp_failure = stats
                .latest_video_rtcp_send_failure_time_ms
                .is_some_and(|failed_at_ms| (now_ms - failed_at_ms).max(0.0) <= 1_500.0)
                && matches!(
                    stats.latest_video_rtcp_send_failure_reason.as_deref(),
                    Some(
                        "xbxEngineRtcPeerConnectionUnavailable"
                            | "xbxEngineRtcVideoRtcpFeedbackTargetUnavailable"
                            | "xbxEngineRtcReceiverLookupFailedForVideoRtcp"
                    )
                );
            thin_stall_timeline
                || expired_deadline
                || unsent_keyframe_request
                || recent_rtcp_failure
        })
        .unwrap_or(false)
    }

    fn has_transport_await_stage_escalation_failure_evidence(
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        recovery_epoch: u64,
        now_ms: f64,
    ) -> bool {
        const TRANSPORT_AWAIT_HARD_EVIDENCE_FRESH_MS: f64 = 1_500.0;
        RuntimeStatsSink::read_shared(runtime_stats, |stats| {
            if Self::transport_await_startup_bootstrap_probation_active_from_stats(stats, now_ms) {
                return false;
            }
            let unresolved_transport_await = Self::has_unresolved_transport_await_issue(stats);
            if !unresolved_transport_await {
                return false;
            }
            let objective_media_unhealthy = !Self::transport_await_objective_media_healthy(
                stats, now_ms,
            );
            let lacks_recent_clean_anchor = !Self::has_recent_clean_anchor_evidence(
                stats.video_anchor_clean_epoch,
                stats.video_anchor_clean_observed_at_ms,
                stats.video_anchor_clean_source_event.as_deref(),
                stats.latest_anchor_candidate_ledger.as_ref(),
                recovery_epoch,
                now_ms,
            );
            let recent_h264_requires_keyframe = stats
                .latest_h264_inspection_observation
                .as_ref()
                .is_some_and(|inspection| {
                    (now_ms - inspection.observed_at_ms).max(0.0)
                        <= TRANSPORT_AWAIT_HARD_EVIDENCE_FRESH_MS
                        && (!inspection.delta_continuation_ready
                            || !inspection.committed_sps_present
                            || !inspection.committed_pps_present)
                });
            let recent_hard_anchor_failure = stats
                .latest_anchor_candidate_ledger
                .as_ref()
                .is_some_and(|candidate| {
                    candidate.recovery_epoch == recovery_epoch
                        && (now_ms - candidate.observed_at_ms).max(0.0)
                            <= TRANSPORT_AWAIT_HARD_EVIDENCE_FRESH_MS
                        && matches!(
                            candidate.failure_reason,
                            Some(
                                XbxEngineAnchorCandidateFailureReason::ChainBrokenReferenceUnrecoverable
                                    | XbxEngineAnchorCandidateFailureReason::GapExpiredDeadline
                            )
                        )
                });
            let previous_keyframe_failed = stats
                .latest_keyframe_request_episode
                .as_ref()
                .is_some_and(|episode| {
                    episode.request_reason.as_deref() == Some("transportAwaitRecoveryKeyframe")
                        && matches!(
                            episode.response_verdict.as_deref(),
                            Some("missed" | "late")
                        )
                });
            previous_keyframe_failed
                || recent_h264_requires_keyframe
                || (recent_hard_anchor_failure
                    && (objective_media_unhealthy || lacks_recent_clean_anchor))
                || (objective_media_unhealthy
                    && lacks_recent_clean_anchor
                    && recent_h264_requires_keyframe)
        })
        .unwrap_or(false)
    }

    fn transport_await_startup_bootstrap_probation_active(
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        now_ms: f64,
    ) -> bool {
        RuntimeStatsSink::read_shared(runtime_stats, |stats| {
            Self::transport_await_startup_bootstrap_probation_active_from_stats(stats, now_ms)
        })
        .unwrap_or(false)
    }

    fn transport_await_startup_bootstrap_probation_active_from_stats(
        stats: &XbxEngineMediaRuntimeStats,
        now_ms: f64,
    ) -> bool {
        const STARTUP_BOOTSTRAP_PROBATION_FRESH_MS: f64 = 1_500.0;
        let before_first_present = stats.host_display_tick_epoch > 0
            && stats.video_present_epoch == 0
            && stats.video_present_submit_count_total == 0;
        if !before_first_present {
            return false;
        }
        let track_attached_with_video = stats
            .latest_video_track_status
            .as_ref()
            .is_some_and(|track| track.state == "remoteTrackAttached" && track.video_bytes_total > 0);
        if !track_attached_with_video {
            return false;
        }
        let timeline_awaits_keyframe =
            stats
                .latest_video_timeline_observation
                .as_ref()
                .is_some_and(|timeline| {
                    (now_ms - timeline.observed_at_ms).max(0.0)
                        <= STARTUP_BOOTSTRAP_PROBATION_FRESH_MS
                        && matches!(
                            timeline.source_event.as_str(),
                            "frame-await-recovery-keyframe"
                                | "frame-inspection-rejected-await-keyframe"
                        )
                });
        if !timeline_awaits_keyframe {
            return false;
        }
        stats
            .latest_h264_inspection_observation
            .as_ref()
            .is_some_and(|inspection| {
                (now_ms - inspection.observed_at_ms).max(0.0)
                    <= STARTUP_BOOTSTRAP_PROBATION_FRESH_MS
                    && !inspection.bootstrap_ready
                    && matches!(
                        inspection.bootstrap_reject_reason.as_deref(),
                        Some(
                            "bootstrapMissingSps"
                                | "bootstrapMissingPps"
                                | "inspectionRejectInvalidSliceHeader"
                        )
                    )
            })
    }

    fn has_recent_transport_await_keyframe_attempt(
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        recovery_epoch: u64,
        now_ms: f64,
    ) -> bool {
        const TRANSPORT_AWAIT_ATTEMPT_FRESH_MS: f64 = 4_500.0;
        RuntimeStatsSink::read_shared(runtime_stats, |stats| {
            let recent_episode =
                stats
                    .latest_keyframe_request_episode
                    .as_ref()
                    .is_some_and(|episode| {
                        episode.request_reason.as_deref() == Some("transportAwaitRecoveryKeyframe")
                            && (now_ms - episode.requested_at_ms).max(0.0)
                                <= TRANSPORT_AWAIT_ATTEMPT_FRESH_MS
                    });
            let recent_escalation = stats
                .latest_video_escalation_observation
                .as_ref()
                .is_some_and(|observation| {
                    stats.transport_recovery_epoch_at_last_escalation == recovery_epoch
                        && observation.reason == "transportAwaitRecoveryKeyframe"
                        && observation.action == "requestKeyframe"
                        && (now_ms - observation.observed_at_ms).max(0.0)
                            <= TRANSPORT_AWAIT_ATTEMPT_FRESH_MS
                });
            recent_episode || recent_escalation
        })
        .unwrap_or(false)
    }

    fn transport_await_action_leaves_local_probe_domain(action: RecoveryAction) -> bool {
        !matches!(
            action,
            RecoveryAction::WaitForBurst
                | RecoveryAction::CooldownSuppressed
                | RecoveryAction::CoalescedKeyframeInFlight
                | RecoveryAction::StartupGraceSuppressed
                | RecoveryAction::RequestKeyframe
        )
    }

    fn has_wait_keyframe_stage_escalation_failure_evidence(
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        recovery_epoch: u64,
        now_ms: f64,
    ) -> bool {
        Self::has_keyframe_stage_escalation_failure_evidence(runtime_stats, recovery_epoch, now_ms)
    }

    fn has_reconfigure_stage_escalation_failure_evidence(
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        recovery_epoch: u64,
        now_ms: f64,
    ) -> bool {
        const RECONFIGURE_EVIDENCE_FRESH_MS: f64 = 1_500.0;
        RuntimeStatsSink::read_shared(runtime_stats, |stats| {
            let recent_reconfigure_signal = stats
                .latest_video_escalation_observation
                .as_ref()
                .is_some_and(|observation| {
                    observation.reason == "reconfigure"
                        && (now_ms - observation.observed_at_ms).max(0.0)
                            <= RECONFIGURE_EVIDENCE_FRESH_MS
                });
            if !recent_reconfigure_signal {
                return false;
            }
            let pipeline_stalled = stats.video_renderer_stalled.unwrap_or(false)
                || stats.video_decoder_stalled.unwrap_or(false);
            let no_fresh_media_output = !has_fresh_media_output(stats, now_ms);
            let unresolved_transport_await = Self::has_unresolved_transport_await_issue(stats);
            let lacks_recent_clean_anchor = !Self::has_recent_clean_anchor_evidence(
                stats.video_anchor_clean_epoch,
                stats.video_anchor_clean_observed_at_ms,
                stats.video_anchor_clean_source_event.as_deref(),
                stats.latest_anchor_candidate_ledger.as_ref(),
                recovery_epoch,
                now_ms,
            );
            // reconfigure 仅在“重配后仍无有效恢复迹象”时允许升级昂贵恢复。
            (pipeline_stalled || no_fresh_media_output)
                && (unresolved_transport_await || lacks_recent_clean_anchor)
        })
        .unwrap_or(false)
    }

    fn has_keyframe_stage_escalation_failure_evidence(
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        recovery_epoch: u64,
        now_ms: f64,
    ) -> bool {
        const TRANSPORT_AWAIT_KEYFRAME_UNUSABLE_GRACE_MS: f64 = 220.0;
        const TRANSPORT_AWAIT_FAILURE_EVIDENCE_FRESH_MS: f64 = 1_500.0;
        RuntimeStatsSink::read_shared(runtime_stats, |stats| {
            let rejected_anchor_without_usable_clean_anchor = stats
                .latest_anchor_candidate_ledger
                .as_ref()
                .is_some_and(|candidate| {
                    candidate.recovery_epoch == recovery_epoch
                        && (now_ms - candidate.observed_at_ms).max(0.0)
                            <= TRANSPORT_AWAIT_FAILURE_EVIDENCE_FRESH_MS
                        && candidate.state == XbxEngineAnchorCandidateState::Rejected
                        && matches!(
                            candidate.failure_reason,
                            Some(
                                XbxEngineAnchorCandidateFailureReason::AwaitingRecoveryKeyframe
                                    | XbxEngineAnchorCandidateFailureReason::InspectionRejectedMissingSps
                                    | XbxEngineAnchorCandidateFailureReason::InspectionRejectedMissingPps
                                    | XbxEngineAnchorCandidateFailureReason::InspectionRejectedInvalidSliceHeader
                                    | XbxEngineAnchorCandidateFailureReason::ChainBrokenReferenceUnrecoverable
                                    | XbxEngineAnchorCandidateFailureReason::GapExpiredDeadline
                            )
                        )
                });
            if rejected_anchor_without_usable_clean_anchor {
                return true;
            }
            let Some(episode) = stats.latest_keyframe_request_episode.as_ref() else {
                return false;
            };
            if episode.request_reason.as_deref() != Some("transportAwaitRecoveryKeyframe") {
                return false;
            }
            if matches!(episode.response_verdict.as_deref(), Some("missed" | "late"))
                || matches!(episode.status.as_str(), "missed")
            {
                return true;
            }
            let unresolved_transport_await = Self::has_unresolved_transport_await_issue(stats);
            let lacks_recent_clean_anchor = !Self::has_recent_clean_anchor_evidence(
                stats.video_anchor_clean_epoch,
                stats.video_anchor_clean_observed_at_ms,
                stats.video_anchor_clean_source_event.as_deref(),
                stats.latest_anchor_candidate_ledger.as_ref(),
                recovery_epoch,
                now_ms,
            );
            let decoded_without_usable_anchor = episode.status == "decoded"
                && episode.first_keyframe_decoded_at_ms.is_some_and(|decoded_at_ms| {
                    (now_ms - decoded_at_ms).max(0.0) >= TRANSPORT_AWAIT_KEYFRAME_UNUSABLE_GRACE_MS
                })
                && unresolved_transport_await
                && lacks_recent_clean_anchor;
            decoded_without_usable_anchor
        })
        .unwrap_or(false)
    }

    fn clear_transport_await_hard_fallback(&mut self, reset_reason: &str) {
        self.await_recovery_hard_fallback_started_at_ms = None;
        self.await_recovery_hard_fallback_epoch = None;
        let _ = reset_reason;
    }

    fn reset_transport_await_hard_fallback(
        &mut self,
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        reset_reason: &str,
        clear_internal_state: bool,
    ) {
        if clear_internal_state {
            self.clear_transport_await_hard_fallback(reset_reason);
        }
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
            // 仅“还在收包”不足以证明局部恢复仍有效；必须仍有新输出推进，
            // 否则会把 connected-but-unrecoverable 长时间压在 cooldownSuppressed。
            let output_still_progressing = has_fresh_media_output(stats, now_ms);
            track_attached_and_recent
                && stats.inbound_primary_video_bytes_total > 0
                && output_still_progressing
        })
        .unwrap_or(false)
    }

    fn transport_await_soft_reentry_is_recent_and_healthy(
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        recovery_epoch: u64,
        now_ms: f64,
    ) -> bool {
        RuntimeStatsSink::read_shared(runtime_stats, |stats| {
            // 与 session policy 的 `healthy_media_baseline` 对齐：时间线 healthy + clean anchor
            // 单独不足以证明端到端已恢复；否则会在 decode/呈现仍卡住时错误压制 PLI/decoder reset。
            Self::transport_await_objective_media_healthy(stats, now_ms)
                && !Self::has_unresolved_transport_await_issue(stats)
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

    fn has_unresolved_transport_await_issue(stats: &XbxEngineMediaRuntimeStats) -> bool {
        const TRANSPORT_AWAIT_UNRESOLVED_REASONS: [&str; 3] = [
            "awaitingRecoveryKeyframe",
            "awaitRecoveryKeyframe",
            "referenceChainUnrecoverable",
        ];
        let Some(timeline) = stats.latest_video_timeline_observation.as_ref() else {
            return false;
        };
        if timeline
            .chain
            .reason
            .as_deref()
            .is_some_and(|reason| TRANSPORT_AWAIT_UNRESOLVED_REASONS.contains(&reason))
        {
            return true;
        }
        if timeline
            .frame
            .as_ref()
            .and_then(|frame| frame.close_reason.as_deref())
            .is_some_and(|reason| TRANSPORT_AWAIT_UNRESOLVED_REASONS.contains(&reason))
        {
            return true;
        }
        timeline.gap.as_ref().is_some_and(|gap| {
            !matches!(gap.state.as_str(), "resolved" | "expired")
                && timeline
                    .chain
                    .reason
                    .as_deref()
                    .is_some_and(|reason| TRANSPORT_AWAIT_UNRESOLVED_REASONS.contains(&reason))
        })
    }

    /// transport-await 软回退判定用的客观媒体健康：与 `should_absorb_stale_transport_await_replay` 一致。
    fn transport_await_objective_media_healthy(
        stats: &XbxEngineMediaRuntimeStats,
        now_ms: f64,
    ) -> bool {
        let chain_healthy = stats
            .latest_video_timeline_observation
            .as_ref()
            .is_some_and(|observation: &XbxEngineVideoTimelineObservation| {
                observation.chain.state == "healthy"
            });
        if !chain_healthy {
            return false;
        }
        let track_attached_with_video =
            stats
                .latest_video_track_status
                .as_ref()
                .is_some_and(|track| {
                    track.state == "remoteTrackAttached" && track.video_bytes_total > 0
                });
        let pipeline_not_stalled = !stats.video_decoder_stalled.unwrap_or(false)
            && !stats.video_renderer_stalled.unwrap_or(false);
        track_attached_with_video && pipeline_not_stalled && has_fresh_media_output(stats, now_ms)
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
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        recovery_epoch: u64,
        observed_at_ms: f64,
    ) {
        if reason != VideoEscalationReason::TransportAwaitRecoveryKeyframe {
            self.await_recovery_keyframe_streak = 0;
            self.await_recovery_keyframe_last_seen_at_ms = Some(observed_at_ms);
            self.await_recovery_keyframe_streak_started_at_ms = None;
            return;
        }
        if !Self::has_transport_await_stage_escalation_failure_evidence(
            runtime_stats,
            recovery_epoch,
            observed_at_ms,
        ) {
            // 弱 transport-await / startup bootstrap 只保留本地探测，
            // 不累计 streak，避免后续凭抖动直接推进昂贵恢复。
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

impl RecoveryCoordinator {
    fn is_non_executing_recovery_action(action: RecoveryAction) -> bool {
        matches!(
            action,
            RecoveryAction::CooldownSuppressed
                | RecoveryAction::CoalescedKeyframeInFlight
                | RecoveryAction::CoalescedDecoderResetInFlight
                | RecoveryAction::WaitForBurst
                | RecoveryAction::WaitForDecoderResetBurst
                | RecoveryAction::StartupGraceSuppressed
        )
    }
}

fn classify_signal_domain(reason: VideoEscalationReason) -> RecoverySignalDomain {
    match reason {
        VideoEscalationReason::LifecycleRecovering
        | VideoEscalationReason::TransportExpiredDeadline
        | VideoEscalationReason::TransportSevereDeadline
        | VideoEscalationReason::TransportRecoveredLate
        | VideoEscalationReason::TransportSampleLoss => RecoverySignalDomain::Connectivity,
        VideoEscalationReason::DisplaySupplyCritical
        | VideoEscalationReason::Reconfigure
        | VideoEscalationReason::DecoderBackendFailure
        | VideoEscalationReason::AdapterIdleTimeout
        | VideoEscalationReason::AdapterThinStream => RecoverySignalDomain::Local,
        VideoEscalationReason::WaitKeyframe
        | VideoEscalationReason::TransportAwaitRecoveryKeyframe => {
            RecoverySignalDomain::MediaRecovery
        }
    }
}

#[cfg(test)]
#[path = "coordinator.test.rs"]
mod tests;
