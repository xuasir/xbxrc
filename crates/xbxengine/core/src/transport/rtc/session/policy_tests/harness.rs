use super::super::RtcSessionPolicy;
use crate::api::backend::XbxEngineMediaRuntimeStats;
use crate::api::runtime::XbxEngineRuntimeConfig;
use crate::transport::rtc::facts::{
    ConnectionLifecycleStateFact, SessionCommand, TransportCommand,
};
use crate::transport::rtc::policy::display_supply::SchedulingDemandSignal;
use crate::transport::rtc::projection::{
    BweProjection, ConnectionProjection, DiagnosticsProjection, MediaProjection,
    RecoveryProjection, TransportSnapshot,
};
use crate::transport::rtc::session::actor::SessionPolicyHook;
use std::sync::{Arc, Mutex};
pub(super) fn transport_commands(commands: Vec<SessionCommand>) -> Vec<TransportCommand> {
    commands
        .into_iter()
        .filter_map(|command| match command {
            SessionCommand::Transport(command) => Some(command),
            SessionCommand::LocalDecoderReset { .. } => None,
        })
        .collect()
}
pub(super) struct RecoveryIntegrationHarness {
    pub(super) runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    pub(super) policy: RtcSessionPolicy,
    pub(super) next_version: u64,
}

impl RecoveryIntegrationHarness {
    pub(super) fn new(target_type: Option<xbxengine_protocol::XbxEngineTargetTypeDto>) -> Self {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        if let Ok(mut stats) = runtime_stats.lock() {
            stats.session_target_type = target_type;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
        }
        let policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
        Self {
            runtime_stats,
            policy,
            next_version: 1,
        }
    }

    pub(super) fn apply(
        &mut self,
        observed_at_ms: f64,
        lifecycle_state: ConnectionLifecycleStateFact,
        diagnosis: &str,
        frame_count: u64,
        update_stats: impl FnOnce(&mut XbxEngineMediaRuntimeStats),
    ) -> Vec<TransportCommand> {
        self.apply_with_recovery_observed_at(
            observed_at_ms,
            observed_at_ms,
            lifecycle_state,
            diagnosis,
            frame_count,
            update_stats,
        )
    }

    pub(super) fn apply_with_recovery_observed_at(
        &mut self,
        observed_at_ms: f64,
        recovery_observed_at_ms: f64,
        lifecycle_state: ConnectionLifecycleStateFact,
        diagnosis: &str,
        frame_count: u64,
        update_stats: impl FnOnce(&mut XbxEngineMediaRuntimeStats),
    ) -> Vec<TransportCommand> {
        if let Ok(mut stats) = self.runtime_stats.lock() {
            update_stats(&mut stats);
        }
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = lifecycle_state;
        connection.control_channel_open = true;
        connection.latest_transport_path = Some("Direct".to_string());
        connection.latest_rtt_ms = Some(42.0);
        connection.last_observed_at_ms = Some(observed_at_ms);
        let snapshot = TransportSnapshot::new(
            self.next_version,
            observed_at_ms,
            connection,
            MediaProjection {
                frame_count,
                ..MediaProjection::default()
            },
            RecoveryProjection {
                latest_diagnosis_label: Some(diagnosis.to_string()),
                pending_action: false,
                successful_action_count: 0,
                failed_action_count: 0,
                last_observed_at_ms: Some(recovery_observed_at_ms),
                ..Default::default()
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        self.next_version = self.next_version.saturating_add(1);
        self.policy
            .on_snapshot(&snapshot)
            .into_iter()
            .filter_map(|command| match command {
                SessionCommand::Transport(command) => Some(command),
                SessionCommand::LocalDecoderReset { .. } => None,
            })
            .collect()
    }

    /// 与 [`Self::apply`] 相同，但使用调用方提供的 `ConnectionProjection`（用于注入 stale / 无信令等连接快照）。
    pub(super) fn apply_with_connection_projection(
        &mut self,
        observed_at_ms: f64,
        connection: ConnectionProjection,
        diagnosis: &str,
        frame_count: u64,
        update_stats: impl FnOnce(&mut XbxEngineMediaRuntimeStats),
    ) -> Vec<TransportCommand> {
        if let Ok(mut stats) = self.runtime_stats.lock() {
            update_stats(&mut stats);
        }
        let snapshot = TransportSnapshot::new(
            self.next_version,
            observed_at_ms,
            connection,
            MediaProjection {
                frame_count,
                ..MediaProjection::default()
            },
            RecoveryProjection {
                latest_diagnosis_label: Some(diagnosis.to_string()),
                pending_action: false,
                successful_action_count: 0,
                failed_action_count: 0,
                last_observed_at_ms: Some(observed_at_ms),
                ..Default::default()
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        self.next_version = self.next_version.saturating_add(1);
        self.policy
            .on_snapshot(&snapshot)
            .into_iter()
            .filter_map(|command| match command {
                SessionCommand::Transport(command) => Some(command),
                SessionCommand::LocalDecoderReset { .. } => None,
            })
            .collect()
    }

    pub(super) fn with_stats<R>(&self, reader: impl FnOnce(&XbxEngineMediaRuntimeStats) -> R) -> R {
        let stats = self.runtime_stats.lock().expect("runtime stats lock");
        reader(&stats)
    }
}

pub(super) fn build_demand_for_stats(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> SchedulingDemandSignal {
    SchedulingDemandSignal {
        no_pending_pressure_level: stats.host_no_pending_pressure_level.clone(),
        no_pending_streak: Some(stats.host_no_pending_streak),
        present_age_ms: stats
            .latest_video_host_present_time_ms
            .map(|ts| (now_ms - ts).max(0.0)),
        decode_age_ms: stats
            .latest_video_decode_ok_time_ms
            .map(|ts| (now_ms - ts).max(0.0)),
        video_renderer_stalled: stats.video_renderer_stalled.unwrap_or(false),
        host_display_tick_epoch: Some(stats.host_display_tick_epoch),
        host_frame_present_epoch: Some(stats.host_frame_present_epoch),
        host_cadence_phase: stats.host_cadence_phase.clone(),
        host_mailbox_enqueue_count_total: Some(stats.host_mailbox_enqueue_count_total),
        host_mailbox_drop_count_total: Some(stats.host_mailbox_drop_count_total),
        host_mailbox_overwrite_count_total: Some(stats.host_mailbox_overwrite_count_total),
        pacer_submit_count_total: Some(stats.video_pacer_submit_count_total),
        pacer_drop_count_total: Some(stats.video_pacer_drop_count_total),
        renderer_submit_count_total: Some(stats.video_renderer_submit_count_total),
        renderer_drop_count_total: Some(stats.video_renderer_drop_count_total),
    }
}

pub(super) fn classify_supply_state_with_profile(
    stats: &XbxEngineMediaRuntimeStats,
) -> crate::transport::rtc::policy::display_supply::DisplaySupplyState {
    let profile =
        crate::transport::rtc::recovery::runtime_state::resolve_runtime_recovery_profile(stats);
    let now_ms = crate::transport::rtc::stats::now_ms_f64();
    let demand = build_demand_for_stats(stats, now_ms);
    demand.classify_display_supply_state(&profile.display_supply_thresholds)
}

pub(super) fn set_input_rumble_burst(
    stats: &mut XbxEngineMediaRuntimeStats,
    observation_id: u64,
    observed_at_ms: f64,
    payload_len: usize,
) {
    stats.latest_data_channel_message_catalog_observation =
        Some(crate::XbxEngineDataChannelMessageCatalogObservation {
            observation_id,
            direction: "inbound".to_string(),
            channel: "input".to_string(),
            kind_type: Some("ingress".to_string()),
            kind_message: Some("message".to_string()),
            target: Some("input".to_string()),
            keys: vec!["channel".to_string()],
            payload_len,
            observed_at_ms,
        });
}

pub(super) fn assert_recovery_family_hold_semantics(gate_result: &str, action_selected: &str) {
    let gate_lower = gate_result.to_ascii_lowercase();
    let action_lower = action_selected.to_ascii_lowercase();
    let matches_legacy_cooldown =
        gate_result == "suppressed:cooldownSuppressed" || action_selected == "cooldownSuppressed";
    let matches_family_coalesce =
        gate_lower.contains("coalesce") || action_lower.contains("coalesce");
    let matches_in_flight = gate_lower.contains("in-flight")
        || gate_lower.contains("inflight")
        || action_lower.contains("in-flight")
        || action_lower.contains("inflight");

    assert!(
        matches_legacy_cooldown || matches_family_coalesce || matches_in_flight,
        "expected recovery family hold semantics, got gate_result={gate_result}, action_selected={action_selected}"
    );
}

pub(super) fn build_snapshot(
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
        ..Default::default()
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
