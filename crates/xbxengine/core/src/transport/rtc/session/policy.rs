use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::api::backend::XbxEngineMediaRuntimeStats;
use crate::api::runtime::XbxEngineRuntimeConfig;
use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::bwe::evaluator::RtcBweEvaluation;
use crate::transport::rtc::bwe::policy::resolve_target_remb_kbps;
use crate::transport::rtc::facts::{ConnectionLifecycleStateFact, TransportCommand};
use crate::transport::rtc::policy::bwe::BwePolicyProposal;
use crate::transport::rtc::policy::planner::{
    PlannedTransportCommand, PolicyPlanInput, TransportCommandPlanner,
};
use crate::transport::rtc::policy::reconnect::ReconnectPolicyProposal;
use crate::transport::rtc::policy::recovery::RecoveryPolicyProposal;
use crate::transport::rtc::projection::TransportSnapshot;
use crate::transport::rtc::recovery::escalation::{
    RecoveryAction, VideoEscalationController, VideoEscalationReason,
};
use crate::transport::rtc::recovery::startup::SessionPhase;
use crate::transport::rtc::session::actor::SessionPolicyHook;

const DEFAULT_BWE_TARGET_KBPS: u32 = 16_000;
const RECOVERY_REPEAT_SUPPRESS_MS: f64 = 160.0;
const BWE_UNSTABLE_HOLD_CONFIRMATION_TICKS: u8 = 2;

#[derive(Clone, Debug)]
struct RecoverySignalCursor {
    label: String,
    observed_at_ms: f64,
    emitted_at_ms: f64,
}

/// rtc session 主线策略：
/// - 统一把 reconnect/recovery/BWE proposal 收口到 session policy
/// - 复用 planner 的优先级（reconnect > recovery > bwe）
/// - stack 只做命令执行与 CommandResultFact 回写
pub struct RtcSessionPolicy {
    runtime_config: Arc<Mutex<XbxEngineRuntimeConfig>>,
    runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    reconnect_inflight: bool,
    planner: TransportCommandPlanner,
    escalation_controller: VideoEscalationController,
    last_recovery_signal: Option<RecoverySignalCursor>,
    last_bwe_sample_tick_ms: Option<f64>,
    last_sent_remb_kbps: u32,
    hybrid_ramp_cooldown_ticks: u8,
    next_reconnect_observation_id: u64,
    next_bwe_observation_id: u64,
    last_bwe_reason: Option<String>,
    unstable_hold_streak: u8,
}

impl RtcSessionPolicy {
    pub fn new(
        runtime_config: Arc<Mutex<XbxEngineRuntimeConfig>>,
        runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) -> Self {
        Self {
            runtime_config,
            runtime_stats,
            reconnect_inflight: false,
            planner: TransportCommandPlanner::new(),
            escalation_controller: VideoEscalationController::new(Duration::from_millis(320), 2, 3),
            last_recovery_signal: None,
            last_bwe_sample_tick_ms: None,
            last_sent_remb_kbps: DEFAULT_BWE_TARGET_KBPS,
            hybrid_ramp_cooldown_ticks: 0,
            next_reconnect_observation_id: 0,
            next_bwe_observation_id: 0,
            last_bwe_reason: None,
            unstable_hold_streak: 0,
        }
    }
}

impl Default for RtcSessionPolicy {
    fn default() -> Self {
        Self::new(
            Arc::new(Mutex::new(XbxEngineRuntimeConfig::default())),
            Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default())),
        )
    }
}

impl SessionPolicyHook for RtcSessionPolicy {
    fn on_snapshot(&mut self, snapshot: &TransportSnapshot) -> Vec<TransportCommand> {
        if snapshot.connection.lifecycle_state != ConnectionLifecycleStateFact::Recovering {
            self.reconnect_inflight = false;
        }
        let recovery = self.build_recovery_proposal(snapshot);
        let reconnect = self.build_reconnect_proposal(snapshot, recovery.as_ref());
        let bwe = self.build_bwe_proposal(snapshot);
        let bwe_observation_id = bwe
            .as_ref()
            .map(|proposal| proposal.evaluation.observation_id)
            .unwrap_or(0);

        self.planner
            .plan(PolicyPlanInput {
                recovery,
                reconnect,
                bwe,
            })
            .commands
            .into_iter()
            .flat_map(|command| self.map_planned_command(command, bwe_observation_id))
            .collect()
    }
}

impl RtcSessionPolicy {
    fn build_reconnect_proposal(
        &mut self,
        snapshot: &TransportSnapshot,
        recovery: Option<&RecoveryPolicyProposal>,
    ) -> Option<ReconnectPolicyProposal> {
        if self.reconnect_inflight {
            return None;
        }
        if snapshot.connection.lifecycle_state != ConnectionLifecycleStateFact::Recovering {
            return None;
        }
        // reconnect 是 Recovering 主线的独立动作，不能被 recovery/BWE 绑架。
        let (observation_id, reason) = if let Some(recovery_proposal) = recovery {
            let reason = recovery_proposal.reason_label.clone();
            if matches!(
                recovery_proposal.decision.action,
                RecoveryAction::RequestReconnectCandidate
            ) {
                (recovery_proposal.decision.observation_id, reason)
            } else {
                self.next_reconnect_observation_id =
                    self.next_reconnect_observation_id.saturating_add(1);
                (self.next_reconnect_observation_id, reason)
            }
        } else {
            let reason = snapshot
                .recovery
                .latest_diagnosis_label
                .clone()
                .or_else(|| snapshot.diagnostics.latest_label.clone())
                .unwrap_or_else(|| "rtcConnectionRecovering".to_string());
            self.next_reconnect_observation_id =
                self.next_reconnect_observation_id.saturating_add(1);
            (self.next_reconnect_observation_id, reason)
        };
        self.reconnect_inflight = true;
        Some(ReconnectPolicyProposal {
            observation_id,
            reason,
        })
    }

    fn build_recovery_proposal(
        &mut self,
        snapshot: &TransportSnapshot,
    ) -> Option<RecoveryPolicyProposal> {
        let label = snapshot.recovery.latest_diagnosis_label.as_deref()?;
        let reason = map_label_to_escalation_reason(label)?;
        let observed_at_ms = snapshot
            .recovery
            .last_observed_at_ms
            .unwrap_or(snapshot.now_ms);
        if !self.should_emit_recovery_signal(label, observed_at_ms) {
            return None;
        }
        let decision = self.escalation_controller.on_reason(reason);
        Some(RecoveryPolicyProposal {
            decision,
            reason_label: label.to_string(),
        })
    }

    fn build_bwe_proposal(&mut self, snapshot: &TransportSnapshot) -> Option<BwePolicyProposal> {
        if !matches!(
            snapshot.connection.lifecycle_state,
            ConnectionLifecycleStateFact::Connected | ConnectionLifecycleStateFact::Recovering
        ) {
            return None;
        }
        let sample_tick_ms = snapshot.bwe.latest_sample_tick_ms?;
        if self
            .last_bwe_sample_tick_ms
            .is_some_and(|last| sample_tick_ms <= last)
        {
            return None;
        }
        self.last_bwe_sample_tick_ms = Some(sample_tick_ms);

        let loss_ratio = snapshot
            .bwe
            .latest_loss_ratio_1s
            .or(snapshot.connection.latest_loss_ratio_1s)
            .unwrap_or(0.0)
            .clamp(0.0, 1.0);
        let rtt_ms = snapshot
            .bwe
            .latest_rtt_ms
            .or(snapshot.connection.latest_rtt_ms);
        let actual_kbps = snapshot.bwe.latest_actual_video_bitrate_kbps.unwrap_or(0.0);
        let webrtc_config = self
            .runtime_config
            .lock()
            .ok()
            .map(|config| config.webrtc.clone())
            .unwrap_or_default();
        let (session_target_type, twcc_observation, session_phase) =
            RuntimeStatsSink::read_shared(self.runtime_stats.as_ref(), |stats| {
                (
                    stats.session_target_type.clone(),
                    stats.latest_video_twcc_observation.clone(),
                    parse_session_phase(stats.session_phase.as_deref()),
                )
            })
            .unwrap_or((None, None, SessionPhase::Steady));
        let current_target_kbps = snapshot
            .bwe
            .target_remb_kbps
            .unwrap_or(self.last_sent_remb_kbps.max(DEFAULT_BWE_TARGET_KBPS));
        self.last_sent_remb_kbps = current_target_kbps;
        let bwe_decision = resolve_target_remb_kbps(
            &webrtc_config,
            snapshot.bwe.latest_observed_remb_kbps,
            actual_kbps,
            loss_ratio,
            rtt_ms,
            session_target_type.as_ref(),
            snapshot.connection.latest_transport_path.as_deref(),
            session_phase,
            None,
            twcc_observation.as_ref(),
            &mut self.last_sent_remb_kbps,
            &mut self.hybrid_ramp_cooldown_ticks,
        );
        let target_kbps = bwe_decision.target_kbps;
        let decision_reason = bwe_decision.reason;
        let reason_changed = self
            .last_bwe_reason
            .as_ref()
            .is_none_or(|last| last != &decision_reason);
        let is_unstable_hold = decision_reason.ends_with("unstable-hold");
        if is_unstable_hold && target_kbps == current_target_kbps {
            self.unstable_hold_streak = self.unstable_hold_streak.saturating_add(1);
            if self.unstable_hold_streak < BWE_UNSTABLE_HOLD_CONFIRMATION_TICKS {
                return None;
            }
        } else {
            self.unstable_hold_streak = 0;
        }
        if target_kbps == current_target_kbps && !reason_changed {
            return None;
        }
        self.last_bwe_reason = Some(decision_reason.clone());
        self.next_bwe_observation_id = self.next_bwe_observation_id.saturating_add(1);
        let evaluation = RtcBweEvaluation {
            target_remb_kbps: target_kbps,
            decision_reason,
            observation_id: self.next_bwe_observation_id,
        };
        Some(BwePolicyProposal { evaluation })
    }

    fn should_emit_recovery_signal(&mut self, label: &str, observed_at_ms: f64) -> bool {
        if let Some(last) = self.last_recovery_signal.clone() {
            if last.label == label {
                if observed_at_ms <= last.observed_at_ms {
                    return false;
                }
                if observed_at_ms - last.emitted_at_ms < RECOVERY_REPEAT_SUPPRESS_MS {
                    self.last_recovery_signal = Some(RecoverySignalCursor {
                        label: label.to_string(),
                        observed_at_ms,
                        emitted_at_ms: last.emitted_at_ms,
                    });
                    return false;
                }
            }
        }
        self.last_recovery_signal = Some(RecoverySignalCursor {
            label: label.to_string(),
            observed_at_ms,
            emitted_at_ms: observed_at_ms,
        });
        true
    }

    fn map_planned_command(
        &mut self,
        command: PlannedTransportCommand,
        bwe_observation_id: u64,
    ) -> Vec<TransportCommand> {
        match command {
            PlannedTransportCommand::RequestReconnectCandidate {
                observation_id,
                reason,
            } => vec![TransportCommand::RequestReconnectCandidate {
                reason,
                observation_id,
            }],
            PlannedTransportCommand::ExecuteRecoveryAction {
                decision,
                reason_label,
            } => map_recovery_action_to_transport_commands(
                decision.action,
                reason_label,
                decision.observation_id,
            ),
            PlannedTransportCommand::UpdateTargetRemb {
                target_remb_kbps,
                decision_reason,
            } => vec![TransportCommand::SetTargetRembKbps {
                target_kbps: target_remb_kbps,
                reason: decision_reason,
                observation_id: bwe_observation_id,
            }],
        }
    }
}

fn parse_session_phase(value: Option<&str>) -> SessionPhase {
    match value {
        Some("startup") => SessionPhase::Startup,
        Some("recovering") => SessionPhase::Recovering,
        _ => SessionPhase::Steady,
    }
}

fn map_recovery_action_to_transport_commands(
    action: RecoveryAction,
    reason: String,
    observation_id: u64,
) -> Vec<TransportCommand> {
    match action {
        RecoveryAction::RequestKeyframe => vec![TransportCommand::RequestKeyframe {
            reason,
            observation_id,
        }],
        RecoveryAction::RequestDecoderReset => vec![TransportCommand::RequestDecoderReset {
            reason,
            observation_id,
        }],
        RecoveryAction::RequestReconnectCandidate => {
            vec![TransportCommand::RequestReconnectCandidate {
                reason,
                observation_id,
            }]
        }
        RecoveryAction::RequestKeyframeAndDecoderReset | RecoveryAction::StartupLowQualityRetry => {
            vec![
                TransportCommand::RequestKeyframe {
                    reason: reason.clone(),
                    observation_id,
                },
                TransportCommand::RequestDecoderReset {
                    reason,
                    observation_id,
                },
            ]
        }
        RecoveryAction::WaitForBurst
        | RecoveryAction::WaitForDecoderResetBurst
        | RecoveryAction::CooldownSuppressed
        | RecoveryAction::StartupGraceSuppressed => Vec::new(),
    }
}

fn map_label_to_escalation_reason(label: &str) -> Option<VideoEscalationReason> {
    match label {
        "ingressWaitKeyframe" => Some(VideoEscalationReason::WaitKeyframe),
        "ingressFrameAbandoned" => Some(VideoEscalationReason::WaitKeyframe),
        "waitKeyframeEntered" => Some(VideoEscalationReason::WaitKeyframe),
        "frameAbandoned" => Some(VideoEscalationReason::WaitKeyframe),
        "transportAwaitRecoveryKeyframe" => {
            Some(VideoEscalationReason::TransportAwaitRecoveryKeyframe)
        }
        "ingressReconfigure" => Some(VideoEscalationReason::Reconfigure),
        "decoderBackendFailure" => Some(VideoEscalationReason::DecoderBackendFailure),
        "adapterIdleTimeout" => Some(VideoEscalationReason::AdapterIdleTimeout),
        "adapterThinStream" => Some(VideoEscalationReason::AdapterThinStream),
        "transportExpiredDeadline" => Some(VideoEscalationReason::TransportExpiredDeadline),
        "transportSevereDeadline" => Some(VideoEscalationReason::TransportSevereDeadline),
        "transportRecoveredLate" => Some(VideoEscalationReason::TransportRecoveredLate),
        "transportSampleLoss" => Some(VideoEscalationReason::TransportSampleLoss),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::RtcSessionPolicy;
    use crate::api::backend::{XbxEngineMediaRuntimeStats, XbxEngineVideoTwccObservation};
    use crate::api::runtime::XbxEngineRuntimeConfig;
    use crate::transport::rtc::facts::{ConnectionLifecycleStateFact, TransportCommand};
    use crate::transport::rtc::projection::{
        BweProjection, ConnectionProjection, DiagnosticsProjection, MediaProjection,
        RecoveryProjection, TransportSnapshot,
    };
    use crate::transport::rtc::session::actor::SessionPolicyHook;
    use std::sync::{Arc, Mutex};

    #[test]
    fn reconnect_command_is_emitted_once_per_recovering_transition() {
        let mut policy = RtcSessionPolicy::default();
        let mut connection = ConnectionProjection::default();
        let mut recovery = RecoveryProjection::default();
        recovery.latest_diagnosis_label = Some("rtcPeerConnectionFailed".to_string());
        connection.lifecycle_state = ConnectionLifecycleStateFact::Recovering;

        let snapshot = TransportSnapshot::new(
            1,
            1.0,
            connection.clone(),
            MediaProjection::default(),
            recovery.clone(),
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let commands = policy.on_snapshot(&snapshot);
        assert_eq!(commands.len(), 1);
        assert!(policy.on_snapshot(&snapshot).is_empty());

        connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
        let steady_snapshot = TransportSnapshot::new(
            2,
            2.0,
            connection.clone(),
            MediaProjection::default(),
            recovery.clone(),
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        assert!(policy.on_snapshot(&steady_snapshot).is_empty());

        connection.lifecycle_state = ConnectionLifecycleStateFact::Recovering;
        let recovering_again = TransportSnapshot::new(
            3,
            3.0,
            connection,
            MediaProjection::default(),
            recovery,
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        assert_eq!(policy.on_snapshot(&recovering_again).len(), 1);
    }

    #[test]
    fn transport_await_recovery_keyframe_can_emit_keyframe_command() {
        let mut policy = RtcSessionPolicy::default();
        let mut snapshot = build_snapshot(
            ConnectionLifecycleStateFact::Connected,
            "transportAwaitRecoveryKeyframe",
            100.0,
        );
        let commands = policy.on_snapshot(&snapshot);
        assert!(commands
            .iter()
            .any(|command| matches!(command, TransportCommand::RequestKeyframe { .. })));

        snapshot.recovery.last_observed_at_ms = Some(120.0);
        assert!(policy.on_snapshot(&snapshot).is_empty());
    }

    #[test]
    fn decoder_backend_failure_can_emit_decoder_reset_command() {
        let mut policy = RtcSessionPolicy::default();
        let snapshot = build_snapshot(
            ConnectionLifecycleStateFact::Connected,
            "decoderBackendFailure",
            180.0,
        );
        let commands = policy.on_snapshot(&snapshot);
        assert!(commands
            .iter()
            .any(|command| matches!(command, TransportCommand::RequestDecoderReset { .. })));
    }

    #[test]
    fn bwe_tick_emits_target_remb_update_when_metrics_are_healthy() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        if let Ok(mut config) = runtime_config.lock() {
            config.webrtc.bwe_mode = "observed-remb".to_string();
        }
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
        connection.latest_loss_ratio_1s = Some(0.01);
        connection.latest_rtt_ms = Some(40.0);
        connection.latest_transport_path = Some("udp-direct".to_string());
        let bwe = BweProjection {
            latest_rtt_ms: Some(40.0),
            latest_loss_ratio_1s: Some(0.01),
            latest_actual_video_bitrate_kbps: Some(16_000.0),
            latest_observed_remb_kbps: Some(20_000),
            latest_transport_path: Some("udp-direct".to_string()),
            latest_sample_tick_ms: Some(300.0),
            target_remb_kbps: Some(16_000),
            last_observed_at_ms: Some(300.0),
        };
        let snapshot = TransportSnapshot::new(
            1,
            300.0,
            connection,
            MediaProjection::default(),
            RecoveryProjection::default(),
            bwe,
            DiagnosticsProjection::default(),
        );
        let commands = policy.on_snapshot(&snapshot);
        let command = commands
            .into_iter()
            .find_map(|command| {
                if let TransportCommand::SetTargetRembKbps { target_kbps, .. } = command {
                    Some(target_kbps)
                } else {
                    None
                }
            })
            .unwrap_or(0);
        assert!(command > 16_000);
    }

    #[test]
    fn runtime_config_floor_is_respected() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        if let Ok(mut config) = runtime_config.lock() {
            config.webrtc.bwe_mode = "observed-remb".to_string();
            config.webrtc.remb_floor_kbps = 25_000;
            config.webrtc.remb_ceiling_kbps = 150_000;
        }
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
        connection.latest_loss_ratio_1s = Some(0.0);
        connection.latest_rtt_ms = Some(35.0);
        connection.latest_transport_path = Some("Direct".to_string());
        let bwe = BweProjection {
            latest_rtt_ms: Some(35.0),
            latest_loss_ratio_1s: Some(0.0),
            latest_actual_video_bitrate_kbps: Some(14_000.0),
            latest_observed_remb_kbps: Some(16_000),
            latest_transport_path: Some("Direct".to_string()),
            latest_sample_tick_ms: Some(400.0),
            target_remb_kbps: Some(12_000),
            last_observed_at_ms: Some(400.0),
        };
        let snapshot = TransportSnapshot::new(
            2,
            400.0,
            connection,
            MediaProjection::default(),
            RecoveryProjection::default(),
            bwe,
            DiagnosticsProjection::default(),
        );
        let target = policy
            .on_snapshot(&snapshot)
            .into_iter()
            .find_map(|command| {
                if let TransportCommand::SetTargetRembKbps { target_kbps, .. } = command {
                    Some(target_kbps)
                } else {
                    None
                }
            })
            .unwrap_or(0);
        assert_eq!(target, 25_000);
    }

    #[test]
    fn session_target_type_and_twcc_input_flow_into_new_bwe_policy() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        if let Ok(mut config) = runtime_config.lock() {
            config.webrtc.bwe_mode = "twcc-gcc".to_string();
            config.webrtc.remb_floor_kbps = 8_000;
            config.webrtc.remb_ceiling_kbps = 150_000;
        }
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        if let Ok(mut stats) = runtime_stats.lock() {
            stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud);
            stats.latest_video_twcc_observation = Some(XbxEngineVideoTwccObservation {
                observation_id: 1,
                source: "local-feedback".to_string(),
                feedback_packet_count: 3,
                covered_sequence_start: 100,
                covered_sequence_end: 120,
                covered_sequence_span: 20,
                observed_packet_count: 20,
                observed_byte_count: 30_000,
                coverage_ratio: None,
                ledger_hit_ratio: None,
                feedback_interval_ms: Some(80.0),
                arrival_span_ms: Some(70.0),
                receive_bitrate_kbps: Some(28_000.0),
                twcc_sample_valid: true,

                twcc_invalid_reason: None,

                quality: crate::XbxEngineTwccObservationQuality::Stable,
                delivery_ratio: 0.995,
                packet_loss_ratio: 0.0,
                observed_at_ms: 10.0,
            });
            stats.session_phase = Some("steady".to_string());
        }
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
        connection.latest_loss_ratio_1s = Some(0.0);
        connection.latest_rtt_ms = Some(40.0);
        connection.latest_transport_path = Some("Direct".to_string());
        let bwe = BweProjection {
            latest_rtt_ms: Some(40.0),
            latest_loss_ratio_1s: Some(0.0),
            latest_actual_video_bitrate_kbps: Some(18_000.0),
            latest_observed_remb_kbps: Some(28_000),
            latest_transport_path: Some("Direct".to_string()),
            latest_sample_tick_ms: Some(1.0),
            target_remb_kbps: Some(18_000),
            last_observed_at_ms: Some(1.0),
        };
        let snapshot = TransportSnapshot::new(
            1,
            1.0,
            connection,
            MediaProjection::default(),
            RecoveryProjection::default(),
            bwe,
            DiagnosticsProjection::default(),
        );
        let reason = policy
            .on_snapshot(&snapshot)
            .into_iter()
            .find_map(|command| {
                if let TransportCommand::SetTargetRembKbps { reason, .. } = command {
                    Some(reason)
                } else {
                    None
                }
            });
        assert!(reason.is_some_and(|value| value.starts_with("twcc-gcc-cloud-")));
    }

    #[test]
    fn bwe_emits_reason_update_even_when_target_is_unchanged() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        if let Ok(mut config) = runtime_config.lock() {
            config.webrtc.bwe_mode = "twcc-gcc".to_string();
            config.webrtc.remb_floor_kbps = 8_000;
            config.webrtc.remb_ceiling_kbps = 50_000;
            config.webrtc.video_pipeline.feedback_interval_ms = 1_000;
        }
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        if let Ok(mut stats) = runtime_stats.lock() {
            stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud);
            stats.session_phase = Some("steady".to_string());
        }
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
        policy.last_sent_remb_kbps = 25_000;
        policy.last_bwe_reason = Some("twcc-gcc-cloud-await-feedback".to_string());

        if let Ok(mut stats) = runtime_stats.lock() {
            stats.latest_video_twcc_observation = Some(XbxEngineVideoTwccObservation {
                observation_id: 1,
                source: "local-feedback".to_string(),
                feedback_packet_count: 3,
                covered_sequence_start: 100,
                covered_sequence_end: 220,
                covered_sequence_span: 120,
                observed_packet_count: 120,
                observed_byte_count: 180_000,
                coverage_ratio: None,
                ledger_hit_ratio: None,
                feedback_interval_ms: Some(1_000.0),
                arrival_span_ms: Some(1_000.0),
                receive_bitrate_kbps: Some(24_500.0),
                twcc_sample_valid: true,

                twcc_invalid_reason: None,

                quality: crate::XbxEngineTwccObservationQuality::Stable,
                delivery_ratio: 1.0,
                packet_loss_ratio: 0.0,
                observed_at_ms: 10.0,
            });
        }

        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
        connection.latest_loss_ratio_1s = Some(0.0);
        connection.latest_rtt_ms = Some(40.0);
        connection.latest_transport_path = Some("Direct".to_string());
        let bwe = BweProjection {
            latest_rtt_ms: Some(40.0),
            latest_loss_ratio_1s: Some(0.0),
            latest_actual_video_bitrate_kbps: Some(18_000.0),
            latest_observed_remb_kbps: Some(25_000),
            latest_transport_path: Some("Direct".to_string()),
            latest_sample_tick_ms: Some(1.0),
            target_remb_kbps: Some(25_000),
            last_observed_at_ms: Some(1.0),
        };
        let snapshot = TransportSnapshot::new(
            1,
            1.0,
            connection,
            MediaProjection::default(),
            RecoveryProjection::default(),
            bwe,
            DiagnosticsProjection::default(),
        );

        let reason = policy
            .on_snapshot(&snapshot)
            .into_iter()
            .find_map(|command| {
                if let TransportCommand::SetTargetRembKbps { reason, .. } = command {
                    Some(reason)
                } else {
                    None
                }
            });

        assert!(reason.is_some());
        assert_ne!(reason.as_deref(), Some("twcc-gcc-cloud-await-feedback"));
    }

    #[test]
    fn reconnect_keeps_priority_over_recovery_and_bwe() {
        let mut policy = RtcSessionPolicy::default();
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::Recovering;
        connection.latest_loss_ratio_1s = Some(0.01);
        connection.latest_rtt_ms = Some(40.0);
        let recovery = RecoveryProjection {
            latest_diagnosis_label: Some("transportAwaitRecoveryKeyframe".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(100.0),
        };
        let bwe = BweProjection {
            latest_rtt_ms: Some(40.0),
            latest_loss_ratio_1s: Some(0.01),
            latest_actual_video_bitrate_kbps: Some(12_000.0),
            latest_observed_remb_kbps: Some(18_000),
            latest_transport_path: Some("udp-direct".to_string()),
            latest_sample_tick_ms: Some(100.0),
            target_remb_kbps: Some(12_000),
            last_observed_at_ms: Some(100.0),
        };
        let snapshot = TransportSnapshot::new(
            1,
            100.0,
            connection,
            MediaProjection::default(),
            recovery,
            bwe,
            DiagnosticsProjection::default(),
        );
        let commands = policy.on_snapshot(&snapshot);
        assert_eq!(commands.len(), 1);
        assert!(matches!(
            commands[0],
            TransportCommand::RequestReconnectCandidate { .. }
        ));
    }

    #[test]
    fn unstable_hold_requires_consecutive_confirmation_before_emit() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        if let Ok(mut config) = runtime_config.lock() {
            config.webrtc.bwe_mode = "twcc-gcc".to_string();
            config.webrtc.video_pipeline.feedback_interval_ms = 100;
        }
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        if let Ok(mut stats) = runtime_stats.lock() {
            stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud);
            stats.session_phase = Some("steady".to_string());
            stats.latest_video_twcc_observation = Some(XbxEngineVideoTwccObservation {
                observation_id: 1,
                source: "local-feedback".to_string(),
                feedback_packet_count: 1,
                covered_sequence_start: 1,
                covered_sequence_end: 2,
                covered_sequence_span: 2,
                observed_packet_count: 1,
                observed_byte_count: 1200,
                coverage_ratio: None,
                ledger_hit_ratio: None,
                feedback_interval_ms: None,
                arrival_span_ms: None,
                receive_bitrate_kbps: Some(0.0),
                twcc_sample_valid: true,

                twcc_invalid_reason: None,

                quality: crate::XbxEngineTwccObservationQuality::Stable,
                delivery_ratio: 1.0,
                packet_loss_ratio: 0.0,
                observed_at_ms: 1.0,
            });
        }
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
        policy.last_sent_remb_kbps = 25_000;

        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
        connection.latest_transport_path = Some("Direct".to_string());
        let snapshot_first = TransportSnapshot::new(
            1,
            1.0,
            connection.clone(),
            MediaProjection::default(),
            RecoveryProjection::default(),
            BweProjection {
                latest_rtt_ms: Some(180.0),
                latest_loss_ratio_1s: Some(0.0),
                latest_actual_video_bitrate_kbps: Some(1_000.0),
                latest_observed_remb_kbps: Some(25_000),
                latest_transport_path: Some("Direct".to_string()),
                latest_sample_tick_ms: Some(1.0),
                target_remb_kbps: Some(25_000),
                last_observed_at_ms: Some(1.0),
            },
            DiagnosticsProjection::default(),
        );
        let first_commands = policy.on_snapshot(&snapshot_first);
        assert!(first_commands
            .iter()
            .all(|command| !matches!(command, TransportCommand::SetTargetRembKbps { .. })));

        let snapshot_second = TransportSnapshot::new(
            2,
            2.0,
            connection,
            MediaProjection::default(),
            RecoveryProjection::default(),
            BweProjection {
                latest_rtt_ms: Some(180.0),
                latest_loss_ratio_1s: Some(0.0),
                latest_actual_video_bitrate_kbps: Some(1_000.0),
                latest_observed_remb_kbps: Some(25_000),
                latest_transport_path: Some("Direct".to_string()),
                latest_sample_tick_ms: Some(2.0),
                target_remb_kbps: Some(25_000),
                last_observed_at_ms: Some(2.0),
            },
            DiagnosticsProjection::default(),
        );
        let second_commands = policy.on_snapshot(&snapshot_second);
        assert!(second_commands.iter().any(|command| {
            matches!(
                command,
                TransportCommand::SetTargetRembKbps { reason, .. }
                    if reason.contains("unstable-hold")
            )
        }));
    }

    fn build_snapshot(
        lifecycle_state: ConnectionLifecycleStateFact,
        diagnosis: &str,
        observed_at_ms: f64,
    ) -> TransportSnapshot {
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = lifecycle_state;
        let recovery = RecoveryProjection {
            latest_diagnosis_label: Some(diagnosis.to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(observed_at_ms),
        };
        TransportSnapshot::new(
            1,
            observed_at_ms,
            connection,
            MediaProjection::default(),
            recovery,
            BweProjection::default(),
            DiagnosticsProjection::default(),
        )
    }
}
