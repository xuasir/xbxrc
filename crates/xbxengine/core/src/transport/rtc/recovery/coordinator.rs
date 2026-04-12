use std::sync::Mutex;
use std::time::{Duration, Instant};
use xbxengine_protocol::XbxEngineTransportStateDto;

use crate::runtime_stats_sink::expire_latest_keyframe_request_episode_if_unsent;
use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::recovery::contract::{
    current_clean_anchor_observed_at_ms, derive_gap_severity_from_timeline_observation,
    gap_severity_indicates_transport_recovery_pressure,
    has_current_transport_await_issue_from_observation, inspection_has_invalid_recovery_bootstrap,
    is_terminal_transport_await_deferred_episode,
};
use crate::transport::rtc::recovery::decoder_backend_failure::{
    resolve_decoder_backend_failure_recovery, DecoderBackendFailureResolution,
};
use crate::transport::rtc::recovery::escalation::{
    KeyframeTransportFeedback, RecoveryAction, RecoveryActionBudgetState,
    VideoEscalationBurstRollbackSnapshot, VideoEscalationController, VideoEscalationDecision,
    VideoEscalationReason,
};
use crate::transport::rtc::recovery::hard_stall::resolve_persistent_stall_recovery;
use crate::transport::rtc::recovery::nack_outcome::{
    resolve_recent_nack_outcome, CloudStartupExpiredDeadlineBudget, RecentNackOutcomeResolution,
};
use crate::transport::rtc::recovery::policy::RecoveryScenarioProfile;
use crate::transport::rtc::recovery::repeat_suppression::resolve_recent_repeat_suppression;
use crate::transport::rtc::recovery::runtime_state::{
    has_fresh_media_output, recovery_stage_label_from_stats, resolve_recovery_profile,
    resolve_runtime_recovery_profile, unix_now_ms,
};
#[cfg(test)]
use crate::transport::rtc::recovery::runtime_state::{
    runtime_state_for_diagnosis as build_runtime_state_for_diagnosis, RecoveryRuntimeState,
};
use crate::transport::rtc::recovery::startup::{
    resolve_session_phase, should_fast_reset_startup_recovery, should_suppress_startup_escalation,
    SessionPhase, StartupRecoveryProbe,
};
use crate::transport::rtc::session::control_model::{
    resolve_session_fault_domain, SessionFaultDomain,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TransportAwaitRecoveryStage {
    ProbeKeyframe,
    BootstrapInFlight,
    AwaitDecodeProgress,
    AwaitDecoderResetProgress,
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
const TRANSPORT_AWAIT_KEYFRAME_PACKET_ONLY_GRACE_MS: f64 = 220.0;
const TRANSPORT_AWAIT_KEYFRAME_DECODED_GRACE_MS: f64 = 220.0;
const TRANSPORT_AWAIT_DECODER_RESET_INFLIGHT_GRACE_MS: f64 = 900.0;
const TRANSPORT_AWAIT_INVALID_KEYFRAME_RESPONSE_FRESH_MS: f64 = 1_500.0;
const TRANSPORT_AWAIT_RECOVERY_SUSTAINING_MAX_AGE_MS: f64 = 2_400.0;
const TRANSPORT_AWAIT_RECOVERY_SUSTAINING_PROGRESS_MAX_AGE_MS: f64 = 900.0;
const CONNECTIVITY_JITTER_ABSORB_PRESENT_AGE_MAX_MS: f64 = 280.0;

impl RecoveryCoordinator {
    fn transport_await_decoder_reset_or_reconnect_fallback(
        &mut self,
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        recovery_epoch: u64,
        now_ms: f64,
    ) -> crate::transport::rtc::recovery::escalation::VideoEscalationDecision {
        let budget = self.escalation_controller.budget_state();
        if Self::should_retry_transport_await_keyframe_after_failed_local_reset(
            runtime_stats,
            recovery_epoch,
            now_ms,
        ) {
            return self.escalation_controller.suppressed(
                if budget.keyframe_budget_used < budget.keyframe_budget_limit {
                    RecoveryAction::RequestKeyframe
                } else if budget.reconnect_budget_used < budget.reconnect_budget_limit {
                    RecoveryAction::RequestReconnectCandidate
                } else {
                    RecoveryAction::CooldownSuppressed
                },
            );
        }
        if self.transport_await_decoder_reset_budget_exhausted() {
            return self.escalation_controller.suppressed(
                if budget.reconnect_budget_used < budget.reconnect_budget_limit {
                    RecoveryAction::RequestReconnectCandidate
                } else {
                    RecoveryAction::CooldownSuppressed
                },
            );
        }
        self.escalation_controller
            .suppressed(RecoveryAction::RequestDecoderReset)
    }

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

    pub(crate) fn transport_await_recovery_stage(
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        recovery_epoch: u64,
        now_ms: f64,
        bootstrap_in_flight_hint: bool,
    ) -> Option<TransportAwaitRecoveryStage> {
        if Self::transport_await_terminal_deferred_episode_active(
            runtime_stats,
            recovery_epoch,
            now_ms,
        ) {
            return Some(TransportAwaitRecoveryStage::ProbeKeyframe);
        }
        if bootstrap_in_flight_hint
            && Self::transport_await_bootstrap_in_flight_active(
                runtime_stats,
                recovery_epoch,
                now_ms,
            )
        {
            return Some(TransportAwaitRecoveryStage::BootstrapInFlight);
        }
        if Self::has_transport_await_stage_escalation_failure_evidence(
            runtime_stats,
            recovery_epoch,
            now_ms,
        ) {
            return None;
        }
        let latest_reset_attempt_started_at_ms =
            RuntimeStatsSink::read_shared(runtime_stats, |stats| {
                let observation = stats.latest_video_escalation_observation.as_ref()?;
                if stats.transport_recovery_epoch_at_last_escalation != recovery_epoch {
                    return None;
                }
                if observation.reason != "transportAwaitRecoveryKeyframe" {
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
                Some(observation.observed_at_ms)
            })
            .flatten();
        if latest_reset_attempt_started_at_ms.is_some_and(|started_at_ms| {
            Self::has_transport_await_decoder_reset_attempt_still_in_flight(
                runtime_stats,
                started_at_ms,
                now_ms,
            )
        }) {
            return Some(TransportAwaitRecoveryStage::AwaitDecoderResetProgress);
        }
        if Self::transport_await_bootstrap_in_flight_active(runtime_stats, recovery_epoch, now_ms) {
            return Some(TransportAwaitRecoveryStage::BootstrapInFlight);
        }
        if Self::has_recent_transport_await_keyframe_attempt(runtime_stats, recovery_epoch, now_ms)
        {
            return Some(TransportAwaitRecoveryStage::AwaitDecodeProgress);
        }
        Some(TransportAwaitRecoveryStage::ProbeKeyframe)
    }

    pub(crate) fn transport_await_recovery_stage_from_runtime(
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        now_ms: f64,
    ) -> Option<TransportAwaitRecoveryStage> {
        Self::transport_await_lane(runtime_stats, now_ms)
    }

    pub(crate) fn transport_await_local_recovery_active(
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        recovery_epoch: u64,
        now_ms: f64,
    ) -> bool {
        Self::transport_await_recovery_stage(runtime_stats, recovery_epoch, now_ms, false).is_some()
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
        self.release_stale_transport_await_keyframe_family(
            runtime_stats,
            recovery_epoch,
            signal.observed_at_ms,
        );
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
        if signal.reason == VideoEscalationReason::TransportAwaitRecoveryKeyframe
            && Self::transport_await_unsent_terminal_response_active(
                runtime_stats,
                recovery_epoch,
                signal.observed_at_ms,
            )
        {
            let decision = self
                .escalation_controller
                .reopen_transport_await_keyframe(recovery_epoch);
            return RecoveryCoordinatorProposal {
                signal,
                decision,
                budget_before,
                budget_after: self.escalation_controller.budget_state(),
            };
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
            signal.reason_label.as_str(),
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

    pub(crate) fn rollback_decoder_reset_burst_after_transport_family_defer(&mut self) {
        self.escalation_controller
            .rollback_decoder_reset_burst_after_transport_family_defer();
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
        reason_label: &str,
        phase: SessionPhase,
        profile: RecoveryScenarioProfile,
        recovery_epoch: u64,
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        observed_at_ms: f64,
    ) -> VideoEscalationDecision {
        let bootstrap_in_flight_hint = reason
            == VideoEscalationReason::TransportAwaitRecoveryKeyframe
            && Self::is_recovery_sustaining_reason_label(reason_label);
        let signal_domain = classify_signal_domain(reason);
        let transport_await_recovery_stage = (reason
            == VideoEscalationReason::TransportAwaitRecoveryKeyframe)
            .then(|| {
                Self::transport_await_recovery_stage(
                    runtime_stats,
                    recovery_epoch,
                    observed_at_ms,
                    bootstrap_in_flight_hint,
                )
            })
            .flatten();
        let transport_await_terminal_deferred_qualified = reason
            == VideoEscalationReason::TransportAwaitRecoveryKeyframe
            && Self::transport_await_terminal_deferred_episode_active(
                runtime_stats,
                recovery_epoch,
                observed_at_ms,
            );
        let transport_await_probe_keyframe_qualified =
            transport_await_recovery_stage == Some(TransportAwaitRecoveryStage::ProbeKeyframe);
        let transport_await_bootstrap_in_flight_qualified =
            transport_await_recovery_stage == Some(TransportAwaitRecoveryStage::BootstrapInFlight);
        // clean-anchor sustaining 窗口会吞抖动，但若 inspection 已明确「无效恢复入口」(NonIDR 等)，
        // 不应继续当作 hard_evidence=false / 禁止升级，否则会在 transport-await 上傻等。
        let transport_await_sustaining_shattered = transport_await_bootstrap_in_flight_qualified
            && Self::transport_await_fresh_invalid_recovery_bootstrap_shatters_sustaining(
                runtime_stats,
                recovery_epoch,
                observed_at_ms,
            );
        let transport_await_hard_evidence = if transport_await_bootstrap_in_flight_qualified
            && !transport_await_sustaining_shattered
        {
            false
        } else {
            reason != VideoEscalationReason::TransportAwaitRecoveryKeyframe
                || Self::has_transport_await_stage_escalation_failure_evidence(
                    runtime_stats,
                    recovery_epoch,
                    observed_at_ms,
                )
        };
        let transport_await_await_decode_progress_qualified = transport_await_recovery_stage
            == Some(TransportAwaitRecoveryStage::AwaitDecodeProgress);
        let transport_await_await_reset_progress_qualified = transport_await_recovery_stage
            == Some(TransportAwaitRecoveryStage::AwaitDecoderResetProgress);
        let transport_await_first_frame_priority_active = reason
            == VideoEscalationReason::TransportAwaitRecoveryKeyframe
            && Self::transport_await_first_frame_acquisition_priority_active(
                runtime_stats,
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
        let (mut escalation_decision, burst_rollback_snap): (
            VideoEscalationDecision,
            Option<VideoEscalationBurstRollbackSnapshot>,
        ) = if phase == SessionPhase::Startup
            && should_suppress_startup_escalation(
                &reason,
                self.stream_started_at,
                self.startup_grace,
            ) {
            (
                self.escalation_controller
                    .suppressed(RecoveryAction::StartupGraceSuppressed),
                None,
            )
        } else if signal_domain == RecoverySignalDomain::Connectivity
            && reason != VideoEscalationReason::LifecycleRecovering
            && Self::should_absorb_connectivity_jitter(runtime_stats, reason, observed_at_ms)
        {
            // 小幅连接域抖动下，若本地 media edge 仍持续推进，则先吸收在本地观察层，
            // 避免将 transient deadline/sample-loss 直接推入 reconnect 升级计数。
            (
                self.escalation_controller
                    .suppressed(RecoveryAction::CooldownSuppressed),
                None,
            )
        } else {
            let snap = self.escalation_controller.capture_burst_rollback_snapshot();
            let decision = self.escalation_controller.on_reason_with_epoch_policy(
                reason,
                recovery_epoch,
                signal_domain == RecoverySignalDomain::Connectivity,
                allow_transport_await_stage_escalation,
                allow_wait_keyframe_stage_escalation,
                allow_reconfigure_stage_escalation,
            );
            (decision, Some(snap))
        };
        let naive_action = escalation_decision.action;
        if reason == VideoEscalationReason::TransportAwaitRecoveryKeyframe
            && transport_await_bootstrap_in_flight_qualified
            && !transport_await_sustaining_shattered
            && !transport_await_terminal_deferred_qualified
            && !Self::is_non_executing_recovery_action(escalation_decision.action)
        {
            escalation_decision = self
                .escalation_controller
                .suppressed(RecoveryAction::WaitForBurst);
        }
        if reason == VideoEscalationReason::TransportAwaitRecoveryKeyframe
            && transport_await_first_frame_priority_active
        {
            self.clear_transport_await_hard_fallback("firstFrameAcquisitionPriority");
        }
        if reason == VideoEscalationReason::TransportAwaitRecoveryKeyframe
            && !transport_await_hard_evidence
            && transport_await_probe_keyframe_qualified
            && transport_await_first_frame_priority_active
            && !transport_await_sustaining_shattered
            && !transport_await_terminal_deferred_qualified
            && Self::transport_await_action_leaves_local_probe_domain(escalation_decision.action)
        {
            escalation_decision = self
                .escalation_controller
                .suppressed(RecoveryAction::WaitForBurst);
        }
        if reason == VideoEscalationReason::TransportAwaitRecoveryKeyframe
            && transport_await_await_reset_progress_qualified
        {
            escalation_decision = self
                .escalation_controller
                .suppressed(RecoveryAction::CoalescedDecoderResetInFlight);
        }
        if reason == VideoEscalationReason::TransportAwaitRecoveryKeyframe
            && !transport_await_hard_evidence
            && transport_await_await_decode_progress_qualified
            && !transport_await_sustaining_shattered
            && !transport_await_terminal_deferred_qualified
            && !Self::is_non_executing_recovery_action(escalation_decision.action)
        {
            escalation_decision = self
                .escalation_controller
                .suppressed(RecoveryAction::WaitForBurst);
        }
        if reason == VideoEscalationReason::TransportAwaitRecoveryKeyframe
            && transport_await_terminal_deferred_qualified
            && matches!(
                escalation_decision.action,
                RecoveryAction::WaitForBurst | RecoveryAction::CoalescedKeyframeInFlight
            )
        {
            escalation_decision.action = RecoveryAction::RequestKeyframe;
        }
        if reason == VideoEscalationReason::TransportAwaitRecoveryKeyframe
            && matches!(
                escalation_decision.action,
                RecoveryAction::CoalescedKeyframeInFlight
            )
            && Self::transport_await_ingress_still_waiting(
                runtime_stats,
                recovery_epoch,
                observed_at_ms,
            )
            && Self::has_transport_await_stage_escalation_failure_evidence(
                runtime_stats,
                recovery_epoch,
                observed_at_ms,
            )
            && Self::has_keyframe_stage_escalation_failure_evidence(
                runtime_stats,
                recovery_epoch,
                observed_at_ms,
            )
            && !Self::transport_await_soft_reentry_is_recent_and_healthy(
                runtime_stats,
                recovery_epoch,
                observed_at_ms,
            )
            && !transport_await_bootstrap_in_flight_qualified
            && !transport_await_first_frame_priority_active
        {
            escalation_decision = self.transport_await_decoder_reset_or_reconnect_fallback(
                runtime_stats,
                recovery_epoch,
                observed_at_ms,
            );
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
                RecoveryAction::RequestKeyframe
                    | RecoveryAction::RequestDecoderReset
                    | RecoveryAction::CoalescedKeyframeInFlight
                    | RecoveryAction::CooldownSuppressed
            )
            && Self::has_keyframe_stage_escalation_failure_evidence(
                runtime_stats,
                recovery_epoch,
                observed_at_ms,
            )
            && !transport_await_first_frame_priority_active
        {
            // 一旦同一 recovery epoch 已经出现明确的 keyframe-stage failure evidence，
            // 说明 transport-await 不该再退回“重发一次 keyframe”阶段，而应直接跨到 reset。
            let decoder_reset_attempt_started_at_ms = self
                .await_recovery_keyframe_streak_started_at_ms
                .filter(|started_at_ms| {
                    Self::has_transport_await_decoder_reset_attempt_since(
                        runtime_stats,
                        *started_at_ms,
                    )
                });
            let decoder_reset_in_flight =
                decoder_reset_attempt_started_at_ms.is_some_and(|started_at_ms| {
                    Self::has_transport_await_decoder_reset_attempt_still_in_flight(
                        runtime_stats,
                        started_at_ms,
                        observed_at_ms,
                    )
                });
            escalation_decision =
                self.escalation_controller
                    .suppressed(if decoder_reset_in_flight {
                        RecoveryAction::CoalescedDecoderResetInFlight
                    } else {
                        RecoveryAction::RequestDecoderReset
                    });
        }
        if let Some(forced_stage_decision) = self.maybe_force_transport_await_stage_upgrade(
            runtime_stats,
            reason,
            reason_label,
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
            reason_label,
            recovery_epoch,
            profile,
            runtime_stats,
            observed_at_ms,
        ) {
            escalation_decision = hard_fallback_decision;
        }
        if reason == VideoEscalationReason::TransportAwaitRecoveryKeyframe
            && transport_await_first_frame_priority_active
        {
            escalation_decision.action = Self::constrain_transport_await_first_frame_action(
                runtime_stats,
                recovery_epoch,
                observed_at_ms,
                escalation_decision.action,
            );
        }
        if let Some(snap) = burst_rollback_snap {
            if Self::coordinator_burst_rollback_warranted(naive_action, escalation_decision.action)
            {
                self.escalation_controller
                    .restore_burst_rollback_snapshot(snap);
            }
        }
        let action = escalation_decision.action;
        if startup_fast_reset
            && !transport_await_first_frame_priority_active
            && action == RecoveryAction::RequestKeyframe
        {
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
        _reason_label: &str,
        recovery_epoch: u64,
        profile: RecoveryScenarioProfile,
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        observed_at_ms: f64,
    ) -> Option<VideoEscalationDecision> {
        if reason != VideoEscalationReason::TransportAwaitRecoveryKeyframe {
            return None;
        }
        if Self::transport_await_first_frame_acquisition_priority_active(
            runtime_stats,
            observed_at_ms,
        ) {
            self.reset_transport_await_hard_fallback(
                runtime_stats,
                "firstFrameAcquisitionPriority",
                true,
            );
            return None;
        }
        if Self::transport_await_bootstrap_in_flight_active(
            runtime_stats,
            recovery_epoch,
            observed_at_ms,
        ) {
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
        let decoder_reset_attempted =
            Self::has_transport_await_decoder_reset_attempt_since(runtime_stats, started_at_ms);
        let decoder_reset_still_in_flight =
            Self::has_transport_await_decoder_reset_attempt_still_in_flight(
                runtime_stats,
                started_at_ms,
                observed_at_ms,
            );
        if !decoder_reset_attempted {
            // hard fallback 只接受“持续坏窗 + 已经失去本地恢复进展”的升级。
            // 如果还没有 decoder reset 尝试，除 reconnecting 以外都先留在本地恢复链。
            if recovery_stage != "reconnecting" {
                return None;
            }
        }
        if decoder_reset_still_in_flight {
            return Some(
                self.escalation_controller
                    .suppressed(RecoveryAction::CoalescedDecoderResetInFlight),
            );
        }
        // 本地 decoder reset 已经明确耗尽，且 hard fallback 仍然确认没有任何恢复进展，
        // 继续压回 RequestDecoderReset 只会把 transport-await 锁死在本地恢复回路里。
        let decision = self.transport_await_decoder_reset_or_reconnect_fallback(
            runtime_stats,
            recovery_epoch,
            observed_at_ms,
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
        reason_label: &str,
        profile: RecoveryScenarioProfile,
        recovery_epoch: u64,
        observed_at_ms: f64,
        current_action: RecoveryAction,
    ) -> Option<VideoEscalationDecision> {
        const TRANSPORT_AWAIT_CONNECTED_BAD_WINDOW_STAGE_MIN_MS: f64 = 120.0;
        let bootstrap_in_flight_soft_hold_active =
            Self::is_recovery_sustaining_reason_label(reason_label)
                && Self::transport_await_bootstrap_in_flight_active(
                    runtime_stats,
                    recovery_epoch,
                    observed_at_ms,
                );
        if reason != VideoEscalationReason::TransportAwaitRecoveryKeyframe
            || bootstrap_in_flight_soft_hold_active
            || !matches!(
                current_action,
                RecoveryAction::CooldownSuppressed | RecoveryAction::CoalescedKeyframeInFlight
            )
        {
            return None;
        }
        if Self::transport_await_first_frame_acquisition_priority_active(
            runtime_stats,
            observed_at_ms,
        ) {
            return None;
        }
        if Self::transport_await_pre_first_frame_availability_active(runtime_stats, observed_at_ms)
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
            return Some(self.transport_await_decoder_reset_or_reconnect_fallback(
                runtime_stats,
                recovery_epoch,
                observed_at_ms,
            ));
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
            return Some(self.transport_await_decoder_reset_or_reconnect_fallback(
                runtime_stats,
                recovery_epoch,
                observed_at_ms,
            ));
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
            if Self::transport_await_pre_first_frame_availability_active_from_stats(stats, now_ms) {
                return false;
            }
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
            let timeline_recovery_pressure = stats
                .latest_video_timeline_observation
                .as_ref()
                .is_some_and(|timeline| {
                    (now_ms - timeline.observed_at_ms).max(0.0) <= 1_500.0
                        && gap_severity_indicates_transport_recovery_pressure(
                            derive_gap_severity_from_timeline_observation(timeline),
                        )
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
                || timeline_recovery_pressure
                || expired_deadline
                || unsent_keyframe_request
                || recent_rtcp_failure
        })
        .unwrap_or(false)
    }

    /// sustaining（bootstrap-in-flight）期间若已观测到「无效恢复 bootstrap」且 transport-await 仍未解，
    /// 视为 sustaining 被击碎：允许重新累积 hard evidence / keyframe 升级，避免在 NonIDR 响应上傻等。
    fn transport_await_fresh_invalid_recovery_bootstrap_shatters_sustaining(
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        recovery_epoch: u64,
        now_ms: f64,
    ) -> bool {
        RuntimeStatsSink::read_shared(runtime_stats, |stats| {
            Self::transport_await_fresh_invalid_recovery_bootstrap_shatters_sustaining_from_stats(
                stats,
                recovery_epoch,
                now_ms,
            )
        })
        .unwrap_or(false)
    }

    fn transport_await_fresh_invalid_recovery_bootstrap_shatters_sustaining_from_stats(
        stats: &XbxEngineMediaRuntimeStats,
        _recovery_epoch: u64,
        now_ms: f64,
    ) -> bool {
        const FRESH_MS: f64 = 1_500.0;
        // 时间线仍处在 transport-await 未解时，ledger 上可能暂时还有 SubmittedCleanAnchor；
        // 若 inspection 已给出「无效恢复 bootstrap」(NonIDR 等)，说明 sustaining 乐观假设不成立，
        // 不能再把 hard_evidence / keyframe 升级压在 WaitForBurst 后傻等。
        if !Self::has_unresolved_transport_await_issue(stats) {
            return false;
        }
        stats
            .latest_h264_inspection_observation
            .as_ref()
            .is_some_and(|inspection| {
                (now_ms - inspection.observed_at_ms).max(0.0) <= FRESH_MS
                    && inspection_has_invalid_recovery_bootstrap(inspection)
            })
    }

    fn has_transport_await_stage_escalation_failure_evidence(
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        recovery_epoch: u64,
        now_ms: f64,
    ) -> bool {
        const TRANSPORT_AWAIT_HARD_EVIDENCE_FRESH_MS: f64 = 1_500.0;
        RuntimeStatsSink::read_shared(runtime_stats, |stats| {
            if Self::transport_await_pre_first_frame_availability_active_from_stats(stats, now_ms) {
                return false;
            }
            if Self::transport_await_bootstrap_in_flight_active_from_stats(
                stats,
                recovery_epoch,
                now_ms,
            ) && !Self::transport_await_fresh_invalid_recovery_bootstrap_shatters_sustaining_from_stats(
                stats,
                recovery_epoch,
                now_ms,
            ) {
                return false;
            }
            if Self::transport_await_first_frame_acquisition_priority_active_from_stats(stats, now_ms) {
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
            let packet_seen_without_decode = Self::transport_await_packet_seen_without_decode_failure(
                stats,
                recovery_epoch,
                now_ms,
            );
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
                || packet_seen_without_decode
                || recent_h264_requires_keyframe
                || (recent_hard_anchor_failure
                    && (objective_media_unhealthy || lacks_recent_clean_anchor))
                || (objective_media_unhealthy
                    && lacks_recent_clean_anchor
                    && recent_h264_requires_keyframe)
        })
        .unwrap_or(false)
    }

    fn transport_await_first_frame_acquisition_priority_active(
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        now_ms: f64,
    ) -> bool {
        RuntimeStatsSink::read_shared(runtime_stats, |stats| {
            Self::transport_await_first_frame_acquisition_priority_active_from_stats(stats, now_ms)
        })
        .unwrap_or(false)
    }

    fn transport_await_pre_first_frame_availability_active(
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        now_ms: f64,
    ) -> bool {
        RuntimeStatsSink::read_shared(runtime_stats, |stats| {
            Self::transport_await_pre_first_frame_availability_active_from_stats(stats, now_ms)
        })
        .unwrap_or(false)
    }

    fn transport_await_first_frame_acquisition_priority_active_from_stats(
        stats: &XbxEngineMediaRuntimeStats,
        now_ms: f64,
    ) -> bool {
        Self::transport_await_pre_first_frame_availability_active_from_stats(stats, now_ms)
    }

    fn transport_await_pre_first_frame_availability_active_from_stats(
        stats: &XbxEngineMediaRuntimeStats,
        now_ms: f64,
    ) -> bool {
        let before_first_present = stats.host_display_tick_epoch > 0
            && stats.video_present_epoch == 0
            && stats.video_present_submit_count_total == 0
            && stats.latest_video_host_present_time_ms.is_none()
            && stats.latest_video_decode_ok_time_ms.is_none();
        if !before_first_present {
            return false;
        }
        let track_attached_with_video =
            stats
                .latest_video_track_status
                .as_ref()
                .is_some_and(|track| {
                    track.state == "remoteTrackAttached" && track.video_bytes_total > 0
                });
        if !track_attached_with_video {
            return false;
        }
        // 与 `session::startup_compat` 对齐：首帧前抑制有上限，避免永久挡住无效关键帧判定。
        Self::transport_await_pre_first_frame_fallback_within_window(stats, now_ms)
    }

    fn transport_await_pre_first_frame_fallback_within_window(
        stats: &XbxEngineMediaRuntimeStats,
        observed_at_ms: f64,
    ) -> bool {
        let fallback_ms =
            resolve_runtime_recovery_profile(stats).pre_first_frame_reconnect_fallback_ms();
        match stats.first_video_packet_arrival_time_ms {
            Some(t0) => (observed_at_ms - t0).max(0.0) <= fallback_ms,
            None => false,
        }
    }

    fn constrain_transport_await_first_frame_action(
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        recovery_epoch: u64,
        now_ms: f64,
        action: RecoveryAction,
    ) -> RecoveryAction {
        if Self::transport_await_terminal_deferred_episode_active(
            runtime_stats,
            recovery_epoch,
            now_ms,
        ) {
            return RecoveryAction::RequestKeyframe;
        }
        match action {
            RecoveryAction::RequestKeyframe
            | RecoveryAction::CoalescedKeyframeInFlight
            | RecoveryAction::WaitForBurst
            | RecoveryAction::CooldownSuppressed => action,
            RecoveryAction::CoalescedDecoderResetInFlight
            | RecoveryAction::WaitForDecoderResetBurst
            | RecoveryAction::StartupGraceSuppressed => RecoveryAction::WaitForBurst,
            _ => {
                if Self::has_recent_transport_await_keyframe_attempt(
                    runtime_stats,
                    recovery_epoch,
                    now_ms,
                ) {
                    RecoveryAction::CoalescedKeyframeInFlight
                } else {
                    RecoveryAction::RequestKeyframe
                }
            }
        }
    }

    fn has_recent_transport_await_keyframe_attempt(
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        recovery_epoch: u64,
        now_ms: f64,
    ) -> bool {
        const TRANSPORT_AWAIT_ATTEMPT_FRESH_MS: f64 = 4_500.0;
        RuntimeStatsSink::read_shared(runtime_stats, |stats| {
            if Self::transport_await_terminal_deferred_episode(stats, recovery_epoch, now_ms) {
                return false;
            }
            if !Self::has_unresolved_transport_await_issue(stats) {
                return false;
            }
            let recent_episode =
                stats
                    .latest_keyframe_request_episode
                    .as_ref()
                    .is_some_and(|episode| {
                        Self::transport_await_keyframe_attempt_still_blocks_retry(
                            stats,
                            episode,
                            recovery_epoch,
                            now_ms,
                        ) && (now_ms - episode.requested_at_ms).max(0.0)
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
            if Self::transport_await_pre_first_frame_availability_active_from_stats(stats, now_ms) {
                return false;
            }
            if Self::transport_await_bootstrap_in_flight_active_from_stats(
                stats,
                recovery_epoch,
                now_ms,
            ) {
                return false;
            }
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
                    (now_ms - decoded_at_ms).max(0.0)
                        >= TRANSPORT_AWAIT_KEYFRAME_UNUSABLE_GRACE_MS
                })
                && unresolved_transport_await
                && lacks_recent_clean_anchor;
            let packet_seen_without_decode =
                Self::transport_await_packet_seen_without_decode_failure(
                    stats,
                    recovery_epoch,
                    now_ms,
                );
            decoded_without_usable_anchor || packet_seen_without_decode
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
                    matches!(
                        observation.action.as_str(),
                        "requestDecoderReset"
                            | "requestKeyframe+decoderReset"
                            | "requestKeyframe+decoderReset(startupLowQualityRetry)"
                    ) && observation.observed_at_ms >= started_at_ms
                });
            decoder_reset_applied || decoder_reset_requested
        })
        .unwrap_or(false)
    }

    fn should_retry_transport_await_keyframe_after_failed_local_reset(
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        recovery_epoch: u64,
        now_ms: f64,
    ) -> bool {
        RuntimeStatsSink::read_shared(runtime_stats, |stats| {
            let Some(latest_attempt_at_ms) =
                Self::latest_transport_await_decoder_reset_attempt_at_ms(stats)
            else {
                return false;
            };
            let keyframe_retry_still_blocked = stats
                .latest_keyframe_request_episode
                .as_ref()
                .is_some_and(|episode| {
                    Self::transport_await_keyframe_attempt_still_blocks_retry(
                        stats,
                        episode,
                        recovery_epoch,
                        now_ms,
                    )
                });
            if keyframe_retry_still_blocked {
                return false;
            }
            Self::transport_await_invalid_keyframe_response_after_attempt(
                stats,
                recovery_epoch,
                latest_attempt_at_ms,
                now_ms,
            ) || Self::transport_await_has_recent_unusable_nonidr_keyframe_response(
                stats,
                recovery_epoch,
                now_ms,
                Some(latest_attempt_at_ms),
            )
        })
        .unwrap_or(false)
    }

    fn has_transport_await_decoder_reset_attempt_still_in_flight(
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        started_at_ms: f64,
        now_ms: f64,
    ) -> bool {
        RuntimeStatsSink::read_shared(runtime_stats, |stats| {
            Self::transport_await_decoder_reset_attempt_still_in_flight_from_stats(
                stats,
                started_at_ms,
                now_ms,
            )
        })
        .unwrap_or(false)
    }

    fn transport_await_decoder_reset_attempt_still_in_flight_from_stats(
        stats: &XbxEngineMediaRuntimeStats,
        started_at_ms: f64,
        now_ms: f64,
    ) -> bool {
        let latest_attempt_at_ms = stats
            .latest_video_decoder_reset_time_ms
            .filter(|at_ms| *at_ms >= started_at_ms)
            .or_else(|| {
                stats
                    .latest_video_escalation_observation
                    .as_ref()
                    .filter(|observation| {
                        observation.observed_at_ms >= started_at_ms
                            && observation.reason == "transportAwaitRecoveryKeyframe"
                            && matches!(
                                observation.action.as_str(),
                                "requestDecoderReset"
                                    | "requestKeyframe+decoderReset"
                                    | "requestKeyframe+decoderReset(startupLowQualityRetry)"
                            )
                    })
                    .map(|observation| observation.observed_at_ms)
            });
        let Some(latest_attempt_at_ms) = latest_attempt_at_ms else {
            return false;
        };
        if Self::transport_await_has_recent_unusable_nonidr_keyframe_response(
            stats,
            stats.transport_recovery_epoch,
            now_ms,
            Some(latest_attempt_at_ms),
        ) {
            return false;
        }
        let progress_after_reset = stats
            .latest_video_host_present_time_ms
            .is_some_and(|presented_at_ms| presented_at_ms > latest_attempt_at_ms)
            || stats
                .latest_video_decode_ok_time_ms
                .is_some_and(|decoded_at_ms| decoded_at_ms > latest_attempt_at_ms)
            || stats
                .video_anchor_clean_observed_at_ms
                .is_some_and(|anchor_at_ms| anchor_at_ms > latest_attempt_at_ms);
        if Self::transport_await_invalid_keyframe_response_after_attempt(
            stats,
            stats.transport_recovery_epoch,
            latest_attempt_at_ms,
            now_ms,
        ) {
            return false;
        }
        !progress_after_reset
            && (now_ms - latest_attempt_at_ms).max(0.0)
                <= TRANSPORT_AWAIT_DECODER_RESET_INFLIGHT_GRACE_MS
    }

    fn transport_await_invalid_keyframe_response_after_attempt(
        stats: &XbxEngineMediaRuntimeStats,
        recovery_epoch: u64,
        attempt_at_ms: f64,
        now_ms: f64,
    ) -> bool {
        if Self::transport_await_pre_first_frame_availability_active_from_stats(stats, now_ms) {
            return false;
        }
        let unresolved_transport_await = Self::has_unresolved_transport_await_issue(stats);
        if !unresolved_transport_await {
            return false;
        }
        let lacks_recent_clean_anchor = !Self::has_recent_clean_anchor_evidence(
            stats.video_anchor_clean_epoch,
            stats.video_anchor_clean_observed_at_ms,
            stats.video_anchor_clean_source_event.as_deref(),
            stats.latest_anchor_candidate_ledger.as_ref(),
            recovery_epoch,
            now_ms,
        );
        let packet_seen_without_decode = stats
            .latest_keyframe_request_episode
            .as_ref()
            .is_some_and(|episode| {
                episode.request_reason.as_deref() == Some("transportAwaitRecoveryKeyframe")
                    && episode.status == "packet-seen"
                    && episode.first_keyframe_decoded_at_ms.is_none()
                    && episode
                        .first_keyframe_packet_at_ms
                        .is_some_and(|packet_at_ms| {
                            packet_at_ms >= attempt_at_ms
                                && (now_ms - packet_at_ms).max(0.0)
                                    >= TRANSPORT_AWAIT_KEYFRAME_PACKET_ONLY_GRACE_MS
                        })
            });
        if packet_seen_without_decode && lacks_recent_clean_anchor {
            return true;
        }
        let decoded_without_usable_anchor = stats
            .latest_keyframe_request_episode
            .as_ref()
            .is_some_and(|episode| {
                episode.request_reason.as_deref() == Some("transportAwaitRecoveryKeyframe")
                    && episode.status == "decoded"
                    && episode
                        .first_keyframe_decoded_at_ms
                        .is_some_and(|decoded_at_ms| {
                            decoded_at_ms >= attempt_at_ms
                                && (now_ms - decoded_at_ms).max(0.0)
                                    >= TRANSPORT_AWAIT_KEYFRAME_DECODED_GRACE_MS
                        })
            });
        if decoded_without_usable_anchor && lacks_recent_clean_anchor {
            return true;
        }
        let inspection_implies_post_attempt_unusable_response = stats
            .latest_h264_inspection_observation
            .as_ref()
            .is_some_and(|inspection| {
                inspection.observed_at_ms >= attempt_at_ms
                    && inspection.admission_accepted
                    // admission 为 Accept 时 continuation 可能仍带 bootstrap_reject_reason（仅描述 AU 自举），
                    // 不应与「尝试后仍不可用关键帧响应」混为一谈。
                    && !(inspection.delta_continuation_ready
                        && inspection.committed_sps_present
                        && inspection.committed_pps_present)
                    && matches!(
                        inspection.bootstrap_reject_reason.as_deref(),
                        Some(
                            "NonIdrVcl"
                                | "bootstrapMissingSps"
                                | "bootstrapMissingPps"
                                | "inspectionRejectInvalidSliceHeader"
                        )
                    )
                    && stats
                        .latest_keyframe_request_episode
                        .as_ref()
                        .is_some_and(|episode| {
                            episode.request_reason.as_deref()
                                == Some("transportAwaitRecoveryKeyframe")
                                && matches!(episode.status.as_str(), "packet-seen" | "decoded")
                                && episode.response_verdict.as_deref() != Some("transportDeferred")
                        })
            });
        if inspection_implies_post_attempt_unusable_response && lacks_recent_clean_anchor {
            return true;
        }
        stats
            .latest_h264_inspection_observation
            .as_ref()
            .is_some_and(|inspection| {
                inspection.observed_at_ms >= attempt_at_ms
                    && (now_ms - inspection.observed_at_ms).max(0.0)
                        <= TRANSPORT_AWAIT_INVALID_KEYFRAME_RESPONSE_FRESH_MS
                    && !inspection.bootstrap_ready
                    && matches!(
                        inspection.bootstrap_reject_reason.as_deref(),
                        Some(
                            "NonIdrVcl"
                                | "bootstrapMissingSps"
                                | "bootstrapMissingPps"
                                | "inspectionRejectInvalidSliceHeader"
                        )
                    )
                    && lacks_recent_clean_anchor
            })
    }

    fn latest_transport_await_decoder_reset_attempt_at_ms(
        stats: &XbxEngineMediaRuntimeStats,
    ) -> Option<f64> {
        stats.latest_video_decoder_reset_time_ms.or_else(|| {
            stats
                .latest_video_escalation_observation
                .as_ref()
                .filter(|observation| {
                    observation.reason == "transportAwaitRecoveryKeyframe"
                        && matches!(
                            observation.action.as_str(),
                            "requestDecoderReset"
                                | "requestKeyframe+decoderReset"
                                | "requestKeyframe+decoderReset(startupLowQualityRetry)"
                        )
                })
                .map(|observation| observation.observed_at_ms)
        })
    }

    fn transport_await_lane(
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        now_ms: f64,
    ) -> Option<TransportAwaitRecoveryStage> {
        RuntimeStatsSink::read_shared(runtime_stats, |stats| {
            if !Self::has_unresolved_transport_await_issue(stats) {
                return None;
            }
            if let Some(started_at_ms) =
                Self::latest_transport_await_decoder_reset_attempt_at_ms(stats)
            {
                if Self::transport_await_decoder_reset_attempt_still_in_flight_from_stats(
                    stats,
                    started_at_ms,
                    now_ms,
                ) {
                    return Some(TransportAwaitRecoveryStage::AwaitDecoderResetProgress);
                }
            }
            if Self::transport_await_bootstrap_in_flight_active_from_stats(
                stats,
                stats.transport_recovery_epoch,
                now_ms,
            ) {
                return Some(TransportAwaitRecoveryStage::BootstrapInFlight);
            }
            if Self::transport_await_has_recent_unusable_nonidr_keyframe_response(
                stats,
                stats.transport_recovery_epoch,
                now_ms,
                None,
            ) {
                return Some(TransportAwaitRecoveryStage::ProbeKeyframe);
            }
            let Some(episode) = stats.latest_keyframe_request_episode.as_ref() else {
                return Some(TransportAwaitRecoveryStage::ProbeKeyframe);
            };
            if episode.request_reason.as_deref() != Some("transportAwaitRecoveryKeyframe") {
                return Some(TransportAwaitRecoveryStage::ProbeKeyframe);
            }
            match episode.status.as_str() {
                "packet-seen" | "decoded" => Some(TransportAwaitRecoveryStage::AwaitDecodeProgress),
                "requested" | "sent" => Some(TransportAwaitRecoveryStage::ProbeKeyframe),
                _ => Some(TransportAwaitRecoveryStage::ProbeKeyframe),
            }
        })
        .unwrap_or(None)
    }

    fn transport_await_bootstrap_in_flight_active(
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        recovery_epoch: u64,
        now_ms: f64,
    ) -> bool {
        RuntimeStatsSink::read_shared(runtime_stats, |stats| {
            Self::transport_await_bootstrap_in_flight_active_from_stats(
                stats,
                recovery_epoch,
                now_ms,
            )
        })
        .unwrap_or(false)
    }

    fn transport_await_terminal_deferred_episode_active(
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        recovery_epoch: u64,
        now_ms: f64,
    ) -> bool {
        RuntimeStatsSink::read_shared(runtime_stats, |stats| {
            Self::transport_await_terminal_deferred_episode(stats, recovery_epoch, now_ms)
        })
        .unwrap_or(false)
    }

    fn transport_await_unsent_terminal_response_active(
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        recovery_epoch: u64,
        now_ms: f64,
    ) -> bool {
        RuntimeStatsSink::read_shared(runtime_stats, |stats| {
            let has_clean_anchor_evidence = Self::has_recent_clean_anchor_evidence(
                stats.video_anchor_clean_epoch,
                stats.video_anchor_clean_observed_at_ms,
                stats.video_anchor_clean_source_event.as_deref(),
                stats.latest_anchor_candidate_ledger.as_ref(),
                recovery_epoch,
                now_ms,
            );
            stats
                .latest_keyframe_request_episode
                .as_ref()
                .is_some_and(|episode| {
                    episode.sent_at_ms.is_none()
                        && is_terminal_transport_await_deferred_episode(
                            episode,
                            stats.latest_h264_inspection_observation.as_ref(),
                            has_clean_anchor_evidence,
                            now_ms,
                            TRANSPORT_AWAIT_INVALID_KEYFRAME_RESPONSE_FRESH_MS,
                        )
                })
        })
        .unwrap_or(false)
    }

    fn transport_await_bootstrap_in_flight_active_from_stats(
        stats: &XbxEngineMediaRuntimeStats,
        recovery_epoch: u64,
        now_ms: f64,
    ) -> bool {
        if Self::transport_await_terminal_deferred_episode(stats, recovery_epoch, now_ms) {
            return false;
        }
        let recent_clean_anchor_submitted_at_ms = stats
            .latest_anchor_candidate_ledger
            .as_ref()
            .filter(|candidate| {
                candidate.recovery_epoch == recovery_epoch
                    && candidate.state == XbxEngineAnchorCandidateState::SubmittedCleanAnchor
                    && candidate.source_event == "chain-clean-keyframe-submitted"
            })
            .map(|candidate| candidate.observed_at_ms)
            .or_else(|| {
                stats
                    .video_anchor_clean_epoch
                    .filter(|epoch| *epoch == recovery_epoch)
                    .zip(stats.video_anchor_clean_observed_at_ms)
                    .filter(|_| {
                        stats.video_anchor_clean_source_event.as_deref()
                            == Some("chain-clean-keyframe-submitted")
                    })
                    .map(|(_, observed_at_ms)| observed_at_ms)
            });
        let Some(submitted_at_ms) = recent_clean_anchor_submitted_at_ms else {
            return false;
        };
        let has_transport_await_attempt_context = stats
            .latest_keyframe_request_episode
            .as_ref()
            .is_some_and(|episode| {
                episode.request_reason.as_deref() == Some("transportAwaitRecoveryKeyframe")
                    && episode.requested_at_ms <= now_ms
                    && episode.requested_at_ms <= submitted_at_ms
                    && (submitted_at_ms - episode.requested_at_ms).max(0.0)
                        <= TRANSPORT_AWAIT_RECOVERY_SUSTAINING_MAX_AGE_MS
            })
            || stats
                .latest_video_escalation_observation
                .as_ref()
                .is_some_and(|observation| {
                    stats.transport_recovery_epoch_at_last_escalation == recovery_epoch
                        && observation.reason == "transportAwaitRecoveryKeyframe"
                        && observation.action == "requestKeyframe"
                        && observation.observed_at_ms <= now_ms
                        && observation.observed_at_ms <= submitted_at_ms
                        && (submitted_at_ms - observation.observed_at_ms).max(0.0)
                            <= TRANSPORT_AWAIT_RECOVERY_SUSTAINING_MAX_AGE_MS
                        && (now_ms - observation.observed_at_ms).max(0.0)
                            <= TRANSPORT_AWAIT_RECOVERY_SUSTAINING_MAX_AGE_MS
                });
        if !has_transport_await_attempt_context {
            return false;
        }
        if (now_ms - submitted_at_ms).max(0.0) > TRANSPORT_AWAIT_RECOVERY_SUSTAINING_MAX_AGE_MS {
            return false;
        }
        if stats.transport_state != XbxEngineTransportStateDto::Connected {
            return false;
        }
        if stats.video_renderer_stalled.unwrap_or(false) {
            return false;
        }
        let track_attached_with_video =
            stats
                .latest_video_track_status
                .as_ref()
                .is_some_and(|track| {
                    track.state == "remoteTrackAttached" && track.video_bytes_total > 0
                });
        if !track_attached_with_video {
            return false;
        }
        if Self::transport_await_has_recent_unusable_nonidr_keyframe_response(
            stats,
            recovery_epoch,
            now_ms,
            None,
        ) {
            return false;
        }
        let decode_progress_after_submit = stats
            .latest_video_decode_ok_time_ms
            .is_some_and(|decoded_at_ms| decoded_at_ms > submitted_at_ms);
        let present_progress_after_submit = stats
            .latest_video_host_present_time_ms
            .is_some_and(|presented_at_ms| presented_at_ms > submitted_at_ms);
        let latest_output_progress_at_ms = [
            stats.latest_video_decode_ok_time_ms,
            stats.latest_video_host_present_time_ms,
        ]
        .into_iter()
        .flatten()
        .max_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
        let timeline_progress_visible = stats
            .latest_video_timeline_observation
            .as_ref()
            .is_some_and(|timeline| {
                (now_ms - timeline.observed_at_ms).max(0.0)
                    <= TRANSPORT_AWAIT_RECOVERY_SUSTAINING_PROGRESS_MAX_AGE_MS
                    && matches!(
                        timeline.source_event.as_str(),
                        "frame-complete-candidate"
                            | "frame-observed"
                            | "gap-repair-in-flight"
                            | "gap-resolved"
                            | "gap-reorder-pending"
                    )
            });
        let output_progress_recent = latest_output_progress_at_ms.is_some_and(|latest_at_ms| {
            latest_at_ms > submitted_at_ms
                && (now_ms - latest_at_ms).max(0.0)
                    <= TRANSPORT_AWAIT_RECOVERY_SUSTAINING_PROGRESS_MAX_AGE_MS
        });
        (decode_progress_after_submit || present_progress_after_submit || timeline_progress_visible)
            && (output_progress_recent || timeline_progress_visible)
    }

    fn transport_await_terminal_deferred_episode(
        stats: &XbxEngineMediaRuntimeStats,
        recovery_epoch: u64,
        now_ms: f64,
    ) -> bool {
        let has_clean_anchor_evidence = Self::has_recent_clean_anchor_evidence(
            stats.video_anchor_clean_epoch,
            stats.video_anchor_clean_observed_at_ms,
            stats.video_anchor_clean_source_event.as_deref(),
            stats.latest_anchor_candidate_ledger.as_ref(),
            recovery_epoch,
            now_ms,
        );
        stats
            .latest_keyframe_request_episode
            .as_ref()
            .is_some_and(|episode| {
                is_terminal_transport_await_deferred_episode(
                    episode,
                    stats.latest_h264_inspection_observation.as_ref(),
                    has_clean_anchor_evidence,
                    now_ms,
                    TRANSPORT_AWAIT_INVALID_KEYFRAME_RESPONSE_FRESH_MS,
                )
            })
    }

    fn transport_await_has_recent_invalid_bootstrap_for_episode(
        stats: &XbxEngineMediaRuntimeStats,
        episode: &crate::XbxEngineKeyframeRequestEpisodeObservation,
        now_ms: f64,
    ) -> bool {
        let Some(inspection) = stats.latest_h264_inspection_observation.as_ref() else {
            return false;
        };
        if (now_ms - inspection.observed_at_ms).max(0.0)
            > TRANSPORT_AWAIT_INVALID_KEYFRAME_RESPONSE_FRESH_MS
        {
            return false;
        }
        if !inspection_has_invalid_recovery_bootstrap(inspection) {
            return false;
        }
        match (
            episode.response_rtp_timestamp,
            inspection.frame_rtp_timestamp,
        ) {
            (Some(response_ts), Some(frame_ts)) => frame_ts == response_ts,
            _ => inspection.observed_at_ms >= episode.requested_at_ms,
        }
    }

    fn transport_await_packet_seen_without_decode_failure(
        stats: &XbxEngineMediaRuntimeStats,
        recovery_epoch: u64,
        now_ms: f64,
    ) -> bool {
        if Self::transport_await_has_recent_unusable_nonidr_keyframe_response(
            stats,
            recovery_epoch,
            now_ms,
            None,
        ) {
            return true;
        }
        let Some(episode) = stats.latest_keyframe_request_episode.as_ref() else {
            return false;
        };
        if episode.request_reason.as_deref() != Some("transportAwaitRecoveryKeyframe") {
            return false;
        }
        if episode.status != "packet-seen" || episode.first_keyframe_decoded_at_ms.is_some() {
            return false;
        }
        let Some(first_packet_at_ms) = episode.first_keyframe_packet_at_ms else {
            return false;
        };
        if (now_ms - first_packet_at_ms).max(0.0) < TRANSPORT_AWAIT_KEYFRAME_PACKET_ONLY_GRACE_MS {
            return false;
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
        unresolved_transport_await
            && lacks_recent_clean_anchor
            && !Self::transport_await_objective_media_healthy(stats, now_ms)
    }

    fn transport_await_keyframe_attempt_still_blocks_retry(
        stats: &XbxEngineMediaRuntimeStats,
        episode: &crate::XbxEngineKeyframeRequestEpisodeObservation,
        recovery_epoch: u64,
        now_ms: f64,
    ) -> bool {
        if episode.request_reason.as_deref() != Some("transportAwaitRecoveryKeyframe") {
            return false;
        }
        match episode.status.as_str() {
            "requested" => {
                episode.sent_at_ms.is_none()
                    && matches!(episode.response_verdict.as_deref(), None | Some("pending"))
                    && (now_ms - episode.requested_at_ms).max(0.0)
                        <= UNSENT_KEYFRAME_REQUEST_GRACE_MS
            }
            "sent" => {
                episode.sent_at_ms.is_some()
                    && matches!(episode.response_verdict.as_deref(), None | Some("pending"))
                    && episode.deadline_at_ms.unwrap_or(now_ms + 1.0) >= now_ms
            }
            "packet-seen" => episode
                .first_keyframe_packet_at_ms
                .is_some_and(|packet_at_ms| {
                    (now_ms - packet_at_ms).max(0.0)
                        <= TRANSPORT_AWAIT_KEYFRAME_PACKET_ONLY_GRACE_MS
                        && !Self::transport_await_packet_seen_without_decode_failure(
                            stats,
                            recovery_epoch,
                            now_ms,
                        )
                }),
            "decoded" => episode
                .first_keyframe_decoded_at_ms
                .is_some_and(|decoded_at_ms| {
                    (now_ms - decoded_at_ms).max(0.0) <= TRANSPORT_AWAIT_KEYFRAME_DECODED_GRACE_MS
                }),
            _ => false,
        }
    }

    fn transport_await_has_recent_unusable_nonidr_keyframe_response(
        stats: &XbxEngineMediaRuntimeStats,
        recovery_epoch: u64,
        now_ms: f64,
        attempt_started_at_ms: Option<f64>,
    ) -> bool {
        const TRANSPORT_AWAIT_UNUSABLE_RESPONSE_FRESH_MS: f64 = 1_500.0;
        if Self::transport_await_pre_first_frame_availability_active_from_stats(stats, now_ms) {
            return false;
        }
        let Some(episode) = stats.latest_keyframe_request_episode.as_ref() else {
            return false;
        };
        if episode.request_reason.as_deref() != Some("transportAwaitRecoveryKeyframe") {
            return false;
        }
        if !matches!(episode.status.as_str(), "packet-seen" | "decoded") {
            return false;
        }
        let Some(response_at_ms) = episode
            .first_keyframe_decoded_at_ms
            .or(episode.first_keyframe_packet_at_ms)
        else {
            return false;
        };
        if attempt_started_at_ms.is_some_and(|started_at_ms| response_at_ms < started_at_ms) {
            return false;
        }
        if (now_ms - response_at_ms).max(0.0) > TRANSPORT_AWAIT_UNUSABLE_RESPONSE_FRESH_MS {
            return false;
        }
        let Some(inspection) = stats.latest_h264_inspection_observation.as_ref() else {
            return false;
        };
        if (now_ms - inspection.observed_at_ms).max(0.0)
            > TRANSPORT_AWAIT_UNUSABLE_RESPONSE_FRESH_MS
        {
            return false;
        }
        let reject_reason_unusable = matches!(
            inspection.bootstrap_reject_reason.as_deref(),
            Some(
                "NonIdrVcl"
                    | "bootstrapMissingSps"
                    | "bootstrapMissingPps"
                    | "inspectionRejectInvalidSliceHeader"
            )
        );
        let metadata_unusable = !inspection.committed_sps_present
            || !inspection.committed_pps_present
            || !inspection.delta_continuation_ready;
        if inspection.bootstrap_ready
            || inspection.is_idr
            || !(reject_reason_unusable || metadata_unusable)
        {
            return false;
        }
        let track_attached_with_video =
            stats
                .latest_video_track_status
                .as_ref()
                .is_some_and(|track| {
                    track.state == "remoteTrackAttached" && track.video_bytes_total > 0
                });
        if !track_attached_with_video {
            return false;
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
        unresolved_transport_await && lacks_recent_clean_anchor
    }

    fn release_stale_transport_await_keyframe_family(
        &mut self,
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        recovery_epoch: u64,
        now_ms: f64,
    ) {
        let should_release = RuntimeStatsSink::read_shared(runtime_stats, |stats| {
            let Some(episode) = stats.latest_keyframe_request_episode.as_ref() else {
                return false;
            };
            episode.request_reason.as_deref() == Some("transportAwaitRecoveryKeyframe")
                && !Self::transport_await_keyframe_attempt_still_blocks_retry(
                    stats,
                    episode,
                    recovery_epoch,
                    now_ms,
                )
        })
        .unwrap_or(false);
        if should_release {
            self.escalation_controller.reset_keyframe_epoch();
        }
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
            if Self::transport_await_has_recent_unusable_nonidr_keyframe_response(
                stats,
                stats.transport_recovery_epoch,
                now_ms,
                None,
            ) {
                return false;
            }
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
        let Some(timeline) = stats.latest_video_timeline_observation.as_ref() else {
            return false;
        };
        has_current_transport_await_issue_from_observation(
            timeline,
            current_clean_anchor_observed_at_ms(
                stats.video_anchor_clean_epoch,
                stats.video_anchor_clean_observed_at_ms,
                stats.video_anchor_clean_source_event.as_deref(),
                stats.transport_recovery_epoch,
            ),
        )
    }

    fn transport_await_ingress_still_waiting(
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        recovery_epoch: u64,
        now_ms: f64,
    ) -> bool {
        RuntimeStatsSink::read_shared(runtime_stats, |stats| {
            if stats.transport_recovery_epoch != recovery_epoch {
                return false;
            }
            let timeline_waiting = Self::has_unresolved_transport_await_issue(stats);
            if !timeline_waiting {
                return false;
            }
            let last_waiting_at_ms = stats
                .latest_video_timeline_observation
                .as_ref()
                .map(|timeline| timeline.observed_at_ms)
                .unwrap_or(now_ms);
            (now_ms - last_waiting_at_ms).max(0.0) <= 1_200.0
        })
        .unwrap_or(false)
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

    fn should_absorb_connectivity_jitter(
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        reason: VideoEscalationReason,
        observed_at_ms: f64,
    ) -> bool {
        if !matches!(
            reason,
            VideoEscalationReason::TransportExpiredDeadline
                | VideoEscalationReason::TransportSevereDeadline
                | VideoEscalationReason::TransportRecoveredLate
                | VideoEscalationReason::TransportSampleLoss
        ) {
            return false;
        }
        RuntimeStatsSink::read_shared(runtime_stats, |stats| {
            if stats.transport_state != XbxEngineTransportStateDto::Connected {
                return false;
            }
            let track_attached_with_video =
                stats
                    .latest_video_track_status
                    .as_ref()
                    .is_some_and(|track| {
                        track.state == "remoteTrackAttached" && track.video_bytes_total > 0
                    });
            if !track_attached_with_video {
                return false;
            }
            let present_age_ms = stats
                .latest_video_host_present_time_ms
                .map(|at_ms| (observed_at_ms - at_ms).max(0.0))
                .unwrap_or(f64::INFINITY);
            let pipeline_not_stalled = !stats.video_decoder_stalled.unwrap_or(false)
                && !stats.video_renderer_stalled.unwrap_or(false);
            track_attached_with_video
                && pipeline_not_stalled
                && present_age_ms <= CONNECTIVITY_JITTER_ABSORB_PRESENT_AGE_MAX_MS
                && has_fresh_media_output(stats, observed_at_ms)
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
                .suppressed(RecoveryAction::RequestKeyframe),
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
                        VideoEscalationReason::DecoderBackendFailure.label(),
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
                    reason.label(),
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
    fn is_recovery_sustaining_reason_label(reason_label: &str) -> bool {
        reason_label == "recoverySustaining"
    }

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

    /// `on_reason_with_epoch_policy` 已推进 burst 计数后，若 coordinator 将动作压成等待/合并，
    /// 应回滚计数，避免“未执行仍吃 burst”。
    ///
    /// 注意：不把 `CooldownSuppressed` 纳入回滚——该结果常来自 soft reentry 等后处理，
    /// 回滚会破坏连续 `propose` 下 escalation 累积状态，导致无法稳定得到 `CoalescedKeyframeInFlight`。
    fn coordinator_burst_rollback_warranted(
        naive: RecoveryAction,
        final_action: RecoveryAction,
    ) -> bool {
        matches!(
            (naive, final_action),
            (
                RecoveryAction::RequestKeyframe,
                RecoveryAction::WaitForBurst
            ) | (
                RecoveryAction::RequestKeyframe,
                RecoveryAction::CoalescedDecoderResetInFlight
            ) | (
                RecoveryAction::RequestDecoderReset,
                RecoveryAction::CoalescedDecoderResetInFlight
            ) | (
                RecoveryAction::CoalescedKeyframeInFlight,
                RecoveryAction::CoalescedDecoderResetInFlight
            ) | (
                RecoveryAction::CoalescedKeyframeInFlight,
                RecoveryAction::WaitForBurst
            )
        )
    }
}

// RFC：与 `session::control_model::resolve_session_fault_domain` 单源对齐后再映射到 coordinator 内部桶。
fn fault_domain_to_recovery_signal_domain(domain: SessionFaultDomain) -> RecoverySignalDomain {
    match domain {
        SessionFaultDomain::Transport => RecoverySignalDomain::Connectivity,
        SessionFaultDomain::ReferenceChain => RecoverySignalDomain::MediaRecovery,
        SessionFaultDomain::DecodePipeline | SessionFaultDomain::DisplaySupply => {
            RecoverySignalDomain::Local
        }
    }
}

fn classify_signal_domain(reason: VideoEscalationReason) -> RecoverySignalDomain {
    fault_domain_to_recovery_signal_domain(resolve_session_fault_domain(reason))
}

#[cfg(test)]
#[path = "coordinator.test.rs"]
mod tests;
