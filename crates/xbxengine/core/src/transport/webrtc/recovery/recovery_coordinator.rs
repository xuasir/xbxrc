use std::sync::Mutex;
use std::time::{Duration, Instant};

use xbxengine_protocol::XbxEngineTransportStateDto;

use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::webrtc::escalation::{
    RecoveryAction, VideoEscalationController, VideoEscalationDecision, VideoEscalationReason,
};
use crate::transport::webrtc::policy::{RecoveryScenarioProfile, ScenarioPolicyResolver};
use crate::transport::webrtc::recovery::recovery_diagnosis::VideoRecoveryDiagnosis;
use crate::transport::webrtc::recovery::recovery_signal::{
    VideoIngressSignal, VideoRecoverySignal,
};
use crate::transport::webrtc::startup_recovery::{
    extract_startup_recovery_bitrate_kbps, resolve_session_phase,
    should_fast_reset_startup_recovery, should_suppress_startup_escalation, SessionPhase,
    StartupRecoveryProbe,
};
use crate::XbxEngineMediaRuntimeStats;

const RECENT_NACK_OUTCOME_WINDOW_MS: f64 = 180.0;
const RECENT_NACK_OUTCOME_WINDOW_MS_CLOUD: f64 = 520.0;
const CLOUD_STARTUP_NACK_BUDGET_WINDOW_MS: f64 = 1_200.0;
const CLOUD_STARTUP_NACK_BUDGET_THRESHOLD: u8 = 3;
const WAIT_KEYFRAME_REPEAT_SUPPRESS_MS: f64 = 260.0;
const IDLE_TIMEOUT_REPEAT_SUPPRESS_MS: f64 = 360.0;
const HARD_STALL_DECODER_RESET_MS: f64 = 1_200.0;
const HARD_STALL_RECONNECT_MS: f64 = 3_000.0;
const HARD_STALL_MIN_RESET_SPACING_MS: f64 = 1_200.0;
const HARD_STALL_MIN_RECONNECT_SPACING_MS: f64 = 1_800.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RecoveryCouplingMode {
    Healthy,
    StartupLowQuality,
    WaitingKeyframe,
    RecoveringReferenceChain,
    Stalled,
    ThinStream,
}

impl RecoveryCouplingMode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            RecoveryCouplingMode::Healthy => "healthy",
            RecoveryCouplingMode::StartupLowQuality => "startupLowQuality",
            RecoveryCouplingMode::WaitingKeyframe => "waitingKeyframe",
            RecoveryCouplingMode::RecoveringReferenceChain => "recoveringReferenceChain",
            RecoveryCouplingMode::Stalled => "stalled",
            RecoveryCouplingMode::ThinStream => "thinStream",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct RecoveryCouplingState {
    pub(crate) mode: RecoveryCouplingMode,
    pub(crate) suppress_ramp_up: bool,
    pub(crate) prefer_hold: bool,
    pub(crate) allow_peak_range: bool,
}

impl RecoveryCouplingState {
    pub(crate) fn summary(self) -> String {
        format!(
            "{}:{}:{}:{}",
            self.mode.as_str(),
            if self.suppress_ramp_up {
                "suppressRampUp"
            } else {
                "allowRampUp"
            },
            if self.prefer_hold {
                "preferHold"
            } else {
                "allowAdvance"
            },
            if self.allow_peak_range {
                "allowPeak"
            } else {
                "capPeak"
            },
        )
    }
}

#[derive(Clone, Debug)]
pub(crate) struct RecoveryRuntimeState {
    pub(crate) phase: SessionPhase,
    pub(crate) recovery_policy_profile: &'static str,
    pub(crate) diagnosis_label: String,
    pub(crate) coupling: RecoveryCouplingState,
}

pub(crate) struct RecoveryDispatch {
    pub(crate) runtime_state: RecoveryRuntimeState,
    pub(crate) decision: VideoEscalationDecision,
}

fn resolve_recovery_profile(
    runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
) -> RecoveryScenarioProfile {
    let (session_target_type, transport_path) =
        RuntimeStatsSink::read_shared(runtime_stats, |stats| {
            (
                stats.session_target_type.clone(),
                stats.transport_path.clone(),
            )
        })
        .unwrap_or((None, None));
    ScenarioPolicyResolver::resolve_recovery_profile(
        session_target_type.as_ref(),
        transport_path.as_deref(),
    )
}

/**
 * 统一承接 startup/recovery 的局部状态：
 * - `stack` 只负责喂事件和执行动作
 * - startup grace / fast-reset / low-quality probe 不再散落在事件循环里
 */
pub struct RecoveryCoordinator {
    escalation_controller: VideoEscalationController,
    startup_probe: StartupRecoveryProbe,
    stream_started_at: Instant,
    startup_grace: Duration,
    cloud_startup_expired_deadline_first_seen_at_ms: Option<f64>,
    cloud_startup_expired_deadline_last_observation_id: Option<u64>,
    cloud_startup_expired_deadline_streak: u8,
}

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
            cloud_startup_expired_deadline_first_seen_at_ms: None,
            cloud_startup_expired_deadline_last_observation_id: None,
            cloud_startup_expired_deadline_streak: 0,
        }
    }

    pub fn on_reason_with_runtime_stats(
        &mut self,
        reason: VideoEscalationReason,
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
    ) -> VideoEscalationDecision {
        if let Some(decision) =
            self.resolve_decoder_backend_failure_recovery(runtime_stats, &reason)
        {
            return decision;
        }
        if let Some(decision) = self.resolve_persistent_stall_recovery(runtime_stats, &reason) {
            return decision;
        }
        if let Some(decision) = self.resolve_recent_repeat_suppression(runtime_stats, &reason) {
            return decision;
        }
        if let Some(decision) = self.resolve_recent_nack_outcome(runtime_stats, &reason) {
            return decision;
        }
        let phase =
            resolve_session_phase(runtime_stats, self.stream_started_at, self.startup_grace);
        let profile = resolve_recovery_profile(runtime_stats);
        self.on_reason_with_policy(reason, phase, profile)
    }

    fn on_reason_with_policy(
        &mut self,
        reason: VideoEscalationReason,
        phase: SessionPhase,
        profile: RecoveryScenarioProfile,
    ) -> VideoEscalationDecision {
        let startup_fast_reset = profile.startup_fast_reset_enabled
            && phase == SessionPhase::Startup
            && should_fast_reset_startup_recovery(
                &reason,
                self.stream_started_at,
                self.startup_grace,
            );
        let escalation_decision = if phase == SessionPhase::Startup
            && should_suppress_startup_escalation(
                &reason,
                self.stream_started_at,
                self.startup_grace,
            ) {
            self.escalation_controller
                .suppressed(RecoveryAction::StartupGraceSuppressed)
        } else {
            self.escalation_controller.on_reason(reason)
        };
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

    pub(crate) fn current_profile_name(
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
    ) -> &'static str {
        resolve_recovery_profile(runtime_stats).kind.as_str()
    }

    pub(crate) fn runtime_state_for_diagnosis(
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        diagnosis_label: &str,
        stream_started_at: Instant,
        startup_grace: Duration,
    ) -> RecoveryRuntimeState {
        let phase = resolve_session_phase(runtime_stats, stream_started_at, startup_grace);
        let diagnosis_label =
            Self::resolve_effective_diagnosis_label(runtime_stats, diagnosis_label);
        RecoveryRuntimeState {
            phase,
            recovery_policy_profile: Self::current_profile_name(runtime_stats),
            diagnosis_label,
            coupling: Self::current_coupling_state(runtime_stats, stream_started_at, startup_grace),
        }
    }

    pub(crate) fn runtime_state_for_label(
        &self,
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        diagnosis_label: &str,
    ) -> RecoveryRuntimeState {
        Self::runtime_state_for_diagnosis(
            runtime_stats,
            diagnosis_label,
            self.stream_started_at,
            self.startup_grace,
        )
    }

    pub(crate) fn on_transport_signal_with_runtime_stats(
        &mut self,
        signal: VideoRecoverySignal,
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
    ) -> RecoveryDispatch {
        self.dispatch_diagnosis(signal.diagnose(), runtime_stats)
    }

    pub(crate) fn on_ingress_signal_with_runtime_stats(
        &mut self,
        signal: VideoIngressSignal,
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
    ) -> RecoveryDispatch {
        self.dispatch_diagnosis(signal.diagnose(), runtime_stats)
    }

    fn dispatch_diagnosis(
        &mut self,
        diagnosis: VideoRecoveryDiagnosis,
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
    ) -> RecoveryDispatch {
        let diagnosis_label = self
            .runtime_state_for_label(runtime_stats, diagnosis.label)
            .diagnosis_label;
        let decision = self.on_reason_with_runtime_stats(diagnosis.reason, runtime_stats);
        let runtime_state = self.runtime_state_for_label(runtime_stats, diagnosis_label.as_str());
        RecoveryDispatch {
            runtime_state,
            decision,
        }
    }

    fn resolve_effective_diagnosis_label(
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        diagnosis_label: &str,
    ) -> String {
        if !matches!(
            diagnosis_label,
            "transportExpiredDeadline"
                | "transportSevereDeadline"
                | "transportSampleLoss"
                | "adapterIdleTimeout"
                | "transportAwaitRecoveryKeyframe"
                | "ingressWaitKeyframe"
        ) {
            return diagnosis_label.to_string();
        }
        let now_ms = unix_now_ms();
        RuntimeStatsSink::read_shared(runtime_stats, |stats| {
            let profile = ScenarioPolicyResolver::resolve_recovery_profile(
                stats.session_target_type.as_ref(),
                stats.transport_path.as_deref(),
            );
            if decoder_backend_failure_signal_is_active(stats, profile, now_ms) {
                "decoderBackendFailure".to_string()
            } else {
                diagnosis_label.to_string()
            }
        })
        .unwrap_or_else(|| diagnosis_label.to_string())
    }

    pub(crate) fn current_coupling_state(
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        stream_started_at: Instant,
        startup_grace: Duration,
    ) -> RecoveryCouplingState {
        let phase = resolve_session_phase(runtime_stats, stream_started_at, startup_grace);
        resolve_recovery_coupling_state(runtime_stats, phase)
    }

    fn resolve_decoder_backend_failure_recovery(
        &mut self,
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        reason: &VideoEscalationReason,
    ) -> Option<VideoEscalationDecision> {
        if !matches!(
            reason,
            VideoEscalationReason::TransportExpiredDeadline
                | VideoEscalationReason::TransportSevereDeadline
                | VideoEscalationReason::TransportSampleLoss
                | VideoEscalationReason::TransportAwaitRecoveryKeyframe
                | VideoEscalationReason::WaitKeyframe
                | VideoEscalationReason::AdapterIdleTimeout
        ) {
            return None;
        }

        let now_ms = unix_now_ms();
        let Some((profile, since_last_reset_ms)) =
            RuntimeStatsSink::read_shared(runtime_stats, |stats| {
                let profile = ScenarioPolicyResolver::resolve_recovery_profile(
                    stats.session_target_type.as_ref(),
                    stats.transport_path.as_deref(),
                );
                if !decoder_backend_failure_signal_is_active(stats, profile, now_ms) {
                    return None;
                }
                let since_last_reset_ms = stats
                    .latest_video_decoder_reset_time_ms
                    .map(|at_ms| (now_ms - at_ms).max(0.0))
                    .unwrap_or(f64::INFINITY);
                Some((profile, since_last_reset_ms))
            })
            .flatten()
        else {
            return None;
        };

        if since_last_reset_ms < profile.decoder_backend_failure_min_reset_spacing_ms {
            return Some(
                self.escalation_controller
                    .suppressed(RecoveryAction::CooldownSuppressed),
            );
        }

        let phase =
            resolve_session_phase(runtime_stats, self.stream_started_at, self.startup_grace);
        Some(self.on_reason_with_policy(
            VideoEscalationReason::DecoderBackendFailure,
            phase,
            profile,
        ))
    }

    // 优先消费最近一次 NACK outcome：
    // - 已追回：不要立刻把恢复继续升级
    // - 已过期：只对关键/reference 帧升级；delta 直接放弃，避免拖慢后续帧
    fn resolve_recent_nack_outcome(
        &mut self,
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        reason: &VideoEscalationReason,
    ) -> Option<VideoEscalationDecision> {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis() as f64)
            .unwrap_or(0.0);
        let Some((nack, stalled_with_fresh_packets)) =
            RuntimeStatsSink::read_shared(runtime_stats, |stats| {
                let nack = stats.latest_video_nack_observation.clone()?;
                let recent_window_ms = if matches!(
                    stats.session_target_type,
                    Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud)
                ) || matches!(
                    stats.transport_policy_profile.as_deref(),
                    Some("cloudGaming")
                ) {
                    RECENT_NACK_OUTCOME_WINDOW_MS_CLOUD
                } else {
                    RECENT_NACK_OUTCOME_WINDOW_MS
                };
                if now_ms - nack.observed_at_ms > recent_window_ms {
                    return None;
                }
                let present_age_ms = stats
                    .latest_video_present_time_ms
                    .map(|at_ms| (now_ms - at_ms).max(0.0))
                    .unwrap_or(f64::INFINITY);
                let packet_age_ms = stats
                    .latest_video_packet_arrival_time_ms
                    .map(|at_ms| (now_ms - at_ms).max(0.0))
                    .unwrap_or(f64::INFINITY);
                let stalled_with_fresh_packets = stats.video_renderer_stalled.unwrap_or(false)
                    && present_age_ms >= HARD_STALL_DECODER_RESET_MS
                    && packet_age_ms <= HARD_STALL_DECODER_RESET_MS;
                Some((nack, stalled_with_fresh_packets))
            })
            .flatten()
        else {
            return None;
        };

        let is_delta = nack.frame_is_keyframe == Some(false)
            && matches!(nack.frame_importance.as_deref(), Some("delta"));
        let is_important = nack.frame_is_keyframe == Some(true)
            || matches!(
                nack.frame_importance.as_deref(),
                Some("reference" | "keyframe")
            );
        let is_cloud_startup = RuntimeStatsSink::read_shared(runtime_stats, |stats| {
            matches!(
                stats.session_target_type,
                Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud)
            ) || matches!(
                stats.transport_policy_profile.as_deref(),
                Some("cloudGaming")
            )
        })
        .unwrap_or(false)
            && matches!(
                resolve_session_phase(runtime_stats, self.stream_started_at, self.startup_grace),
                SessionPhase::Startup
            );

        match nack.action.as_str() {
            "recovered" | "recoveredLate" => {
                self.clear_cloud_startup_expired_deadline_budget();
                if is_important
                    && matches!(
                        reason,
                        VideoEscalationReason::WaitKeyframe
                            | VideoEscalationReason::TransportAwaitRecoveryKeyframe
                            | VideoEscalationReason::AdapterIdleTimeout
                    )
                {
                    Some(
                        self.escalation_controller
                            .suppressed(RecoveryAction::WaitForBurst),
                    )
                } else {
                    Some(
                        self.escalation_controller
                            .suppressed(RecoveryAction::CooldownSuppressed),
                    )
                }
            }
            "expiredDeadline" | "expiredMaxAge" => {
                if is_delta {
                    let profile = resolve_recovery_profile(runtime_stats);
                    if is_cloud_startup
                        && matches!(reason, VideoEscalationReason::TransportExpiredDeadline)
                    {
                        if self.update_cloud_startup_expired_deadline_budget(&nack) {
                            let phase = resolve_session_phase(
                                runtime_stats,
                                self.stream_started_at,
                                self.startup_grace,
                            );
                            return Some(self.on_reason_with_policy(
                                VideoEscalationReason::TransportExpiredDeadline,
                                phase,
                                profile,
                            ));
                        }
                        return Some(
                            self.escalation_controller
                                .suppressed(RecoveryAction::CooldownSuppressed),
                        );
                    }
                    if stalled_with_fresh_packets
                        && matches!(
                            reason,
                            VideoEscalationReason::TransportExpiredDeadline
                                | VideoEscalationReason::TransportSampleLoss
                                | VideoEscalationReason::WaitKeyframe
                                | VideoEscalationReason::AdapterIdleTimeout
                        )
                    {
                        let phase = resolve_session_phase(
                            runtime_stats,
                            self.stream_started_at,
                            self.startup_grace,
                        );
                        let profile = resolve_recovery_profile(runtime_stats);
                        return Some(self.on_reason_with_policy(
                            VideoEscalationReason::TransportAwaitRecoveryKeyframe,
                            phase,
                            profile,
                        ));
                    }
                    Some(
                        self.escalation_controller
                            .suppressed(RecoveryAction::CooldownSuppressed),
                    )
                } else if is_important
                    && matches!(
                        reason,
                        VideoEscalationReason::AdapterIdleTimeout
                            | VideoEscalationReason::TransportSampleLoss
                            | VideoEscalationReason::TransportAwaitRecoveryKeyframe
                            | VideoEscalationReason::WaitKeyframe
                    )
                {
                    let phase = resolve_session_phase(
                        runtime_stats,
                        self.stream_started_at,
                        self.startup_grace,
                    );
                    let profile = resolve_recovery_profile(runtime_stats);
                    let escalated_reason =
                        if matches!(reason, VideoEscalationReason::AdapterIdleTimeout) {
                            VideoEscalationReason::TransportAwaitRecoveryKeyframe
                        } else {
                            VideoEscalationReason::TransportSampleLoss
                        };
                    Some(self.on_reason_with_policy(escalated_reason, phase, profile))
                } else if is_important
                    && matches!(
                        reason,
                        VideoEscalationReason::TransportSampleLoss
                            | VideoEscalationReason::TransportAwaitRecoveryKeyframe
                            | VideoEscalationReason::WaitKeyframe
                    )
                {
                    let phase = resolve_session_phase(
                        runtime_stats,
                        self.stream_started_at,
                        self.startup_grace,
                    );
                    let profile = resolve_recovery_profile(runtime_stats);
                    Some(self.on_reason_with_policy(
                        VideoEscalationReason::TransportSampleLoss,
                        phase,
                        profile,
                    ))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn update_cloud_startup_expired_deadline_budget(
        &mut self,
        nack: &crate::XbxEngineVideoNackObservation,
    ) -> bool {
        let current_observation_id = nack.observation_id;
        if self.cloud_startup_expired_deadline_last_observation_id == Some(current_observation_id) {
            return self.cloud_startup_expired_deadline_streak
                >= CLOUD_STARTUP_NACK_BUDGET_THRESHOLD;
        }

        let now_ms = nack.observed_at_ms.max(unix_now_ms());
        let same_window = self
            .cloud_startup_expired_deadline_first_seen_at_ms
            .map(|first_seen_at_ms| {
                now_ms - first_seen_at_ms <= CLOUD_STARTUP_NACK_BUDGET_WINDOW_MS
            })
            .unwrap_or(false);
        if same_window {
            self.cloud_startup_expired_deadline_streak = self
                .cloud_startup_expired_deadline_streak
                .saturating_add(1)
                .max(1);
        } else {
            self.cloud_startup_expired_deadline_first_seen_at_ms = Some(now_ms);
            self.cloud_startup_expired_deadline_streak = 1;
        }
        self.cloud_startup_expired_deadline_last_observation_id = Some(current_observation_id);
        self.cloud_startup_expired_deadline_streak >= CLOUD_STARTUP_NACK_BUDGET_THRESHOLD
    }

    fn clear_cloud_startup_expired_deadline_budget(&mut self) {
        self.cloud_startup_expired_deadline_first_seen_at_ms = None;
        self.cloud_startup_expired_deadline_last_observation_id = None;
        self.cloud_startup_expired_deadline_streak = 0;
    }

    // 已经进入同一轮恢复时，短窗口内抑制重复 reason：
    // - WaitKeyframe 在刚发过 keyframe/reset 后，先别继续一帧一帧推高恢复动作
    // - AdapterIdleTimeout 在刚发过 decoder reset 后，先观察这一轮恢复是否生效
    fn resolve_recent_repeat_suppression(
        &mut self,
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        reason: &VideoEscalationReason,
    ) -> Option<VideoEscalationDecision> {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis() as f64)
            .unwrap_or(0.0);
        let (escalation, has_new_transport_recovery_epoch) =
            RuntimeStatsSink::read_shared(runtime_stats, |stats| {
                let escalation = stats.latest_video_escalation_observation.clone()?;
                let has_new_transport_recovery_epoch = stats.transport_recovery_epoch
                    > stats.transport_recovery_epoch_at_last_escalation;
                Some((escalation, has_new_transport_recovery_epoch))
            })
            .flatten()?;
        let elapsed_ms = now_ms - escalation.observed_at_ms;
        if elapsed_ms < 0.0 {
            return None;
        }

        match reason {
            VideoEscalationReason::WaitKeyframe => {
                let same_wait_keyframe_chain = matches!(
                    escalation.reason.as_str(),
                    "waitKeyframe" | "ingressWaitKeyframe" | "transportAwaitRecoveryKeyframe"
                );
                let active_recovery_action = matches!(
                    escalation.action.as_str(),
                    "requestKeyframe"
                        | "requestDecoderReset"
                        | "requestKeyframe+decoderReset"
                        | "requestKeyframe+decoderReset(startupLowQualityRetry)"
                );
                if same_wait_keyframe_chain
                    && active_recovery_action
                    && !has_new_transport_recovery_epoch
                    && elapsed_ms <= WAIT_KEYFRAME_REPEAT_SUPPRESS_MS
                {
                    return Some(
                        self.escalation_controller
                            .suppressed(RecoveryAction::CooldownSuppressed),
                    );
                }
            }
            VideoEscalationReason::AdapterIdleTimeout => {
                let same_idle_chain = escalation.reason == "adapterIdleTimeout";
                let decoder_reset_inflight = matches!(
                    escalation.action.as_str(),
                    "requestDecoderReset"
                        | "requestKeyframe+decoderReset"
                        | "requestKeyframe+decoderReset(startupLowQualityRetry)"
                );
                if same_idle_chain
                    && decoder_reset_inflight
                    && !has_new_transport_recovery_epoch
                    && elapsed_ms <= IDLE_TIMEOUT_REPEAT_SUPPRESS_MS
                {
                    return Some(
                        self.escalation_controller
                            .suppressed(RecoveryAction::CooldownSuppressed),
                    );
                }
            }
            _ => {}
        }

        None
    }

    // 当视频已经长时间 0kbps 且没有任何新呈现时，不允许一直停在 cooldownSuppressed。
    // 这条链只依赖“当前已进入硬停滞事实”，不能再绑定单个 diagnosis label，
    // 否则 transportExpiredDeadline / severe deadline 会卡在 cooldownSuppressed 而无法升级。
    fn resolve_persistent_stall_recovery(
        &mut self,
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        reason: &VideoEscalationReason,
    ) -> Option<VideoEscalationDecision> {
        if !matches!(
            reason,
            VideoEscalationReason::AdapterIdleTimeout
                | VideoEscalationReason::TransportExpiredDeadline
                | VideoEscalationReason::TransportSevereDeadline
        ) {
            return None;
        }

        let now_ms = unix_now_ms();
        let Some(stats) = RuntimeStatsSink::read_shared(runtime_stats, |stats| {
            (
                stats.transport_state.clone(),
                stats.inbound_video_bitrate_kbps.unwrap_or(0.0),
                stats
                    .latest_video_present_time_ms
                    .map(|at_ms| (now_ms - at_ms).max(0.0))
                    .unwrap_or(HARD_STALL_RECONNECT_MS),
                stats
                    .latest_video_packet_arrival_time_ms
                    .map(|at_ms| (now_ms - at_ms).max(0.0))
                    .unwrap_or(HARD_STALL_RECONNECT_MS),
                if stats.video_renderer_stalled.unwrap_or(false)
                    || stats
                        .latest_video_present_time_ms
                        .map(|at_ms| (now_ms - at_ms).max(0.0))
                        .unwrap_or(HARD_STALL_RECONNECT_MS)
                        >= HARD_STALL_DECODER_RESET_MS
                {
                    0.0
                } else {
                    stats.video_present_fps
                },
                stats.direct_gaming_bitrate_band.clone(),
                stats
                    .latest_video_escalation_observation
                    .as_ref()
                    .map(|observation| observation.action.as_str().to_string()),
                stats
                    .latest_video_decoder_reset_time_ms
                    .map(|at_ms| (now_ms - at_ms).max(0.0))
                    .unwrap_or(f64::INFINITY),
            )
        }) else {
            return None;
        };
        let (
            transport_state,
            inbound_video_bitrate_kbps,
            present_age_ms,
            packet_age_ms,
            effective_present_fps,
            direct_gaming_bitrate_band,
            latest_action,
            since_last_decoder_reset_ms,
        ) = stats;
        if transport_state != XbxEngineTransportStateDto::Connected {
            return None;
        }
        let hard_paused_stream = inbound_video_bitrate_kbps <= 0.1
            && direct_gaming_bitrate_band.as_deref() == Some("paused")
            && effective_present_fps <= 1.0
            && present_age_ms >= HARD_STALL_DECODER_RESET_MS
            && packet_age_ms >= HARD_STALL_DECODER_RESET_MS;
        if !hard_paused_stream {
            return None;
        }

        if present_age_ms >= HARD_STALL_RECONNECT_MS
            && packet_age_ms >= HARD_STALL_RECONNECT_MS
            && since_last_decoder_reset_ms >= HARD_STALL_MIN_RECONNECT_SPACING_MS
            && matches!(
                latest_action.as_deref(),
                Some(
                    "requestDecoderReset"
                        | "requestKeyframe+decoderReset"
                        | "requestKeyframe+decoderReset(startupLowQualityRetry)"
                        | "cooldownSuppressed"
                )
            )
        {
            return Some(VideoEscalationDecision {
                observation_id: 0,
                action: RecoveryAction::RequestReconnectCandidate,
            });
        }

        // transport stall 已经证明是“媒体链没法自救”，这里直接交给会话级重连，
        // 避免又回到 decoder reset 分支，把传输故障误当成解码故障。
        if matches!(
            reason,
            VideoEscalationReason::TransportExpiredDeadline
                | VideoEscalationReason::TransportSevereDeadline
        ) {
            return Some(VideoEscalationDecision {
                observation_id: 0,
                action: RecoveryAction::RequestReconnectCandidate,
            });
        }

        if since_last_decoder_reset_ms >= HARD_STALL_MIN_RESET_SPACING_MS {
            return Some(VideoEscalationDecision {
                observation_id: 0,
                action: RecoveryAction::RequestDecoderReset,
            });
        }

        None
    }
}

fn unix_now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0)
}

fn decoder_backend_failure_signal_is_active(
    stats: &XbxEngineMediaRuntimeStats,
    profile: RecoveryScenarioProfile,
    now_ms: f64,
) -> bool {
    if stats.transport_state != XbxEngineTransportStateDto::Connected {
        return false;
    }
    if stats.video_decoder_hardware_failure_streak
        < profile.decoder_backend_failure_min_consecutive_failures
    {
        return false;
    }
    let failure_age_ms = stats
        .latest_video_decoder_hardware_failure_time_ms
        .map(|at_ms| (now_ms - at_ms).max(0.0))
        .unwrap_or(f64::INFINITY);
    if failure_age_ms > profile.decoder_backend_failure_recent_window_ms {
        return false;
    }
    let packet_age_ms = stats
        .latest_video_packet_arrival_time_ms
        .map(|at_ms| (now_ms - at_ms).max(0.0))
        .unwrap_or(f64::INFINITY);
    if packet_age_ms > profile.decoder_backend_failure_max_packet_age_ms {
        return false;
    }
    let decode_age_ms = stats
        .latest_video_decode_ok_time_ms
        .map(|at_ms| (now_ms - at_ms).max(0.0))
        .unwrap_or(f64::INFINITY);
    let present_age_ms = stats
        .latest_video_present_time_ms
        .map(|at_ms| (now_ms - at_ms).max(0.0))
        .unwrap_or(f64::INFINITY);
    let pipeline_not_advancing = stats.video_renderer_stalled.unwrap_or(false)
        || decode_age_ms >= HARD_STALL_DECODER_RESET_MS
        || present_age_ms >= HARD_STALL_DECODER_RESET_MS;
    if !pipeline_not_advancing {
        return false;
    }
    let Some((delivery_ratio, loss_ratio)) = extract_twcc_health_ratios(stats) else {
        return false;
    };
    delivery_ratio >= profile.decoder_backend_failure_min_twcc_delivery_ratio
        && loss_ratio <= profile.decoder_backend_failure_max_twcc_loss_ratio
}

fn extract_twcc_health_ratios(stats: &XbxEngineMediaRuntimeStats) -> Option<(f64, f64)> {
    if let Some(observation) = stats.latest_video_twcc_observation.as_ref() {
        return Some((observation.delivery_ratio, observation.packet_loss_ratio));
    }
    stats
        .latest_video_bwe_observation
        .as_ref()
        .and_then(|observation| {
            observation
                .twcc_delivery_ratio
                .zip(observation.twcc_loss_ratio)
        })
}

pub(crate) fn resolve_recovery_coupling_state(
    runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
    phase: SessionPhase,
) -> RecoveryCouplingState {
    // 这里先做粗粒度联动：recovery 只向 BWE 输出“该不该继续激进爬升”的约束，
    // 先不直接输出更复杂的 target，避免过早把两条策略硬耦合。
    let Some((
        diagnosis,
        _effective_bitrate_kbps,
        _recovery_profile,
        stable_output,
        startup_low,
        decoder_stalled,
        renderer_stalled,
    )) = RuntimeStatsSink::read_shared(runtime_stats, |stats| {
        let diagnosis = stats.recovery_diagnosis.clone();
        let effective_bitrate_kbps = extract_startup_recovery_bitrate_kbps(stats).unwrap_or(0.0);
        let recovery_profile = ScenarioPolicyResolver::resolve_recovery_profile(
            stats.session_target_type.as_ref(),
            stats.transport_path.as_deref(),
        );
        let stable_output = effective_bitrate_kbps
            >= recovery_profile.startup_low_quality_recovered_kbps
            && stats.video_present_fps >= 50.0;
        let startup_low = phase == SessionPhase::Startup
            && stats.direct_gaming_bitrate_band.as_deref() == Some("startupLow");
        Some((
            diagnosis,
            effective_bitrate_kbps,
            recovery_profile,
            stable_output,
            startup_low,
            stats.video_decoder_stalled.unwrap_or(false),
            stats.video_renderer_stalled.unwrap_or(false),
        ))
    })
    .flatten()
    else {
        return RecoveryCouplingState {
            mode: RecoveryCouplingMode::Healthy,
            suppress_ramp_up: false,
            prefer_hold: false,
            allow_peak_range: true,
        };
    };

    if startup_low {
        return RecoveryCouplingState {
            mode: RecoveryCouplingMode::StartupLowQuality,
            suppress_ramp_up: true,
            prefer_hold: true,
            allow_peak_range: false,
        };
    }

    // 已经回到 steady/healthy 且输出恢复后，直接退出 coupling，
    // 避免 BWE 在恢复结束后继续长时间按 recovery hold 运行。
    if phase == SessionPhase::Steady
        && diagnosis.is_none()
        && stable_output
        && !decoder_stalled
        && !renderer_stalled
    {
        return RecoveryCouplingState {
            mode: RecoveryCouplingMode::Healthy,
            suppress_ramp_up: false,
            prefer_hold: false,
            allow_peak_range: true,
        };
    }

    let coupling = match diagnosis.as_deref() {
        Some("waitKeyframe" | "transportAwaitRecoveryKeyframe" | "ingressWaitKeyframe") => {
            RecoveryCouplingState {
                mode: RecoveryCouplingMode::WaitingKeyframe,
                suppress_ramp_up: true,
                prefer_hold: true,
                allow_peak_range: false,
            }
        }
        Some("transportSampleLoss" | "reconfigure" | "ingressReconfigure") => {
            RecoveryCouplingState {
                mode: RecoveryCouplingMode::RecoveringReferenceChain,
                suppress_ramp_up: true,
                prefer_hold: true,
                allow_peak_range: false,
            }
        }
        Some("adapterThinStream" | "thinStream") => RecoveryCouplingState {
            mode: RecoveryCouplingMode::ThinStream,
            suppress_ramp_up: true,
            prefer_hold: true,
            allow_peak_range: false,
        },
        Some("adapterIdleTimeout" | "decoderBackendFailure") => RecoveryCouplingState {
            mode: RecoveryCouplingMode::Stalled,
            suppress_ramp_up: true,
            prefer_hold: true,
            allow_peak_range: false,
        },
        _ if phase == SessionPhase::Recovering && !stable_output => RecoveryCouplingState {
            mode: RecoveryCouplingMode::RecoveringReferenceChain,
            suppress_ramp_up: true,
            prefer_hold: true,
            allow_peak_range: false,
        },
        _ => RecoveryCouplingState {
            mode: RecoveryCouplingMode::Healthy,
            suppress_ramp_up: false,
            prefer_hold: false,
            allow_peak_range: true,
        },
    };
    coupling
}

#[cfg(test)]
mod tests {
    use super::{
        resolve_recovery_coupling_state, resolve_recovery_profile, unix_now_ms,
        RecoveryCoordinator, RecoveryCouplingMode,
    };
    use crate::runtime_stats_sink::RuntimeStatsSink;
    use crate::transport::webrtc::escalation::{
        RecoveryAction, VideoEscalationController, VideoEscalationReason,
    };
    use crate::transport::webrtc::startup_recovery::SessionPhase;
    use crate::XbxEngineMediaRuntimeStats;
    use crate::{XbxEngineVideoNackObservation, XbxEngineVideoTwccObservation};
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    use xbxengine_protocol::{XbxEngineTargetTypeDto, XbxEngineTransportStateDto};

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
    }

    fn healthy_twcc_observation(now_ms: f64) -> XbxEngineVideoTwccObservation {
        XbxEngineVideoTwccObservation {
            observation_id: 1,
            feedback_packet_count: 20,
            covered_sequence_start: 10,
            covered_sequence_end: 29,
            covered_sequence_span: 20,
            observed_packet_count: 20,
            observed_byte_count: 32_000,
            feedback_interval_ms: Some(100.0),
            arrival_span_ms: Some(95.0),
            receive_bitrate_kbps: Some(18_000.0),
            delivery_ratio: 0.99,
            packet_loss_ratio: 0.01,
            observed_at_ms: now_ms,
        }
    }

    #[test]
    fn recovered_nack_suppresses_transport_sample_loss_escalation() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.latest_video_nack_observation = Some(XbxEngineVideoNackObservation {
            observation_id: 1,
            action: "recovered".to_string(),
            source: "sampleLoss".to_string(),
            first_sequence: 1,
            last_sequence: 2,
            packet_count: 2,
            retry_count: 0,
            frame_rtp_timestamp: Some(1),
            frame_is_keyframe: Some(false),
            frame_importance: Some("delta".to_string()),
            deadline_at_ms: None,
            observed_at_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as f64,
        });
        let mut coordinator = RecoveryCoordinator::new(
            VideoEscalationController::new(Duration::from_millis(250), 2, 2),
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
        stats.latest_video_nack_observation = Some(XbxEngineVideoNackObservation {
            observation_id: 1,
            action: "expiredDeadline".to_string(),
            source: "sampleLoss".to_string(),
            first_sequence: 1,
            last_sequence: 2,
            packet_count: 2,
            retry_count: 2,
            frame_rtp_timestamp: Some(1),
            frame_is_keyframe: Some(false),
            frame_importance: Some("delta".to_string()),
            deadline_at_ms: None,
            observed_at_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as f64,
        });
        let mut coordinator = RecoveryCoordinator::new(
            VideoEscalationController::new(Duration::from_millis(250), 2, 2),
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
            VideoEscalationController::new(Duration::from_millis(250), 1, 1),
            Instant::now(),
            Duration::from_secs(2),
        );

        stats.latest_video_nack_observation = Some(XbxEngineVideoNackObservation {
            observation_id: 1,
            action: "expiredDeadline".to_string(),
            source: "sampleLoss".to_string(),
            first_sequence: 1,
            last_sequence: 2,
            packet_count: 2,
            retry_count: 2,
            frame_rtp_timestamp: Some(1),
            frame_is_keyframe: Some(false),
            frame_importance: Some("delta".to_string()),
            deadline_at_ms: None,
            observed_at_ms,
        });
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
        stats.latest_video_nack_observation = Some(XbxEngineVideoNackObservation {
            observation_id: 1,
            action: "expiredDeadline".to_string(),
            source: "sampleLoss".to_string(),
            first_sequence: 1,
            last_sequence: 2,
            packet_count: 2,
            retry_count: 2,
            frame_rtp_timestamp: Some(1),
            frame_is_keyframe: Some(false),
            frame_importance: Some("delta".to_string()),
            deadline_at_ms: None,
            observed_at_ms: now_ms,
        });
        let mut coordinator = RecoveryCoordinator::new(
            VideoEscalationController::new(Duration::from_millis(250), 1, 1),
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
        stats.latest_video_nack_observation = Some(XbxEngineVideoNackObservation {
            observation_id: 1,
            action: "expiredDeadline".to_string(),
            source: "sampleLoss".to_string(),
            first_sequence: 1,
            last_sequence: 2,
            packet_count: 2,
            retry_count: 2,
            frame_rtp_timestamp: Some(1),
            frame_is_keyframe: Some(false),
            frame_importance: Some("delta".to_string()),
            deadline_at_ms: None,
            observed_at_ms: now_ms,
        });

        let mut coordinator = RecoveryCoordinator::new(
            VideoEscalationController::new(Duration::from_millis(250), 2, 2),
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
            VideoEscalationController::new(Duration::from_millis(250), 2, 2),
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
        assert_eq!(state.diagnosis_label, "transportExpiredDeadline");
    }

    #[test]
    fn recovered_reference_nack_waits_for_burst_in_wait_keyframe_chain() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.latest_video_nack_observation = Some(XbxEngineVideoNackObservation {
            observation_id: 1,
            action: "recovered".to_string(),
            source: "sampleLoss".to_string(),
            first_sequence: 1,
            last_sequence: 2,
            packet_count: 2,
            retry_count: 0,
            frame_rtp_timestamp: Some(1),
            frame_is_keyframe: Some(true),
            frame_importance: Some("reference".to_string()),
            deadline_at_ms: None,
            observed_at_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as f64,
        });
        let mut coordinator = RecoveryCoordinator::new(
            VideoEscalationController::new(Duration::from_millis(250), 2, 2),
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
        stats.latest_video_nack_observation = Some(XbxEngineVideoNackObservation {
            observation_id: 1,
            action: "expiredDeadline".to_string(),
            source: "sampleLoss".to_string(),
            first_sequence: 1,
            last_sequence: 2,
            packet_count: 2,
            retry_count: 2,
            frame_rtp_timestamp: Some(1),
            frame_is_keyframe: Some(true),
            frame_importance: Some("reference".to_string()),
            deadline_at_ms: None,
            observed_at_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as f64,
        });
        let mut coordinator = RecoveryCoordinator::new(
            VideoEscalationController::new(Duration::from_millis(250), 1, 1),
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
            VideoEscalationController::new(Duration::from_millis(250), 2, 2),
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
            VideoEscalationController::new(Duration::from_millis(250), 2, 2),
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
            VideoEscalationController::new(Duration::from_millis(250), 2, 2),
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
            VideoEscalationController::new(Duration::from_millis(250), 2, 2),
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
            VideoEscalationController::new(Duration::from_millis(250), 2, 2),
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
    fn hard_paused_stream_escalates_to_reconnect_candidate_after_long_stall() {
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
            VideoEscalationController::new(Duration::from_millis(250), 2, 2),
            Instant::now(),
            Duration::from_millis(800),
        );
        let decision = coordinator.on_reason_with_runtime_stats(
            VideoEscalationReason::AdapterIdleTimeout,
            &Mutex::new(stats),
        );
        assert_eq!(decision.action, RecoveryAction::RequestReconnectCandidate);
    }

    #[test]
    fn hard_paused_stream_ignores_stale_present_fps_when_renderer_is_stalled() {
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
            VideoEscalationController::new(Duration::from_millis(250), 2, 2),
            Instant::now(),
            Duration::from_millis(800),
        );
        let decision = coordinator.on_reason_with_runtime_stats(
            VideoEscalationReason::AdapterIdleTimeout,
            &Mutex::new(stats),
        );
        assert_eq!(decision.action, RecoveryAction::RequestReconnectCandidate);
    }

    #[test]
    fn transport_expired_deadline_hard_pause_escalates_to_reconnect_candidate() {
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
            VideoEscalationController::new(Duration::from_millis(250), 2, 2),
            Instant::now(),
            Duration::from_millis(800),
        );
        let decision = coordinator.on_reason_with_runtime_stats(
            VideoEscalationReason::TransportExpiredDeadline,
            &Mutex::new(stats),
        );
        assert_eq!(decision.action, RecoveryAction::RequestReconnectCandidate);
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
    fn steady_healthy_output_exits_recovery_coupling() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.inbound_video_bitrate_kbps = Some(16_500.0);
        stats.video_present_fps = 58.0;
        let state = resolve_recovery_coupling_state(&Mutex::new(stats), SessionPhase::Steady);
        assert_eq!(state.mode, RecoveryCouplingMode::Healthy);
        assert!(!state.suppress_ramp_up);
        assert!(state.allow_peak_range);
    }
}
