use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::recovery::decoder_backend_failure::{
    resolve_decoder_backend_failure_recovery, DecoderBackendFailureResolution,
};
use crate::transport::rtc::recovery::escalation::{
    RecoveryAction, VideoEscalationController, VideoEscalationDecision, VideoEscalationReason,
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
use crate::XbxEngineMediaRuntimeStats;

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
    cloud_startup_nack_budget: CloudStartupExpiredDeadlineBudget,
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
            cloud_startup_nack_budget: CloudStartupExpiredDeadlineBudget::default(),
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
        if matches!(reason, VideoEscalationReason::AdapterIdleTimeout)
            && RuntimeStatsSink::read_shared(runtime_stats, |stats| {
                has_fresh_media_output(stats, unix_now_ms())
                    && !stats.video_decoder_stalled.unwrap_or(false)
                    && !stats.video_renderer_stalled.unwrap_or(false)
            })
            .unwrap_or(false)
        {
            return self
                .escalation_controller
                .suppressed(RecoveryAction::CooldownSuppressed);
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
                Some(self.on_reason_with_policy(
                    VideoEscalationReason::DecoderBackendFailure,
                    phase,
                    profile,
                ))
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
                Some(self.on_reason_with_policy(reason, phase, profile))
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

#[cfg(test)]
mod tests {
    use super::RecoveryCoordinator;
    use crate::runtime_stats_sink::RuntimeStatsSink;
    use crate::transport::rtc::recovery::escalation::{
        RecoveryAction, VideoEscalationController, VideoEscalationReason,
    };
    use crate::transport::rtc::recovery::runtime_state::{
        resolve_recovery_coupling_state, resolve_recovery_profile, unix_now_ms,
        RecoveryCouplingMode,
    };
    use crate::transport::rtc::recovery::startup::SessionPhase;
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
            source: "local-feedback".to_string(),
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
}
