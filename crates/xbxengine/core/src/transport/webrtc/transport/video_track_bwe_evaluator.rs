use std::time::{Duration, Instant};

use crate::{
    runtime_stats_sink::RuntimeStatsSink,
    transport::webrtc::bwe_policy::{
        classify_scenario_bitrate_band, resolve_target_remb_kbps,
        resolve_transport_policy_profile_kind,
    },
    transport::webrtc::recovery_coordinator::RecoveryCoordinator,
    transport::webrtc::startup_recovery::{resolve_session_phase, SessionPhase},
    XbxEngineWebRtcRuntimeConfig,
};

use super::video_track_observation_collector::VideoTrackTransportObservation;

// BWE 决策状态只在 evaluator 内维护，避免 collector 直接触达策略状态。
pub(super) struct VideoTrackBweEvaluatorState {
    last_sent_remb_kbps: u32,
    hybrid_ramp_cooldown_ticks: u8,
    bwe_observation_id: u64,
}

pub(super) struct VideoTrackBweEvaluation {
    pub target_remb_kbps: u32,
    pub decision_reason: String,
    pub session_phase: SessionPhase,
    pub transport_policy_profile: String,
    pub recovery_coupling_mode: String,
    pub recovery_coupling_summary: String,
    pub direct_gaming_bitrate_band: Option<String>,
    pub twcc_feedback_interval_ms: Option<f64>,
    pub twcc_observed_packet_count: Option<u16>,
    pub twcc_covered_sequence_span: Option<u16>,
    pub twcc_receive_bitrate_kbps: Option<f64>,
    pub twcc_delivery_ratio: Option<f64>,
    pub twcc_loss_ratio: Option<f64>,
    pub observation_id: u64,
}

impl VideoTrackBweEvaluatorState {
    pub(super) fn new(initial_last_sent_remb_kbps: u32) -> Self {
        Self {
            last_sent_remb_kbps: initial_last_sent_remb_kbps,
            hybrid_ramp_cooldown_ticks: 0,
            bwe_observation_id: 0,
        }
    }

    pub(super) fn evaluate(
        &mut self,
        runtime_stats: &RuntimeStatsSink,
        webrtc_config: &XbxEngineWebRtcRuntimeConfig,
        observation: &VideoTrackTransportObservation,
        bwe_stream_started_at: Instant,
        bwe_startup_grace: Duration,
    ) -> VideoTrackBweEvaluation {
        let (latest_twcc_observation, session_target_type) = runtime_stats
            .read(|shared| {
                (
                    shared.latest_video_twcc_observation.clone(),
                    shared.session_target_type.clone(),
                )
            })
            .unwrap_or((None, None));
        let session_phase = resolve_session_phase(
            runtime_stats.shared(),
            bwe_stream_started_at,
            bwe_startup_grace,
        );
        let recovery_coupling = RecoveryCoordinator::current_coupling_state(
            runtime_stats.shared(),
            bwe_stream_started_at,
            bwe_startup_grace,
        );
        let transport_profile_kind = resolve_transport_policy_profile_kind(
            session_target_type.as_ref(),
            observation.transport_path.as_deref(),
        );
        let bwe_decision = resolve_target_remb_kbps(
            webrtc_config,
            observation.observed_remb_kbps,
            observation.actual_kbps,
            observation.fraction_lost,
            observation.rtt_ms,
            session_target_type.as_ref(),
            observation.transport_path.as_deref(),
            session_phase,
            Some(recovery_coupling),
            latest_twcc_observation.as_ref(),
            &mut self.last_sent_remb_kbps,
            &mut self.hybrid_ramp_cooldown_ticks,
        );
        self.bwe_observation_id = self.bwe_observation_id.saturating_add(1);

        VideoTrackBweEvaluation {
            target_remb_kbps: bwe_decision.target_kbps,
            decision_reason: bwe_decision.reason,
            session_phase,
            transport_policy_profile: transport_profile_kind.as_str().to_string(),
            recovery_coupling_mode: recovery_coupling.mode.as_str().to_string(),
            recovery_coupling_summary: recovery_coupling.summary(),
            direct_gaming_bitrate_band: classify_scenario_bitrate_band(
                session_target_type.as_ref(),
                observation.transport_path.as_deref(),
                Some(observation.actual_kbps),
            )
            .map(str::to_string),
            twcc_feedback_interval_ms: latest_twcc_observation
                .as_ref()
                .and_then(|twcc| twcc.feedback_interval_ms),
            twcc_observed_packet_count: latest_twcc_observation
                .as_ref()
                .map(|twcc| twcc.observed_packet_count),
            twcc_covered_sequence_span: latest_twcc_observation
                .as_ref()
                .map(|twcc| twcc.covered_sequence_span),
            twcc_receive_bitrate_kbps: latest_twcc_observation
                .as_ref()
                .and_then(|twcc| twcc.receive_bitrate_kbps),
            twcc_delivery_ratio: latest_twcc_observation
                .as_ref()
                .map(|twcc| twcc.delivery_ratio),
            twcc_loss_ratio: latest_twcc_observation
                .as_ref()
                .map(|twcc| twcc.packet_loss_ratio),
            observation_id: self.bwe_observation_id,
        }
    }
}
