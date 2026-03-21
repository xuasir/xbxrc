use std::time::{Duration, Instant};

use crate::{
    runtime_stats_sink::RuntimeStatsSink,
    transport::rtc::bwe::policy::{
        classify_scenario_bitrate_band, resolve_target_remb_kbps,
        resolve_transport_policy_profile_kind,
    },
    transport::rtc::recovery::coordinator::RecoveryCoordinator,
    transport::rtc::recovery::startup::{resolve_session_phase, SessionPhase},
    XbxEngineWebRtcRuntimeConfig,
};

pub(crate) struct RtcBweObservation {
    pub actual_kbps: f64,
    pub fraction_lost: f64,
    pub rtt_ms: Option<f64>,
    pub transport_path: Option<String>,
    pub observed_remb_kbps: Option<u32>,
}

pub(crate) struct RtcBweState {
    last_sent_remb_kbps: u32,
    hybrid_ramp_cooldown_ticks: u8,
    bwe_observation_id: u64,
}

pub(crate) struct RtcBweEvaluation {
    pub target_remb_kbps: u32,
    pub decision_reason: String,
    pub session_phase: SessionPhase,
    pub transport_policy_profile: String,
    pub recovery_coupling_mode: String,
    pub recovery_coupling_summary: String,
    pub direct_gaming_bitrate_band: Option<String>,
    pub observation_id: u64,
}

impl RtcBweState {
    pub(crate) fn new(initial_last_sent_remb_kbps: u32) -> Self {
        Self {
            last_sent_remb_kbps: initial_last_sent_remb_kbps,
            hybrid_ramp_cooldown_ticks: 0,
            bwe_observation_id: 0,
        }
    }

    pub(crate) fn evaluate(
        &mut self,
        runtime_stats: &RuntimeStatsSink,
        config: &XbxEngineWebRtcRuntimeConfig,
        observation: &RtcBweObservation,
        bwe_stream_started_at: Instant,
        bwe_startup_grace: Duration,
    ) -> RtcBweEvaluation {
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
            config,
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

        RtcBweEvaluation {
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
            observation_id: self.bwe_observation_id,
        }
    }
}
