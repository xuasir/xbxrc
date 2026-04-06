use super::{resolve_runtime_reconnect_reason_domain, RtcSessionPolicy};
use crate::api::backend::{XbxEngineMediaRuntimeStats, XbxEngineVideoTwccObservation};
use crate::api::runtime::XbxEngineRuntimeConfig;
use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::facts::{ConnectionLifecycleStateFact, TransportCommand};
use crate::transport::rtc::policy::display_supply::SchedulingDemandSignal;
use crate::transport::rtc::projection::{
    BweProjection, ConnectionProjection, DiagnosticsProjection, MediaProjection,
    RecoveryProjection, TransportSnapshot,
};
use crate::transport::rtc::recovery::escalation::{RecoveryAction, VideoEscalationReason};
use crate::transport::rtc::session::actor::SessionPolicyHook;
use std::sync::{Arc, Mutex};
use std::thread::sleep;
use std::time::Duration;

struct RecoveryIntegrationHarness {
    runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    policy: RtcSessionPolicy,
    next_version: u64,
}

impl RecoveryIntegrationHarness {
    fn new(target_type: Option<xbxengine_protocol::XbxEngineTargetTypeDto>) -> Self {
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

    fn apply(
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

    fn apply_with_recovery_observed_at(
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
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        self.next_version = self.next_version.saturating_add(1);
        self.policy.on_snapshot(&snapshot)
    }

    fn with_stats<R>(&self, reader: impl FnOnce(&XbxEngineMediaRuntimeStats) -> R) -> R {
        let stats = self.runtime_stats.lock().expect("runtime stats lock");
        reader(&stats)
    }
}

fn build_demand_for_stats(
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
        host_present_epoch: Some(stats.video_present_epoch),
        host_cadence_phase: stats.host_cadence_phase.clone(),
        present_submit_count_total: Some(stats.video_present_submit_count_total),
        present_drop_count_total: Some(stats.video_present_drop_count_total),
        present_overwrite_count_total: Some(stats.video_present_overwrite_count_total),
        pacer_submit_count_total: Some(stats.video_pacer_submit_count_total),
        pacer_drop_count_total: Some(stats.video_pacer_drop_count_total),
        renderer_submit_count_total: Some(stats.video_renderer_submit_count_total),
        renderer_drop_count_total: Some(stats.video_renderer_drop_count_total),
    }
}

fn classify_supply_state_with_profile(
    stats: &XbxEngineMediaRuntimeStats,
) -> crate::transport::rtc::policy::display_supply::DisplaySupplyState {
    let profile =
        crate::transport::rtc::recovery::runtime_state::resolve_runtime_recovery_profile(stats);
    let now_ms = crate::transport::rtc::stats::now_ms_f64();
    let demand = build_demand_for_stats(stats, now_ms);
    demand.classify_display_supply_state(&profile.display_supply_thresholds)
}

fn set_input_rumble_burst(
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

fn assert_recovery_family_hold_semantics(gate_result: &str, action_selected: &str) {
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

#[test]
fn reconnect_command_is_throttled_and_re_emitted_during_continuous_recovering() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
    let mut connection = ConnectionProjection::default();
    let mut recovery = RecoveryProjection::default();
    recovery.latest_diagnosis_label = Some("rtcPeerConnectionFailed".to_string());
    connection.lifecycle_state = ConnectionLifecycleStateFact::Recovering;
    let media = MediaProjection {
        frame_count: 1,
        ..MediaProjection::default()
    };

    let first = TransportSnapshot::new(
        1,
        100.0,
        connection.clone(),
        media.clone(),
        recovery.clone(),
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let first_commands = policy.on_snapshot(&first);
    assert!(first_commands
        .iter()
        .any(|command| { matches!(command, TransportCommand::RequestReconnectCandidate { .. }) }));

    let second = TransportSnapshot::new(
        2,
        1_200.0,
        connection.clone(),
        media.clone(),
        recovery.clone(),
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let second_commands = policy.on_snapshot(&second);
    assert!(second_commands
        .iter()
        .all(|command| { !matches!(command, TransportCommand::RequestReconnectCandidate { .. }) }));

    let third = TransportSnapshot::new(
        3,
        2_701.0,
        connection,
        media,
        recovery,
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let third_commands = policy.on_snapshot(&third);
    assert!(third_commands
        .iter()
        .any(|command| { matches!(command, TransportCommand::RequestReconnectCandidate { .. }) }));
}

#[test]
fn cloud_lifecycle_reconnect_interval_is_more_relaxed_than_non_cloud() {
    fn run_for_target(
        session_target_type: Option<xbxengine_protocol::XbxEngineTargetTypeDto>,
    ) -> Vec<Vec<TransportCommand>> {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        if let Ok(mut stats) = runtime_stats.lock() {
            stats.session_target_type = session_target_type;
        }
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::Recovering;
        let recovery = RecoveryProjection {
            latest_diagnosis_label: Some("rtcPeerConnectionFailed".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(100.0),
        };
        let media = MediaProjection {
            frame_count: 1,
            ..MediaProjection::default()
        };
        let timestamps = [100.0, 2_000.0];
        timestamps
            .into_iter()
            .enumerate()
            .map(|(idx, ts)| {
                let snapshot = TransportSnapshot::new(
                    (idx as u64) + 1,
                    ts,
                    connection.clone(),
                    media.clone(),
                    RecoveryProjection {
                        last_observed_at_ms: Some(ts),
                        ..recovery.clone()
                    },
                    BweProjection::default(),
                    DiagnosticsProjection::default(),
                );
                policy.on_snapshot(&snapshot)
            })
            .collect()
    }

    let home_commands = run_for_target(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Home));
    let cloud_commands = run_for_target(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

    assert!(home_commands[0]
        .iter()
        .any(|command| { matches!(command, TransportCommand::RequestReconnectCandidate { .. }) }));
    assert!(cloud_commands[0]
        .iter()
        .any(|command| { matches!(command, TransportCommand::RequestReconnectCandidate { .. }) }));
    assert!(home_commands[1]
        .iter()
        .any(|command| { matches!(command, TransportCommand::RequestReconnectCandidate { .. }) }));
    assert!(cloud_commands[1]
        .iter()
        .all(|command| { !matches!(command, TransportCommand::RequestReconnectCandidate { .. }) }));
}

#[test]
fn disconnected_surface_emits_lifecycle_reconnect_without_waiting_no_progress_timeout() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Disconnected;
    let recovery = RecoveryProjection {
        latest_diagnosis_label: Some("rtcControlChannelClosed".to_string()),
        pending_action: false,
        successful_action_count: 0,
        failed_action_count: 0,
        last_observed_at_ms: Some(100.0),
    };
    let first = TransportSnapshot::new(
        1,
        100.0,
        connection,
        MediaProjection::default(),
        recovery,
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let first_commands = policy.on_snapshot(&first);
    assert!(first_commands
        .iter()
        .any(|command| { matches!(command, TransportCommand::RequestReconnectCandidate { .. }) }));
    let stats = runtime_stats.lock().expect("runtime stats lock");
    let ledger = stats
        .latest_recovery_decision_ledger
        .as_ref()
        .expect("recovery decision ledger");
    assert_eq!(
        ledger.input_signal,
        "rtcConnectionRecovering:rtcConnectionDisconnected"
    );
}

#[test]
fn fallback_transport_await_recovery_keyframe_is_not_blocked_before_coordinator() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
    let snapshot = build_snapshot(
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryKeyframe",
        100.0,
    );
    let commands = policy.on_snapshot(&snapshot);
    assert!(commands
        .iter()
        .any(|command| matches!(command, TransportCommand::RequestKeyframe { .. })));
}

#[test]
fn connecting_startup_without_progress_triggers_lifecycle_reconnect() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.session_phase = Some("startup".to_string());
    }
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Connecting;
    let recovery = RecoveryProjection {
        latest_diagnosis_label: Some("none".to_string()),
        pending_action: false,
        successful_action_count: 0,
        failed_action_count: 0,
        last_observed_at_ms: Some(100.0),
    };
    let first = TransportSnapshot::new(
        1,
        100.0,
        connection.clone(),
        MediaProjection::default(),
        recovery.clone(),
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let first_commands = policy.on_snapshot(&first);
    assert!(first_commands
        .iter()
        .all(|command| { !matches!(command, TransportCommand::RequestReconnectCandidate { .. }) }));

    let second = TransportSnapshot::new(
        2,
        4_200.0,
        connection.clone(),
        MediaProjection::default(),
        RecoveryProjection {
            last_observed_at_ms: Some(4_200.0),
            ..recovery.clone()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let second_commands = policy.on_snapshot(&second);
    assert!(second_commands
        .iter()
        .all(|command| { !matches!(command, TransportCommand::RequestReconnectCandidate { .. }) }));

    let third = TransportSnapshot::new(
        3,
        15_600.0,
        connection.clone(),
        MediaProjection::default(),
        RecoveryProjection {
            last_observed_at_ms: Some(15_600.0),
            ..recovery.clone()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let third_commands = policy.on_snapshot(&third);
    assert!(third_commands
        .iter()
        .any(|command| { matches!(command, TransportCommand::RequestReconnectCandidate { .. }) }));

    let fourth = TransportSnapshot::new(
        4,
        16_200.0,
        connection.clone(),
        MediaProjection::default(),
        RecoveryProjection {
            last_observed_at_ms: Some(16_200.0),
            ..recovery.clone()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let fourth_commands = policy.on_snapshot(&fourth);
    assert!(fourth_commands
        .iter()
        .all(|command| { !matches!(command, TransportCommand::RequestReconnectCandidate { .. }) }));

    let fifth = TransportSnapshot::new(
        5,
        20_200.0,
        connection,
        MediaProjection::default(),
        RecoveryProjection {
            last_observed_at_ms: Some(20_200.0),
            ..recovery
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let fifth_commands = policy.on_snapshot(&fifth);
    assert!(fifth_commands
        .iter()
        .any(|command| { matches!(command, TransportCommand::RequestReconnectCandidate { .. }) }));
}

#[test]
fn connecting_seeking_anchor_without_progress_triggers_lifecycle_reconnect() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.video_owner_state = Some("seeking-anchor".to_string());
    }
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Connecting;
    let recovery = RecoveryProjection {
        latest_diagnosis_label: Some("none".to_string()),
        pending_action: false,
        successful_action_count: 0,
        failed_action_count: 0,
        last_observed_at_ms: Some(100.0),
    };
    let first = TransportSnapshot::new(
        1,
        100.0,
        connection.clone(),
        MediaProjection::default(),
        recovery.clone(),
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let first_commands = policy.on_snapshot(&first);
    assert!(first_commands
        .iter()
        .all(|command| { !matches!(command, TransportCommand::RequestReconnectCandidate { .. }) }));

    let second = TransportSnapshot::new(
        2,
        15_600.0,
        connection,
        MediaProjection::default(),
        RecoveryProjection {
            last_observed_at_ms: Some(15_600.0),
            ..recovery
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let second_commands = policy.on_snapshot(&second);
    assert!(second_commands
        .iter()
        .any(|command| { matches!(command, TransportCommand::RequestReconnectCandidate { .. }) }));
}

#[test]
fn connecting_without_semantic_hints_still_triggers_liveness_reconnect() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Connecting;
    let recovery = RecoveryProjection {
        latest_diagnosis_label: Some("none".to_string()),
        pending_action: false,
        successful_action_count: 0,
        failed_action_count: 0,
        last_observed_at_ms: Some(100.0),
    };
    let first = TransportSnapshot::new(
        1,
        100.0,
        connection.clone(),
        MediaProjection::default(),
        recovery.clone(),
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let first_commands = policy.on_snapshot(&first);
    assert!(first_commands
        .iter()
        .all(|command| { !matches!(command, TransportCommand::RequestReconnectCandidate { .. }) }));

    let second = TransportSnapshot::new(
        2,
        4_220.0,
        connection.clone(),
        MediaProjection::default(),
        RecoveryProjection {
            last_observed_at_ms: Some(4_220.0),
            ..recovery.clone()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let second_commands = policy.on_snapshot(&second);
    assert!(second_commands
        .iter()
        .all(|command| { !matches!(command, TransportCommand::RequestReconnectCandidate { .. }) }));

    let third = TransportSnapshot::new(
        3,
        15_600.0,
        connection,
        MediaProjection::default(),
        RecoveryProjection {
            last_observed_at_ms: Some(15_600.0),
            ..recovery
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let third_commands = policy.on_snapshot(&third);
    assert!(third_commands
        .iter()
        .any(|command| { matches!(command, TransportCommand::RequestReconnectCandidate { .. }) }));
}

#[test]
fn new_state_does_not_emit_liveness_reconnect_before_connecting() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::New;
    let recovery = RecoveryProjection {
        latest_diagnosis_label: Some("none".to_string()),
        pending_action: false,
        successful_action_count: 0,
        failed_action_count: 0,
        last_observed_at_ms: Some(100.0),
    };

    let first = TransportSnapshot::new(
        1,
        100.0,
        connection.clone(),
        MediaProjection::default(),
        recovery.clone(),
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let _ = policy.on_snapshot(&first);

    let second = TransportSnapshot::new(
        2,
        10_000.0,
        connection,
        MediaProjection::default(),
        RecoveryProjection {
            last_observed_at_ms: Some(10_000.0),
            ..recovery.clone()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let second_commands = policy.on_snapshot(&second);
    assert!(second_commands
        .iter()
        .all(|command| { !matches!(command, TransportCommand::RequestReconnectCandidate { .. }) }));

    let third = TransportSnapshot::new(
        3,
        15_600.0,
        ConnectionProjection {
            lifecycle_state: ConnectionLifecycleStateFact::New,
            ..ConnectionProjection::default()
        },
        MediaProjection::default(),
        RecoveryProjection {
            last_observed_at_ms: Some(15_600.0),
            ..recovery
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let third_commands = policy.on_snapshot(&third);
    assert!(third_commands
        .iter()
        .all(|command| { !matches!(command, TransportCommand::RequestReconnectCandidate { .. }) }));
}

#[test]
fn lifecycle_reconnect_attempt_limit_enters_failed_terminal() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Recovering;
    let recovery = RecoveryProjection {
        latest_diagnosis_label: Some("rtcPeerConnectionFailed".to_string()),
        pending_action: false,
        successful_action_count: 0,
        failed_action_count: 0,
        last_observed_at_ms: Some(100.0),
    };
    let media = MediaProjection {
        frame_count: 1,
        ..MediaProjection::default()
    };

    let first = TransportSnapshot::new(
        1,
        100.0,
        connection.clone(),
        media.clone(),
        recovery.clone(),
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let first_commands = policy.on_snapshot(&first);
    assert!(first_commands
        .iter()
        .any(|command| { matches!(command, TransportCommand::RequestReconnectCandidate { .. }) }));

    let second = TransportSnapshot::new(
        2,
        2_000.0,
        connection.clone(),
        media.clone(),
        RecoveryProjection {
            last_observed_at_ms: Some(2_000.0),
            ..recovery.clone()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let second_commands = policy.on_snapshot(&second);
    assert!(second_commands
        .iter()
        .any(|command| { matches!(command, TransportCommand::RequestReconnectCandidate { .. }) }));

    let third = TransportSnapshot::new(
        3,
        3_800.0,
        connection.clone(),
        media.clone(),
        RecoveryProjection {
            last_observed_at_ms: Some(3_800.0),
            ..recovery.clone()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let third_commands = policy.on_snapshot(&third);
    assert!(third_commands
        .iter()
        .any(|command| { matches!(command, TransportCommand::RequestReconnectCandidate { .. }) }));

    let fourth = TransportSnapshot::new(
        4,
        5_600.0,
        connection.clone(),
        media.clone(),
        RecoveryProjection {
            last_observed_at_ms: Some(5_600.0),
            ..recovery.clone()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let fourth_commands = policy.on_snapshot(&fourth);
    assert!(
        fourth_commands.is_empty(),
        "attempts exhausted should enter failed-terminal without emitting more commands"
    );

    let stats = runtime_stats.lock().expect("runtime stats lock");
    let ledger = stats
        .latest_recovery_decision_ledger
        .as_ref()
        .expect("recovery decision ledger");
    assert_eq!(ledger.state_after, "failed-terminal");
    assert_eq!(
        ledger.gate_result,
        "terminal:livenessReconnectAttemptLimitExceeded"
    );
    assert_eq!(ledger.action_selected, "failed-terminal");
    drop(stats);

    let fifth = TransportSnapshot::new(
        5,
        7_300.0,
        connection,
        media,
        RecoveryProjection {
            last_observed_at_ms: Some(7_300.0),
            ..recovery
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let fifth_commands = policy.on_snapshot(&fifth);
    assert!(fifth_commands.is_empty());
}

#[test]
fn failed_terminal_clears_after_successful_progress_and_rearms_reconnect() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Recovering;
    let base_recovery = RecoveryProjection {
        latest_diagnosis_label: Some("rtcPeerConnectionFailed".to_string()),
        pending_action: false,
        successful_action_count: 0,
        failed_action_count: 0,
        last_observed_at_ms: Some(100.0),
    };
    let media = MediaProjection {
        frame_count: 1,
        ..MediaProjection::default()
    };
    let timeline = [100.0, 2_000.0, 3_800.0, 5_600.0];
    for (idx, ts) in timeline.into_iter().enumerate() {
        let snapshot = TransportSnapshot::new(
            (idx as u64) + 1,
            ts,
            connection.clone(),
            media.clone(),
            RecoveryProjection {
                last_observed_at_ms: Some(ts),
                ..base_recovery.clone()
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let _ = policy.on_snapshot(&snapshot);
    }
    {
        let stats = runtime_stats.lock().expect("runtime stats lock");
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(ledger.state_after, "failed-terminal");
    }

    let resumed = TransportSnapshot::new(
        5,
        7_800.0,
        connection,
        MediaProjection {
            frame_count: 2,
            ..media
        },
        RecoveryProjection {
            last_observed_at_ms: Some(7_800.0),
            ..base_recovery
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let resumed_commands = policy.on_snapshot(&resumed);
    assert!(resumed_commands
        .iter()
        .any(|command| { matches!(command, TransportCommand::RequestReconnectCandidate { .. }) }));
    let stats = runtime_stats.lock().expect("runtime stats lock");
    let ledger = stats
        .latest_recovery_decision_ledger
        .as_ref()
        .expect("recovery decision ledger");
    assert_eq!(ledger.state_after, "reconnecting");
}

#[test]
fn no_progress_upper_bound_applies_to_connecting_and_recovering_surfaces() {
    let cases = [
        (
            ConnectionLifecycleStateFact::Connecting,
            Some("none".to_string()),
        ),
        (
            ConnectionLifecycleStateFact::Recovering,
            Some("rtcPeerConnectionFailed".to_string()),
        ),
    ];
    for (idx, (lifecycle_state, diagnosis)) in cases.into_iter().enumerate() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = lifecycle_state;
        let recovery = RecoveryProjection {
            latest_diagnosis_label: diagnosis,
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(100.0),
        };
        let media = MediaProjection {
            frame_count: if lifecycle_state == ConnectionLifecycleStateFact::Recovering {
                1
            } else {
                0
            },
            ..MediaProjection::default()
        };
        let first = TransportSnapshot::new(
            ((idx as u64) * 10) + 1,
            100.0,
            connection.clone(),
            media.clone(),
            recovery.clone(),
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let _ = policy.on_snapshot(&first);
        let second_ts = if lifecycle_state == ConnectionLifecycleStateFact::Connecting {
            15_600.0
        } else {
            4_300.0
        };
        let second = TransportSnapshot::new(
            ((idx as u64) * 10) + 2,
            second_ts,
            connection,
            media,
            RecoveryProjection {
                last_observed_at_ms: Some(second_ts),
                ..recovery
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let second_commands = policy.on_snapshot(&second);
        assert!(
            second_commands.iter().any(|command| {
                matches!(command, TransportCommand::RequestReconnectCandidate { .. })
            }),
            "case idx={} should emit reconnect under no-progress upper bound",
            idx
        );
    }
}

#[test]
fn pre_first_frame_transport_progress_uses_relaxed_liveness_timeout() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Connecting;
    connection.latest_transport_path = Some("Direct".to_string());
    connection.latest_rtt_ms = Some(9.0);
    let media = MediaProjection {
        frame_count: 0,
        ..MediaProjection::default()
    };
    let recovery = RecoveryProjection {
        latest_diagnosis_label: Some("none".to_string()),
        pending_action: false,
        successful_action_count: 0,
        failed_action_count: 0,
        last_observed_at_ms: Some(100.0),
    };

    let first = TransportSnapshot::new(
        1,
        100.0,
        connection.clone(),
        media.clone(),
        recovery.clone(),
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let _ = policy.on_snapshot(&first);

    let second = TransportSnapshot::new(
        2,
        4_300.0,
        connection.clone(),
        media.clone(),
        RecoveryProjection {
            last_observed_at_ms: Some(4_300.0),
            ..recovery.clone()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let second_commands = policy.on_snapshot(&second);
    assert!(
        second_commands.iter().all(|command| {
            !matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }),
        "transport 已有进展但尚未首帧时，不应在 4s 上界内过早重连"
    );

    let third = TransportSnapshot::new(
        3,
        15_600.0,
        connection,
        media,
        RecoveryProjection {
            last_observed_at_ms: Some(15_600.0),
            ..recovery
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let third_commands = policy.on_snapshot(&third);
    assert!(third_commands
        .iter()
        .any(|command| { matches!(command, TransportCommand::RequestReconnectCandidate { .. }) }));
}

#[test]
fn recovering_pre_first_frame_without_transport_progress_uses_relaxed_liveness_timeout() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Recovering;
    let media = MediaProjection {
        frame_count: 0,
        ..MediaProjection::default()
    };
    let recovery = RecoveryProjection {
        latest_diagnosis_label: Some("none".to_string()),
        pending_action: false,
        successful_action_count: 0,
        failed_action_count: 0,
        last_observed_at_ms: Some(100.0),
    };

    let first = TransportSnapshot::new(
        1,
        100.0,
        connection.clone(),
        media.clone(),
        recovery.clone(),
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let _ = policy.on_snapshot(&first);

    let second = TransportSnapshot::new(
        2,
        4_300.0,
        connection.clone(),
        media.clone(),
        RecoveryProjection {
            last_observed_at_ms: Some(4_300.0),
            ..recovery.clone()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let second_commands = policy.on_snapshot(&second);
    assert!(
        second_commands.iter().all(|command| {
            !matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }),
        "首帧前即便尚无 transport 进展，也不应在 4s 内过早重连"
    );

    let third = TransportSnapshot::new(
        3,
        15_600.0,
        connection,
        media,
        RecoveryProjection {
            last_observed_at_ms: Some(15_600.0),
            ..recovery
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let third_commands = policy.on_snapshot(&third);
    assert!(third_commands
        .iter()
        .any(|command| { matches!(command, TransportCommand::RequestReconnectCandidate { .. }) }));
}

#[test]
fn pre_first_frame_display_supply_degraded_does_not_upgrade_recovery() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Home));

    let commands = harness.apply(
        10_120.0,
        ConnectionLifecycleStateFact::Connected,
        "none",
        0,
        |stats| {
            stats.session_phase = Some("startup".to_string());
            stats.transport_recovery_epoch = 3;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 96;
            stats.latest_video_host_present_time_ms = Some(9_306.0);
            stats.latest_video_decode_ok_time_ms = Some(10_116.0);
            stats.latest_video_packet_arrival_time_ms = Some(10_118.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 108_000,
                video_packet_count_total: 960,
                audio_bytes_total: 28_000,
                observed_at_ms: 10_118.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-complete-candidate".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        observed_at_ms: 10_118.0,
                    },
                    observed_at_ms: 10_118.0,
                });
        },
    );

    assert!(commands
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })));
    assert!(commands
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestDecoderReset { .. })));
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_ne!(ledger.action_selected, "requestReconnectCandidate");
        assert_ne!(ledger.action_selected, "requestDecoderReset");
    });
}

#[test]
fn cloud_early_connecting_without_builder_waits_for_long_terminal_window() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud);
    }
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Connecting;
    let recovery = RecoveryProjection {
        latest_diagnosis_label: Some("none".to_string()),
        pending_action: false,
        successful_action_count: 0,
        failed_action_count: 0,
        last_observed_at_ms: Some(100.0),
    };
    let media = MediaProjection::default();

    let first = TransportSnapshot::new(
        1,
        100.0,
        connection.clone(),
        media.clone(),
        recovery.clone(),
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let first_commands = policy.on_snapshot(&first);
    assert!(first_commands
        .iter()
        .all(|command| { !matches!(command, TransportCommand::RequestReconnectCandidate { .. }) }));

    let second = TransportSnapshot::new(
        2,
        15_600.0,
        connection.clone(),
        media.clone(),
        RecoveryProjection {
            last_observed_at_ms: Some(15_600.0),
            ..recovery.clone()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let second_commands = policy.on_snapshot(&second);
    assert!(second_commands
        .iter()
        .all(|command| { !matches!(command, TransportCommand::RequestReconnectCandidate { .. }) }));
    let stats = runtime_stats.lock().expect("runtime stats lock");
    assert!(
        stats.latest_rtc_builder_observation.is_none(),
        "early connecting soft hold 应在 builder 尚未出现时就生效"
    );
    drop(stats);

    let third = TransportSnapshot::new(
        3,
        35_600.0,
        connection.clone(),
        media.clone(),
        RecoveryProjection {
            last_observed_at_ms: Some(35_600.0),
            ..recovery.clone()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let third_commands = policy.on_snapshot(&third);
    assert!(third_commands
        .iter()
        .any(|command| { matches!(command, TransportCommand::RequestReconnectCandidate { .. }) }));

    let fourth = TransportSnapshot::new(
        4,
        38_200.0,
        connection.clone(),
        media.clone(),
        RecoveryProjection {
            last_observed_at_ms: Some(38_200.0),
            ..recovery.clone()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let fourth_commands = policy.on_snapshot(&fourth);
    assert!(
        fourth_commands.iter().all(|command| {
            !matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }),
        "connecting + 首帧前应按更长间隔节流 reconnect"
    );

    let reconnect_ticks = [40_200.0, 44_800.0, 49_400.0, 53_900.0, 58_400.0];
    for (idx, ts) in reconnect_ticks.into_iter().enumerate() {
        let snapshot = TransportSnapshot::new(
            (idx as u64) + 5,
            ts,
            connection.clone(),
            media.clone(),
            RecoveryProjection {
                last_observed_at_ms: Some(ts),
                ..recovery.clone()
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let commands = policy.on_snapshot(&snapshot);
        assert!(
            commands.iter().any(|command| {
                matches!(command, TransportCommand::RequestReconnectCandidate { .. })
            }),
            "cloud 在长窗口内应继续允许第 {} 次无进展 reconnect 尝试",
            idx + 2
        );
    }

    let terminal = TransportSnapshot::new(
        10,
        90_200.0,
        connection,
        media,
        RecoveryProjection {
            last_observed_at_ms: Some(90_200.0),
            ..recovery
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let terminal_commands = policy.on_snapshot(&terminal);
    assert!(
        terminal_commands.is_empty(),
        "cloud 只有超过长窗口后才允许进入 failed-terminal"
    );
    let stats = runtime_stats.lock().expect("runtime stats lock");
    let ledger = stats
        .latest_recovery_decision_ledger
        .as_ref()
        .expect("recovery decision ledger");
    assert_eq!(
        ledger.gate_result,
        "terminal:livenessReconnectAttemptLimitExceeded"
    );
    assert_eq!(ledger.state_after, "failed-terminal");
}

#[test]
fn cloud_early_new_without_builder_does_not_emit_liveness_reconnect_candidates() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud);
    }
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::New;
    let recovery = RecoveryProjection {
        latest_diagnosis_label: Some("none".to_string()),
        pending_action: false,
        successful_action_count: 0,
        failed_action_count: 0,
        last_observed_at_ms: Some(100.0),
    };
    let media = MediaProjection::default();

    let first = TransportSnapshot::new(
        1,
        100.0,
        connection.clone(),
        media.clone(),
        recovery.clone(),
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let first_commands = policy.on_snapshot(&first);
    assert!(first_commands
        .iter()
        .all(|command| { !matches!(command, TransportCommand::RequestReconnectCandidate { .. }) }));

    for (idx, ts) in [
        15_600.0, 35_600.0, 40_200.0, 44_800.0, 49_400.0, 53_900.0, 58_400.0,
    ]
    .into_iter()
    .enumerate()
    {
        let snapshot = TransportSnapshot::new(
            (idx as u64) + 2,
            ts,
            connection.clone(),
            media.clone(),
            RecoveryProjection {
                last_observed_at_ms: Some(ts),
                ..recovery.clone()
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let commands = policy.on_snapshot(&snapshot);
        assert!(
            commands.iter().all(|command| {
                !matches!(command, TransportCommand::RequestReconnectCandidate { .. })
            }),
            "cloud new 首窗在进入 Connecting 前不应发第 {} 次 liveness reconnect 候选",
            idx + 1
        );
    }

    let pre_terminal = TransportSnapshot::new(
        8,
        58_500.0,
        connection.clone(),
        media.clone(),
        RecoveryProjection {
            last_observed_at_ms: Some(58_500.0),
            ..recovery.clone()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let pre_terminal_commands = policy.on_snapshot(&pre_terminal);
    assert!(
        pre_terminal_commands.iter().all(|command| {
            !matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }),
        "New 首窗应彻底禁止 liveness reconnect 候选"
    );
    let stats = runtime_stats.lock().expect("runtime stats lock");
    assert!(
        stats.latest_rtc_builder_observation.is_none(),
        "cloud new 首窗 soft hold 应在 builder 尚未出现时就生效"
    );

    let long_new = TransportSnapshot::new(
        9,
        90_200.0,
        connection,
        media,
        RecoveryProjection {
            last_observed_at_ms: Some(90_200.0),
            ..recovery
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let long_new_commands = policy.on_snapshot(&long_new);
    assert!(long_new_commands
        .iter()
        .all(|command| { !matches!(command, TransportCommand::RequestReconnectCandidate { .. }) }));
}

#[test]
fn cloud_early_recovering_without_builder_waits_for_long_terminal_window() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud);
    }
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Recovering;
    let recovery = RecoveryProjection {
        latest_diagnosis_label: Some("rtcPeerConnectionFailed".to_string()),
        pending_action: false,
        successful_action_count: 0,
        failed_action_count: 0,
        last_observed_at_ms: Some(100.0),
    };
    let media = MediaProjection::default();

    let first = TransportSnapshot::new(
        1,
        100.0,
        connection.clone(),
        media.clone(),
        recovery.clone(),
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let _ = policy.on_snapshot(&first);

    let second = TransportSnapshot::new(
        2,
        15_600.0,
        connection.clone(),
        media.clone(),
        RecoveryProjection {
            last_observed_at_ms: Some(15_600.0),
            ..recovery.clone()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let second_commands = policy.on_snapshot(&second);
    assert!(
        second_commands.iter().all(|command| {
            !matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }),
        "cloud recovering 首窗在长窗口前不应进入 reconnect"
    );

    let third = TransportSnapshot::new(
        3,
        35_600.0,
        connection.clone(),
        media.clone(),
        RecoveryProjection {
            last_observed_at_ms: Some(35_600.0),
            ..recovery.clone()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let third_commands = policy.on_snapshot(&third);
    assert!(third_commands
        .iter()
        .any(|command| { matches!(command, TransportCommand::RequestReconnectCandidate { .. }) }));

    for (idx, ts) in [38_200.0, 40_800.0, 43_400.0, 46_000.0, 48_600.0]
        .into_iter()
        .enumerate()
    {
        let snapshot = TransportSnapshot::new(
            (idx as u64) + 4,
            ts,
            connection.clone(),
            media.clone(),
            RecoveryProjection {
                last_observed_at_ms: Some(ts),
                ..recovery.clone()
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let commands = policy.on_snapshot(&snapshot);
        assert!(
            commands.iter().any(|command| {
                matches!(command, TransportCommand::RequestReconnectCandidate { .. })
            }),
            "cloud recovering 首窗长窗口内应继续允许无进展 reconnect 尝试，idx={}",
            idx
        );
    }

    let terminal = TransportSnapshot::new(
        9,
        90_200.0,
        connection,
        media,
        RecoveryProjection {
            last_observed_at_ms: Some(90_200.0),
            ..recovery
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let terminal_commands = policy.on_snapshot(&terminal);
    assert!(
        terminal_commands.is_empty(),
        "cloud recovering 首窗超过长窗口后应进入 failed-terminal"
    );
    let stats = runtime_stats.lock().expect("runtime stats lock");
    let ledger = stats
        .latest_recovery_decision_ledger
        .as_ref()
        .expect("recovery decision ledger");
    assert_eq!(
        ledger.gate_result,
        "terminal:livenessReconnectAttemptLimitExceeded"
    );
    assert_eq!(ledger.state_after, "failed-terminal");
}

#[test]
fn cloud_hard_disconnect_reconnect_budget_exhaustion_enters_failed_terminal_without_spinning() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud);
        stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Disconnected;
        stats.session_phase = Some("startup".to_string());
    }
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Recovering;
    let recovery = RecoveryProjection {
        latest_diagnosis_label: Some("rtcPeerConnectionFailed".to_string()),
        pending_action: false,
        successful_action_count: 0,
        failed_action_count: 0,
        last_observed_at_ms: Some(100.0),
    };
    let media = MediaProjection::default();

    let warmup = TransportSnapshot::new(
        1,
        100.0,
        connection.clone(),
        media.clone(),
        recovery.clone(),
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    assert!(
        policy.on_snapshot(&warmup).is_empty(),
        "cloud hard disconnect should respect long reconnect warmup window"
    );

    let first_reconnect = TransportSnapshot::new(
        2,
        35_600.0,
        connection.clone(),
        media.clone(),
        RecoveryProjection {
            last_observed_at_ms: Some(35_600.0),
            ..recovery.clone()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let first_reconnect_commands = policy.on_snapshot(&first_reconnect);
    assert!(first_reconnect_commands
        .iter()
        .any(|command| matches!(command, TransportCommand::RequestReconnectCandidate { .. })));

    for (idx, ts) in [38_200.0, 40_800.0, 43_400.0, 46_000.0, 48_600.0]
        .into_iter()
        .enumerate()
    {
        let snapshot = TransportSnapshot::new(
            (idx as u64) + 3,
            ts,
            connection.clone(),
            media.clone(),
            RecoveryProjection {
                last_observed_at_ms: Some(ts),
                ..recovery.clone()
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let commands = policy.on_snapshot(&snapshot);
        assert!(
            commands.iter().any(|command| {
                matches!(command, TransportCommand::RequestReconnectCandidate { .. })
            }),
            "cloud hard disconnect should still allow reconnect before terminal, idx={idx}"
        );
    }

    let terminal = TransportSnapshot::new(
        8,
        90_200.0,
        connection.clone(),
        media.clone(),
        RecoveryProjection {
            last_observed_at_ms: Some(90_200.0),
            ..recovery.clone()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let terminal_commands = policy.on_snapshot(&terminal);
    assert!(
        terminal_commands.is_empty(),
        "cloud hard disconnect should enter failed-terminal after budget exhaustion"
    );
    let post_terminal = TransportSnapshot::new(
        9,
        92_800.0,
        connection,
        media,
        RecoveryProjection {
            last_observed_at_ms: Some(92_800.0),
            ..recovery
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let post_terminal_commands = policy.on_snapshot(&post_terminal);
    assert!(
        post_terminal_commands.is_empty(),
        "failed-terminal after hard disconnect should stop spinning"
    );

    let stats = runtime_stats.lock().expect("runtime stats lock");
    let ledger = stats
        .latest_recovery_decision_ledger
        .as_ref()
        .expect("cloud hard disconnect terminal ledger");
    assert_eq!(
        ledger.gate_result,
        "terminal:livenessReconnectAttemptLimitExceeded"
    );
    assert_eq!(ledger.state_after, "failed-terminal");
    assert_eq!(ledger.action_selected, "failed-terminal");
}

#[test]
fn connecting_without_target_type_keeps_reconnecting_before_long_terminal_window() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Connecting;
    let recovery = RecoveryProjection {
        latest_diagnosis_label: Some("none".to_string()),
        pending_action: false,
        successful_action_count: 0,
        failed_action_count: 0,
        last_observed_at_ms: Some(100.0),
    };
    let media = MediaProjection::default();

    for (idx, ts) in [100.0, 15_600.0, 20_200.0, 24_800.0]
        .into_iter()
        .enumerate()
    {
        let snapshot = TransportSnapshot::new(
            (idx as u64) + 1,
            ts,
            connection.clone(),
            media.clone(),
            RecoveryProjection {
                last_observed_at_ms: Some(ts),
                ..recovery.clone()
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let commands = policy.on_snapshot(&snapshot);
        if idx == 0 {
            assert!(commands.iter().all(|command| {
                !matches!(command, TransportCommand::RequestReconnectCandidate { .. })
            }));
        } else {
            assert!(commands.iter().any(|command| {
                matches!(command, TransportCommand::RequestReconnectCandidate { .. })
            }));
        }
    }

    let stats = runtime_stats.lock().expect("runtime stats lock");
    let ledger = stats
        .latest_recovery_decision_ledger
        .as_ref()
        .expect("recovery decision ledger");
    assert_ne!(ledger.state_after, "failed-terminal");
    assert_ne!(
        ledger.gate_result,
        "terminal:livenessReconnectAttemptLimitExceeded"
    );
    drop(stats);

    // target_type 未决的首窗也要遵循长窗口，超过阈值后仍应进入 terminal，避免无限重试。
    let terminal = TransportSnapshot::new(
        5,
        90_200.0,
        connection,
        media,
        RecoveryProjection {
            last_observed_at_ms: Some(90_200.0),
            ..recovery
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let terminal_commands = policy.on_snapshot(&terminal);
    assert!(
        terminal_commands.is_empty(),
        "target_type 缺失场景超过长窗口后应进入 failed-terminal"
    );
    let stats = runtime_stats.lock().expect("runtime stats lock");
    let ledger = stats
        .latest_recovery_decision_ledger
        .as_ref()
        .expect("recovery decision ledger");
    assert_eq!(
        ledger.gate_result,
        "terminal:livenessReconnectAttemptLimitExceeded"
    );
    assert_eq!(ledger.state_after, "failed-terminal");
}

#[test]
fn recovering_without_first_frame_does_not_emit_periodic_reconnect() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Recovering;
    let media = MediaProjection {
        frame_count: 0,
        ..MediaProjection::default()
    };
    let recovery = RecoveryProjection {
        latest_diagnosis_label: Some("rtcPeerConnectionFailed".to_string()),
        pending_action: false,
        successful_action_count: 0,
        failed_action_count: 0,
        last_observed_at_ms: Some(100.0),
    };

    let first = TransportSnapshot::new(
        1,
        100.0,
        connection.clone(),
        media.clone(),
        recovery.clone(),
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let _ = policy.on_snapshot(&first);

    let second = TransportSnapshot::new(
        2,
        2_000.0,
        connection.clone(),
        media.clone(),
        RecoveryProjection {
            last_observed_at_ms: Some(2_000.0),
            ..recovery.clone()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let second_commands = policy.on_snapshot(&second);
    assert!(
        second_commands.iter().all(|command| {
            !matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }),
        "Recovering + 首帧前不应按 1.5s 节流周期反复触发 reconnect"
    );
}

#[test]
fn liveness_uses_snapshot_now_when_last_observed_stalls() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Connecting;
    let recovery = RecoveryProjection {
        latest_diagnosis_label: Some("none".to_string()),
        pending_action: false,
        successful_action_count: 0,
        failed_action_count: 0,
        last_observed_at_ms: Some(100.0),
    };
    let media = MediaProjection {
        frame_count: 0,
        ..MediaProjection::default()
    };

    let first = TransportSnapshot::new(
        1,
        100.0,
        connection.clone(),
        media.clone(),
        recovery.clone(),
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let _ = policy.on_snapshot(&first);

    // 模拟 recovery.last_observed_at_ms 卡住不变，但 snapshot.now_ms 持续推进。
    let second = TransportSnapshot::new(
        2,
        15_600.0,
        connection,
        media,
        recovery,
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let second_commands = policy.on_snapshot(&second);
    assert!(second_commands
        .iter()
        .any(|command| { matches!(command, TransportCommand::RequestReconnectCandidate { .. }) }));
}

#[test]
fn command_success_without_frames_does_not_reset_liveness_budget() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Home);
    }
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Connecting;
    let base_recovery = RecoveryProjection {
        latest_diagnosis_label: Some("none".to_string()),
        pending_action: false,
        successful_action_count: 0,
        failed_action_count: 0,
        last_observed_at_ms: Some(100.0),
    };
    let media = MediaProjection {
        frame_count: 0,
        ..MediaProjection::default()
    };

    let first = TransportSnapshot::new(
        1,
        100.0,
        connection.clone(),
        media.clone(),
        base_recovery.clone(),
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let first_commands = policy.on_snapshot(&first);
    assert!(first_commands
        .iter()
        .all(|command| { !matches!(command, TransportCommand::RequestReconnectCandidate { .. }) }));

    let second = TransportSnapshot::new(
        2,
        15_600.0,
        connection.clone(),
        media.clone(),
        RecoveryProjection {
            successful_action_count: 1,
            last_observed_at_ms: Some(15_600.0),
            ..base_recovery.clone()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let second_commands = policy.on_snapshot(&second);
    assert!(second_commands
        .iter()
        .any(|command| { matches!(command, TransportCommand::RequestReconnectCandidate { .. }) }));

    let third = TransportSnapshot::new(
        3,
        20_200.0,
        connection.clone(),
        media.clone(),
        RecoveryProjection {
            successful_action_count: 2,
            last_observed_at_ms: Some(20_200.0),
            ..base_recovery.clone()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let third_commands = policy.on_snapshot(&third);
    assert!(third_commands
        .iter()
        .any(|command| { matches!(command, TransportCommand::RequestReconnectCandidate { .. }) }));

    let fourth = TransportSnapshot::new(
        4,
        24_800.0,
        connection.clone(),
        media,
        RecoveryProjection {
            successful_action_count: 3,
            last_observed_at_ms: Some(24_800.0),
            ..base_recovery
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let fourth_commands = policy.on_snapshot(&fourth);
    assert!(
        fourth_commands.iter().any(|command| {
            matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }),
        "third no-progress reconnect is still allowed before terminal closes the loop"
    );

    let fifth = TransportSnapshot::new(
        5,
        90_200.0,
        connection,
        MediaProjection::default(),
        RecoveryProjection {
            successful_action_count: 4,
            last_observed_at_ms: Some(90_200.0),
            ..RecoveryProjection::default()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let fifth_commands = policy.on_snapshot(&fifth);
    assert!(
        fifth_commands.is_empty(),
        "no media progress should still exhaust liveness attempts and stop reconnect loop"
    );
    let stats = runtime_stats.lock().expect("runtime stats lock");
    let ledger = stats
        .latest_recovery_decision_ledger
        .as_ref()
        .expect("recovery decision ledger");
    assert_eq!(ledger.state_after, "failed-terminal");
    assert_eq!(
        ledger.gate_result,
        "terminal:livenessReconnectAttemptLimitExceeded"
    );
}

#[test]
fn connected_ingress_progress_without_present_progress_does_not_force_reconnect() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.host_no_pending_pressure_level = Some("critical".to_string());
        stats.host_no_pending_streak = 260;
        // 这里使用与 snapshot 同一时间轴，避免“墙钟时间”与“策略时间”混用。
        stats.latest_video_host_present_time_ms = Some(0.0);
        stats.latest_video_decode_ok_time_ms = Some(0.0);
        stats.inbound_primary_video_bytes_total = 1_000;
    }
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
    connection.control_channel_open = true;
    connection.latest_transport_path = Some("relay/udp".to_string());
    connection.latest_rtt_ms = Some(48.0);
    connection.last_observed_at_ms = Some(100.0);
    let recovery = RecoveryProjection {
        latest_diagnosis_label: Some("none".to_string()),
        pending_action: false,
        successful_action_count: 0,
        failed_action_count: 0,
        last_observed_at_ms: Some(100.0),
    };

    let first = TransportSnapshot::new(
        1,
        100.0,
        connection.clone(),
        MediaProjection {
            frame_count: 10,
            ..MediaProjection::default()
        },
        recovery.clone(),
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let first_commands = policy.on_snapshot(&first);
    assert!(first_commands
        .iter()
        .all(|command| { !matches!(command, TransportCommand::RequestReconnectCandidate { .. }) }));

    if let Ok(mut stats) = runtime_stats.lock() {
        stats.inbound_primary_video_bytes_total = 2_000;
    }
    let second = TransportSnapshot::new(
        2,
        5_000.0,
        connection.clone(),
        MediaProjection {
            frame_count: 11,
            ..MediaProjection::default()
        },
        RecoveryProjection {
            last_observed_at_ms: Some(5_000.0),
            ..recovery.clone()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let second_commands = policy.on_snapshot(&second);
    assert!(second_commands
        .iter()
        .all(|command| { !matches!(command, TransportCommand::RequestReconnectCandidate { .. }) }));

    if let Ok(mut stats) = runtime_stats.lock() {
        stats.inbound_primary_video_bytes_total = 3_000;
    }
    let third = TransportSnapshot::new(
        3,
        10_400.0,
        connection,
        MediaProjection {
            frame_count: 12,
            ..MediaProjection::default()
        },
        RecoveryProjection {
            last_observed_at_ms: Some(10_400.0),
            ..recovery
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let third_commands = policy.on_snapshot(&third);
    assert!(third_commands
        .iter()
        .all(|command| { !matches!(command, TransportCommand::RequestReconnectCandidate { .. }) }));
    let stats = runtime_stats.lock().expect("runtime stats lock");
    let ledger = stats
        .latest_recovery_decision_ledger
        .as_ref()
        .expect("recovery decision ledger");
    assert_ne!(ledger.action_selected, "requestReconnectCandidate");
}

#[test]
fn recovery_decision_ledger_is_written_with_budget_snapshot() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    let snapshot = build_snapshot(
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryKeyframe",
        320.0,
    );
    let commands = policy.on_snapshot(&snapshot);
    assert!(commands
        .iter()
        .any(|command| matches!(command, TransportCommand::RequestKeyframe { .. })));

    let stats = runtime_stats.lock().expect("runtime stats lock");
    let ledger = stats
        .latest_recovery_decision_ledger
        .as_ref()
        .expect("recovery decision ledger");
    assert_eq!(
        ledger.input_signal,
        "transportAwaitRecoveryKeyframe:transportAwaitRecoveryKeyframe"
    );
    assert_eq!(ledger.action_selected, "requestKeyframe");
    assert_eq!(ledger.gate_result, "pass");
    assert!(ledger.budget_before.is_some());
    assert!(ledger.budget_after.is_some());
    assert_eq!(ledger.command_result, None);
}

#[test]
fn recovery_decision_ledger_keeps_pending_action_latest_while_recent_history_records_no_signal() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());

    let first = build_snapshot(
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryKeyframe",
        320.0,
    );
    let first_commands = policy.on_snapshot(&first);
    assert!(first_commands
        .iter()
        .any(|command| matches!(command, TransportCommand::RequestKeyframe { .. })));
    let first_decision_id = runtime_stats
        .lock()
        .expect("runtime stats lock")
        .latest_recovery_decision_ledger
        .as_ref()
        .expect("recovery decision ledger")
        .decision_id;

    // 下一 tick 明确无恢复信号时，也必须写入新的 ledger，保证观测连续完整。
    let second = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 340.0);
    let _ = policy.on_snapshot(&second);
    let stats = runtime_stats.lock().expect("runtime stats lock");
    let latest = stats
        .latest_recovery_decision_ledger
        .as_ref()
        .expect("recovery decision ledger");
    assert_eq!(latest.decision_id, first_decision_id);
    assert_eq!(latest.action_selected, "requestKeyframe");
    assert_eq!(latest.command_result, None);
    let recent = stats
        .recent_recovery_decision_ledgers
        .last()
        .expect("recent recovery ledger");
    assert_ne!(recent.decision_id, first_decision_id);
    assert_eq!(recent.input_signal, "none");
    assert_eq!(recent.gate_result, "no-signal");
    assert_eq!(recent.action_selected, "none");
}

#[test]
fn recovery_decision_ledger_allows_no_signal_to_be_latest_after_pending_command_is_resolved() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());

    let first = build_snapshot(
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryKeyframe",
        320.0,
    );
    let first_commands = policy.on_snapshot(&first);
    assert!(first_commands
        .iter()
        .any(|command| matches!(command, TransportCommand::RequestKeyframe { .. })));
    RuntimeStatsSink::update_shared(runtime_stats.as_ref(), |stats| {
        if let Some(ledger) = stats.latest_recovery_decision_ledger.as_mut() {
            ledger.command_result = Some("deferred".to_string());
        }
        if let Some(ledger) = stats.recent_recovery_decision_ledgers.last_mut() {
            ledger.command_result = Some("deferred".to_string());
        }
    });

    let second = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 340.0);
    let _ = policy.on_snapshot(&second);
    let stats = runtime_stats.lock().expect("runtime stats lock");
    let latest = stats
        .latest_recovery_decision_ledger
        .as_ref()
        .expect("recovery decision ledger");
    assert_eq!(latest.input_signal, "none");
    assert_eq!(latest.gate_result, "no-signal");
    assert_eq!(latest.action_selected, "none");
}

#[test]
fn high_no_pending_but_fresh_present_does_not_force_keyframe() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        stats.host_no_pending_pressure_level = Some("high".to_string());
        stats.host_no_pending_streak = 88;
        stats.latest_video_host_present_time_ms = Some(now_ms - 14.0);
        stats.latest_video_decode_ok_time_ms = Some(now_ms - 10.0);
        stats.video_renderer_stalled = Some(false);
    }
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
    let snapshot = build_snapshot(
        ConnectionLifecycleStateFact::Connected,
        "waitKeyframeEntered",
        220.0,
    );
    let commands = policy.on_snapshot(&snapshot);
    assert!(commands
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestKeyframe { .. })));
}

#[test]
fn critical_display_supply_uses_recovery_controller_budget() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        stats.host_no_pending_pressure_level = Some("critical".to_string());
        stats.host_no_pending_streak = 220;
        stats.latest_video_host_present_time_ms = Some(now_ms - 980.0);
        stats.latest_video_decode_ok_time_ms = Some(now_ms - 520.0);
        stats.video_renderer_stalled = Some(true);
    }
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    let mut snapshot = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 360.0);
    let first = policy.on_snapshot(&snapshot);
    assert!(first
        .iter()
        .any(|command| matches!(command, TransportCommand::RequestKeyframe { .. })));

    snapshot.version = 2;
    snapshot.now_ms = 361.0;
    snapshot.recovery.last_observed_at_ms = Some(361.0);
    let second = policy.on_snapshot(&snapshot);
    assert!(
        second
            .iter()
            .all(|command| !matches!(command, TransportCommand::RequestKeyframe { .. })),
        "second snapshot should be suppressed by escalation cooldown budget"
    );
}

#[test]
fn display_supply_critical_does_not_trigger_reconnect_candidate() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        stats.host_no_pending_pressure_level = Some("critical".to_string());
        stats.host_no_pending_streak = 240;
        stats.latest_video_host_present_time_ms = Some(now_ms - 1_240.0);
        stats.latest_video_decode_ok_time_ms = Some(now_ms - 720.0);
        stats.video_renderer_stalled = Some(true);
        stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
    }
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
    let snapshot = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 365.0);
    let commands = policy.on_snapshot(&snapshot);

    assert!(commands
        .iter()
        .any(|command| matches!(command, TransportCommand::RequestKeyframe { .. })));
    assert!(commands
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })));
}

#[test]
fn owner_contract_is_persisted_to_runtime_stats() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        stats.host_no_pending_pressure_level = Some("critical".to_string());
        stats.host_no_pending_streak = 240;
        stats.latest_video_host_present_time_ms = Some(now_ms - 1000.0);
        stats.latest_video_decode_ok_time_ms = Some(now_ms - 540.0);
        stats.video_renderer_stalled = Some(true);
    }
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    let snapshot = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 450.0);
    let _ = policy.on_snapshot(&snapshot);
    let stats = runtime_stats.lock().expect("runtime stats lock");
    assert_eq!(stats.video_owner_state.as_deref(), Some("supply-starved"));
    assert_eq!(stats.video_owner_source.as_deref(), Some("supply"));
    assert_eq!(
        stats.video_owner_reason.as_deref(),
        Some("displaySupplyCritical")
    );
    assert_eq!(stats.video_owner_observed_at_ms, Some(450.0));
    assert_eq!(
        stats.baseline_remote_profile.as_deref(),
        Some("homeLanGaming")
    );
    assert_eq!(
        stats.recovery_policy_profile.as_deref(),
        Some("homeLanGaming")
    );
    assert_eq!(
        stats.dynamic_remote_subprofile.as_deref(),
        Some("displayConstrained")
    );
    assert_eq!(
        stats.effective_remote_profile_label.as_deref(),
        Some("homeLanGaming+displayConstrained")
    );
}

#[test]
fn recovery_intent_is_suppressed_within_same_epoch_via_coordinator_chain() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        stats.transport_recovery_epoch = 2;
        stats.transport_recovery_epoch_at_last_escalation = 2;
        stats.host_no_pending_pressure_level = Some("critical".to_string());
        stats.host_no_pending_streak = 220;
        stats.latest_video_host_present_time_ms = Some(now_ms - 1200.0);
        stats.latest_video_decode_ok_time_ms = Some(now_ms - 600.0);
        stats.video_renderer_stalled = Some(true);
    }
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
    let mut first = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 500.0);
    first.version = 1;
    let first_cmds = policy.on_snapshot(&first);
    assert!(first_cmds
        .iter()
        .any(|command| matches!(command, TransportCommand::RequestKeyframe { .. })));

    let mut second = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 501.0);
    second.version = 2;
    let second_cmds = policy.on_snapshot(&second);
    assert!(second_cmds
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestKeyframe { .. })));
}

#[test]
fn suppressed_owner_intent_is_not_forwarded_back_into_recovery_analysis() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        stats.transport_recovery_epoch = 2;
        stats.host_no_pending_pressure_level = Some("critical".to_string());
        stats.host_no_pending_streak = 220;
        stats.latest_video_host_present_time_ms = Some(now_ms - 1_200.0);
        stats.latest_video_decode_ok_time_ms = Some(now_ms - 600.0);
        stats.video_renderer_stalled = Some(true);
    }
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    let first = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 500.0);
    let first_cmds = policy.on_snapshot(&first);
    assert!(first_cmds
        .iter()
        .any(|command| matches!(command, TransportCommand::RequestKeyframe { .. })));

    let second = build_snapshot(
        ConnectionLifecycleStateFact::Connected,
        "adapterIdleTimeout",
        501.0,
    );
    let second_cmds = policy.on_snapshot(&second);
    assert!(second_cmds
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestKeyframe { .. })));
    let stats = runtime_stats.lock().expect("runtime stats lock");
    let ledger = stats
        .latest_recovery_decision_ledger
        .as_ref()
        .expect("recovery decision ledger");
    assert_eq!(ledger.input_signal, "none");
    assert_eq!(ledger.gate_result, "no-signal");
    assert_eq!(ledger.action_selected, "none");
}

#[test]
fn new_recovery_epoch_does_not_bypass_existing_recovery_suppression_chain() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        stats.transport_recovery_epoch = 3;
        stats.transport_recovery_epoch_at_last_escalation = 3;
        stats.host_no_pending_pressure_level = Some("critical".to_string());
        stats.host_no_pending_streak = 240;
        stats.latest_video_host_present_time_ms = Some(now_ms - 1300.0);
        stats.latest_video_decode_ok_time_ms = Some(now_ms - 700.0);
        stats.video_renderer_stalled = Some(true);
    }
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    let mut first = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 700.0);
    first.version = 1;
    let first_cmds = policy.on_snapshot(&first);
    assert!(first_cmds
        .iter()
        .any(|command| matches!(command, TransportCommand::RequestKeyframe { .. })));

    let mut second = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 701.0);
    second.version = 2;
    let second_cmds = policy.on_snapshot(&second);
    assert!(second_cmds
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestKeyframe { .. })));

    if let Ok(mut stats) = runtime_stats.lock() {
        stats.transport_recovery_epoch = 4;
        stats.transport_recovery_epoch_at_last_escalation = 3;
    }
    let mut third = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 703.0);
    third.version = 3;
    let third_cmds = policy.on_snapshot(&third);
    assert!(third_cmds
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestKeyframe { .. })));
}

#[test]
fn owner_contract_drives_display_supply_recovery_reason() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        stats.host_no_pending_pressure_level = Some("critical".to_string());
        stats.host_no_pending_streak = 260;
        stats.latest_video_host_present_time_ms = Some(now_ms - 1200.0);
        stats.latest_video_decode_ok_time_ms = Some(now_ms - 600.0);
        stats.video_renderer_stalled = Some(true);
    }
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
    let snapshot = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 400.0);
    let commands = policy.on_snapshot(&snapshot);
    let reason = commands.into_iter().find_map(|cmd| match cmd {
        TransportCommand::RequestKeyframe { reason, .. } => Some(reason),
        _ => None,
    });
    assert_eq!(reason.as_deref(), Some("displaySupplyCritical"));
}

#[test]
fn soft_display_supply_critical_is_absorbed_before_recovery_command() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());

    if let Ok(mut stats) = runtime_stats.lock() {
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud);
        stats.transport_recovery_epoch = 1;
        stats.video_anchor_clean_epoch = Some(1);
        stats.video_anchor_clean_observed_at_ms = Some(390.0);
        stats.video_anchor_clean_source_event = Some("chain-clean-keyframe-submitted".to_string());
        stats.host_no_pending_pressure_level = Some("normal".to_string());
        stats.host_no_pending_streak = 0;
        stats.latest_video_host_present_time_ms = Some(now_ms - 12.0);
        stats.latest_video_decode_ok_time_ms = Some(now_ms - 8.0);
        stats.video_renderer_stalled = Some(false);
        stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
            state: "remoteTrackAttached".to_string(),
            video_width: Some(1920),
            video_height: Some(1080),
            mime_type: None,
            transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
            video_bytes_total: 120_000,
            video_packet_count_total: 900,
            audio_bytes_total: 32_000,
            observed_at_ms: 390.0,
        });
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 1,
            source_event: "frame-observed".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "healthy".to_string(),
                reason: None,
                observed_at_ms: 390.0,
            },
            observed_at_ms: 390.0,
        });
    }

    let first = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 400.0);
    let _ = policy.on_snapshot(&first);
    let second = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 420.0);
    let second_cmds = policy.on_snapshot(&second);
    assert!(second_cmds
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestKeyframe { .. })));

    if let Ok(mut stats) = runtime_stats.lock() {
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        stats.host_no_pending_pressure_level = Some("critical".to_string());
        stats.host_no_pending_streak = 132;
        stats.latest_video_host_present_time_ms = Some(now_ms - 980.0);
        stats.latest_video_decode_ok_time_ms = Some(now_ms - 360.0);
    }

    let third = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 500.0);
    let third_cmds = policy.on_snapshot(&third);
    assert!(third_cmds
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestKeyframe { .. })));
    {
        let stats = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(stats.video_owner_state.as_deref(), Some("degraded-serving"));
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(ledger.state_after, "stable");
    }

    if let Ok(mut stats) = runtime_stats.lock() {
        stats.video_renderer_stalled = Some(true);
    }

    let fourth = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 760.0);
    let fourth_cmds = policy.on_snapshot(&fourth);
    assert!(fourth_cmds.iter().any(|command| {
        matches!(command, TransportCommand::RequestKeyframe { reason, .. } if reason == "displaySupplyCritical")
    }));
}

#[test]
fn display_supply_critical_does_not_stage_reconnect_candidate() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        stats.host_no_pending_pressure_level = Some("critical".to_string());
        stats.host_no_pending_streak = 320;
        stats.latest_video_host_present_time_ms = Some(now_ms - 1_500.0);
        stats.latest_video_decode_ok_time_ms = Some(now_ms - 900.0);
        stats.video_renderer_stalled = Some(true);
    }
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
    let first = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 400.0);
    let first_commands = policy.on_snapshot(&first);
    assert!(first_commands.iter().any(|command| {
        matches!(command, TransportCommand::RequestKeyframe { reason, .. } if reason == "displaySupplyCritical")
    }));
    assert!(first_commands
        .iter()
        .all(|command| { !matches!(command, TransportCommand::RequestReconnectCandidate { .. }) }));

    let mut second = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 820.0);
    second.version = 2;
    second.recovery.last_observed_at_ms = Some(820.0);
    let second_commands = policy.on_snapshot(&second);
    assert!(second_commands
        .iter()
        .all(|command| { !matches!(command, TransportCommand::RequestReconnectCandidate { .. }) }));
}

#[test]
fn owner_does_not_enter_stable_serving_when_audio_only_and_no_pending_critical() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.host_no_pending_pressure_level = Some("critical".to_string());
        stats.host_no_pending_streak = 980;
        stats.latest_video_host_present_time_ms = None;
        stats.latest_video_decode_ok_time_ms = None;
        stats.video_renderer_stalled = Some(false);
        stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
            state: "audioOnly".to_string(),
            video_width: None,
            video_height: None,
            mime_type: None,
            transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
            video_bytes_total: 0,
            video_packet_count_total: 0,
            audio_bytes_total: 128,
            observed_at_ms: 700.0,
        });
    }
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    let snapshot = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 700.0);
    let _ = policy.on_snapshot(&snapshot);
    let stats = runtime_stats.lock().expect("runtime stats lock");
    assert_eq!(stats.video_owner_state.as_deref(), Some("supply-starved"));
}

#[test]
fn owner_keeps_rebuilding_supply_when_timeline_keeps_awaiting_recovery_keyframe() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        stats.host_no_pending_pressure_level = Some("normal".to_string());
        stats.host_no_pending_streak = 24;
        stats.latest_video_host_present_time_ms = Some(now_ms - 220.0);
        stats.latest_video_decode_ok_time_ms = Some(now_ms - 180.0);
        stats.video_renderer_stalled = Some(false);
        stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
            state: "remoteTrackAttached".to_string(),
            video_width: Some(1920),
            video_height: Some(1080),
            mime_type: None,
            transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
            video_bytes_total: 120_000,
            video_packet_count_total: 900,
            audio_bytes_total: 32_000,
            observed_at_ms: 810.0,
        });
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 1,
            source_event: "frame-await-recovery-keyframe".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "healthy".to_string(),
                reason: None,
                observed_at_ms: 810.0,
            },
            observed_at_ms: 810.0,
        });
    }
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    let first = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 810.0);
    let _ = policy.on_snapshot(&first);

    let second = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 820.0);
    let _ = policy.on_snapshot(&second);
    let stats = runtime_stats.lock().expect("runtime stats lock");
    assert_eq!(
        stats.video_owner_state.as_deref(),
        Some("rebuilding-supply")
    );
    assert_eq!(
        stats.video_owner_reason.as_deref(),
        Some("transportAwaitRecoveryKeyframe")
    );
}

#[test]
fn owner_anchor_reason_is_derived_from_timeline_chain_reason_not_recovery_diagnosis() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        stats.host_no_pending_pressure_level = Some("normal".to_string());
        stats.host_no_pending_streak = 8;
        stats.latest_video_host_present_time_ms = Some(now_ms - 240.0);
        stats.latest_video_decode_ok_time_ms = Some(now_ms - 180.0);
        stats.video_renderer_stalled = Some(false);
        stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
            state: "remoteTrackAttached".to_string(),
            video_width: Some(1920),
            video_height: Some(1080),
            mime_type: None,
            transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
            video_bytes_total: 120_000,
            video_packet_count_total: 900,
            audio_bytes_total: 32_000,
            observed_at_ms: 910.0,
        });
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 11,
            source_event: "frame-await-recovery-keyframe".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "recovering".to_string(),
                reason: Some("awaitRecoveryKeyframe".to_string()),
                observed_at_ms: 910.0,
            },
            observed_at_ms: 910.0,
        });
    }
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    let snapshot = build_snapshot(
        ConnectionLifecycleStateFact::Connected,
        "decoderBackendFailure",
        920.0,
    );
    let _ = policy.on_snapshot(&snapshot);
    let stats = runtime_stats.lock().expect("runtime stats lock");
    assert_eq!(
        stats.video_owner_state.as_deref(),
        Some("rebuilding-supply")
    );
    assert_eq!(
        stats.video_owner_reason.as_deref(),
        Some("transportAwaitRecoveryKeyframe")
    );
}

#[test]
fn owner_exits_recovering_after_recovery_completion_evidence() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());

    if let Ok(mut stats) = runtime_stats.lock() {
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        stats.host_no_pending_pressure_level = Some("normal".to_string());
        stats.host_no_pending_streak = 30;
        stats.latest_video_host_present_time_ms = Some(now_ms - 240.0);
        stats.latest_video_decode_ok_time_ms = Some(now_ms - 170.0);
        stats.video_renderer_stalled = Some(false);
        stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
            state: "remoteTrackAttached".to_string(),
            video_width: Some(1920),
            video_height: Some(1080),
            mime_type: None,
            transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
            video_bytes_total: 140_000,
            video_packet_count_total: 1000,
            audio_bytes_total: 36_000,
            observed_at_ms: 900.0,
        });
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 1,
            source_event: "frame-complete-candidate".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "healthy".to_string(),
                reason: None,
                observed_at_ms: 900.0,
            },
            observed_at_ms: 900.0,
        });
    }
    let recovering = build_snapshot(
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryKeyframe",
        900.0,
    );
    let _ = policy.on_snapshot(&recovering);

    if let Ok(mut stats) = runtime_stats.lock() {
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        stats.host_no_pending_streak = 0;
        stats.latest_video_host_present_time_ms = Some(now_ms - 18.0);
        stats.latest_video_decode_ok_time_ms = Some(now_ms - 12.0);
        stats.video_anchor_clean_epoch = Some(stats.transport_recovery_epoch);
        stats.video_anchor_clean_observed_at_ms = Some(now_ms - 15.0);
        stats.video_anchor_clean_source_event = Some("chain-clean-keyframe-submitted".to_string());
        if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
            timeline.source_event = "frame-observed".to_string();
            timeline.chain.state = "healthy".to_string();
        }
    }
    let healed = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 930.0);
    let _ = policy.on_snapshot(&healed);
    let stats = runtime_stats.lock().expect("runtime stats lock");
    assert_eq!(stats.video_owner_state.as_deref(), Some("stable-serving"));
    assert_eq!(stats.video_owner_reason.as_deref(), Some("steady"));
}

#[test]
fn frame_observed_without_clean_anchor_fact_cannot_exit_recovering() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());

    if let Ok(mut stats) = runtime_stats.lock() {
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        stats.host_no_pending_pressure_level = Some("normal".to_string());
        stats.host_no_pending_streak = 30;
        stats.latest_video_host_present_time_ms = Some(now_ms - 240.0);
        stats.latest_video_decode_ok_time_ms = Some(now_ms - 170.0);
        stats.video_renderer_stalled = Some(false);
        stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
            state: "remoteTrackAttached".to_string(),
            video_width: Some(1920),
            video_height: Some(1080),
            mime_type: None,
            transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
            video_bytes_total: 140_000,
            video_packet_count_total: 1000,
            audio_bytes_total: 36_000,
            observed_at_ms: 900.0,
        });
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 1,
            source_event: "frame-await-recovery-keyframe".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "recovering".to_string(),
                reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                observed_at_ms: 900.0,
            },
            observed_at_ms: 900.0,
        });
    }
    let recovering = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 900.0);
    let _ = policy.on_snapshot(&recovering);

    if let Ok(mut stats) = runtime_stats.lock() {
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        stats.host_no_pending_streak = 0;
        stats.latest_video_host_present_time_ms = Some(now_ms - 18.0);
        stats.latest_video_decode_ok_time_ms = Some(now_ms - 12.0);
        stats.video_anchor_clean_epoch = None;
        stats.video_anchor_clean_observed_at_ms = None;
        stats.video_anchor_clean_source_event = None;
        if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
            timeline.source_event = "frame-observed".to_string();
            timeline.chain.state = "healthy".to_string();
        }
    }
    let healed = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 930.0);
    let _ = policy.on_snapshot(&healed);
    let stats = runtime_stats.lock().expect("runtime stats lock");
    assert_eq!(
        stats.video_owner_state.as_deref(),
        Some("rebuilding-supply")
    );
}

#[test]
fn clean_anchor_healthy_chain_can_close_recovery_on_transient_present_feedback_gap() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());

    if let Ok(mut stats) = runtime_stats.lock() {
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        stats.host_no_pending_pressure_level = Some("normal".to_string());
        stats.host_no_pending_streak = 24;
        stats.latest_video_host_present_time_ms = Some(now_ms - 260.0);
        stats.latest_video_decode_ok_time_ms = Some(now_ms - 180.0);
        stats.video_renderer_stalled = Some(false);
        stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
            state: "remoteTrackAttached".to_string(),
            video_width: Some(1920),
            video_height: Some(1080),
            mime_type: None,
            transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
            video_bytes_total: 155_000,
            video_packet_count_total: 1200,
            audio_bytes_total: 42_000,
            observed_at_ms: 900.0,
        });
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 1,
            source_event: "frame-await-recovery-keyframe".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "recovering".to_string(),
                reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                observed_at_ms: 900.0,
            },
            observed_at_ms: 900.0,
        });
    }
    let recovering = build_snapshot(
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryKeyframe",
        900.0,
    );
    let _ = policy.on_snapshot(&recovering);

    if let Ok(mut stats) = runtime_stats.lock() {
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        stats.host_no_pending_streak = 0;
        stats.latest_video_host_present_time_ms = None;
        stats.latest_video_decode_ok_time_ms = Some(now_ms - 9.0);
        stats.video_anchor_clean_epoch = Some(stats.transport_recovery_epoch);
        stats.video_anchor_clean_observed_at_ms = Some(now_ms - 10.0);
        stats.video_anchor_clean_source_event = Some("chain-clean-keyframe-submitted".to_string());
        if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
            timeline.source_event = "frame-observed".to_string();
            timeline.chain.state = "healthy".to_string();
            timeline.chain.reason = None;
        }
    }
    let waiting_present = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 930.0);
    let _ = policy.on_snapshot(&waiting_present);
    {
        let stats = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(stats.video_owner_state.as_deref(), Some("stable-serving"));
        assert_eq!(stats.video_owner_reason.as_deref(), Some("steady"));
    }
}

#[test]
fn clean_anchor_healthy_chain_stays_recovering_when_present_feedback_gap_is_not_settled() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());

    if let Ok(mut stats) = runtime_stats.lock() {
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        stats.host_no_pending_pressure_level = Some("normal".to_string());
        stats.host_no_pending_streak = 24;
        stats.latest_video_host_present_time_ms = Some(now_ms - 260.0);
        stats.latest_video_decode_ok_time_ms = Some(now_ms - 180.0);
        stats.video_renderer_stalled = Some(false);
        stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
            state: "remoteTrackAttached".to_string(),
            video_width: Some(1920),
            video_height: Some(1080),
            mime_type: None,
            transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
            video_bytes_total: 155_000,
            video_packet_count_total: 1200,
            audio_bytes_total: 42_000,
            observed_at_ms: 900.0,
        });
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 1,
            source_event: "frame-await-recovery-keyframe".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "recovering".to_string(),
                reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                observed_at_ms: 900.0,
            },
            observed_at_ms: 900.0,
        });
    }
    let recovering = build_snapshot(
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryKeyframe",
        900.0,
    );
    let _ = policy.on_snapshot(&recovering);

    if let Ok(mut stats) = runtime_stats.lock() {
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        stats.host_no_pending_pressure_level = Some("high".to_string());
        stats.host_no_pending_streak = 6;
        stats.latest_video_host_present_time_ms = None;
        stats.latest_video_decode_ok_time_ms = Some(now_ms - 9.0);
        stats.video_anchor_clean_epoch = Some(stats.transport_recovery_epoch);
        stats.video_anchor_clean_observed_at_ms = Some(now_ms - 10.0);
        stats.video_anchor_clean_source_event = Some("chain-clean-keyframe-submitted".to_string());
        if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
            timeline.source_event = "frame-observed".to_string();
            timeline.chain.state = "healthy".to_string();
            timeline.chain.reason = None;
        }
    }
    let waiting_present = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 930.0);
    let _ = policy.on_snapshot(&waiting_present);
    {
        let stats = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(
            stats.video_owner_state.as_deref(),
            Some("rebuilding-supply")
        );
    }

    if let Ok(mut stats) = runtime_stats.lock() {
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        stats.latest_video_host_present_time_ms = Some(now_ms - 12.0);
        stats.latest_video_decode_ok_time_ms = Some(now_ms - 8.0);
    }
    let healed = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 960.0);
    let _ = policy.on_snapshot(&healed);
    let stats = runtime_stats.lock().expect("runtime stats lock");
    assert_eq!(stats.video_owner_state.as_deref(), Some("stable-serving"));
    assert_eq!(stats.video_owner_reason.as_deref(), Some("steady"));
}

#[test]
fn frame_complete_candidate_without_clean_anchor_fact_can_exit_recovering() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());

    if let Ok(mut stats) = runtime_stats.lock() {
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        stats.host_no_pending_pressure_level = Some("normal".to_string());
        stats.host_no_pending_streak = 30;
        stats.latest_video_host_present_time_ms = Some(now_ms - 240.0);
        stats.latest_video_decode_ok_time_ms = Some(now_ms - 170.0);
        stats.video_renderer_stalled = Some(false);
        stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
            state: "remoteTrackAttached".to_string(),
            video_width: Some(1920),
            video_height: Some(1080),
            mime_type: None,
            transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
            video_bytes_total: 140_000,
            video_packet_count_total: 1000,
            audio_bytes_total: 36_000,
            observed_at_ms: 900.0,
        });
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 1,
            source_event: "frame-await-recovery-keyframe".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "recovering".to_string(),
                reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                observed_at_ms: 900.0,
            },
            observed_at_ms: 900.0,
        });
    }
    let recovering = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 900.0);
    let _ = policy.on_snapshot(&recovering);

    if let Ok(mut stats) = runtime_stats.lock() {
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        stats.host_no_pending_streak = 0;
        stats.latest_video_host_present_time_ms = Some(now_ms - 18.0);
        stats.latest_video_decode_ok_time_ms = Some(now_ms - 12.0);
        stats.video_anchor_clean_epoch = None;
        stats.video_anchor_clean_observed_at_ms = None;
        stats.video_anchor_clean_source_event = None;
        if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
            timeline.source_event = "frame-complete-candidate".to_string();
            timeline.chain.state = "healthy".to_string();
        }
    }
    let healed = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 930.0);
    let _ = policy.on_snapshot(&healed);
    let stats = runtime_stats.lock().expect("runtime stats lock");
    assert_eq!(stats.video_owner_state.as_deref(), Some("stable-serving"));
    assert_eq!(stats.video_owner_reason.as_deref(), Some("steady"));
}

#[test]
fn lifecycle_recovering_clears_stale_clean_anchor_fact() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.video_anchor_clean_epoch = Some(5);
        stats.video_anchor_clean_observed_at_ms = Some(1000.0);
        stats.video_anchor_clean_source_event = Some("chain-clean-keyframe-submitted".to_string());
    }
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Recovering;
    let snapshot = TransportSnapshot::new(
        1,
        1100.0,
        connection,
        MediaProjection {
            frame_count: 1,
            ..MediaProjection::default()
        },
        RecoveryProjection {
            latest_diagnosis_label: Some("rtcConnectionRecovering".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(1100.0),
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let _ = policy.on_snapshot(&snapshot);
    let stats = runtime_stats.lock().expect("runtime stats lock");
    assert_eq!(stats.video_anchor_clean_epoch, None);
    assert_eq!(stats.video_anchor_clean_observed_at_ms, None);
    assert_eq!(stats.video_anchor_clean_source_event, None);
}

#[test]
fn lifecycle_recovering_preserves_current_clean_anchor_fact() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.transport_recovery_epoch = 5;
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 5,
            source_event: "frame-await-recovery-keyframe".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "recovering".to_string(),
                reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                observed_at_ms: 1095.0,
            },
            observed_at_ms: 1095.0,
        });
    }
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Recovering;
    let first_snapshot = TransportSnapshot::new(
        1,
        1100.0,
        connection,
        MediaProjection {
            frame_count: 1,
            ..MediaProjection::default()
        },
        RecoveryProjection {
            latest_diagnosis_label: Some("rtcConnectionRecovering".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(1100.0),
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let _ = policy.on_snapshot(&first_snapshot);
    let current_epoch = {
        let stats = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(
            stats.video_owner_state.as_deref(),
            Some("rebuilding-supply")
        );
        stats.transport_recovery_epoch
    };
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.video_anchor_clean_epoch = Some(current_epoch);
        stats.video_anchor_clean_observed_at_ms = Some(1200.0);
        stats.video_anchor_clean_source_event = Some("chain-clean-keyframe-submitted".to_string());
        stats.host_no_pending_pressure_level = Some("normal".to_string());
        stats.host_no_pending_streak = 0;
        stats.latest_video_host_present_time_ms = Some(1290.0);
        stats.latest_video_decode_ok_time_ms = Some(1292.0);
        stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
            state: "remoteTrackAttached".to_string(),
            video_width: Some(1920),
            video_height: Some(1080),
            mime_type: Some("video/H264".to_string()),
            transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
            video_bytes_total: 42_000,
            video_packet_count_total: 420,
            audio_bytes_total: 0,
            observed_at_ms: 1294.0,
        });
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 6,
            source_event: "frame-observed".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "healthy".to_string(),
                reason: None,
                observed_at_ms: 1295.0,
            },
            observed_at_ms: 1295.0,
        });
    }
    let second_snapshot = TransportSnapshot::new(
        1,
        1300.0,
        ConnectionProjection {
            lifecycle_state: ConnectionLifecycleStateFact::Recovering,
            ..ConnectionProjection::default()
        },
        MediaProjection {
            frame_count: 1,
            ..MediaProjection::default()
        },
        RecoveryProjection {
            latest_diagnosis_label: Some("rtcConnectionRecovering".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(1300.0),
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let _ = policy.on_snapshot(&second_snapshot);
    let stats = runtime_stats.lock().expect("runtime stats lock");
    assert_eq!(stats.transport_recovery_epoch, current_epoch);
    assert_eq!(stats.video_anchor_clean_epoch, Some(current_epoch));
    assert_eq!(stats.video_anchor_clean_observed_at_ms, Some(1200.0));
    assert_eq!(
        stats.video_anchor_clean_source_event.as_deref(),
        Some("chain-clean-keyframe-submitted")
    );
}

#[test]
fn display_supply_thresholds_differ_between_home_and_cloud_profiles() {
    let now_ms = crate::transport::rtc::stats::now_ms_f64();
    let base = XbxEngineMediaRuntimeStats {
        host_no_pending_pressure_level: Some("critical".to_string()),
        host_no_pending_streak: 100,
        latest_video_host_present_time_ms: Some(now_ms - 630.0),
        latest_video_decode_ok_time_ms: Some(now_ms - 340.0),
        video_renderer_stalled: Some(false),
        ..XbxEngineMediaRuntimeStats::default()
    };
    let cloud_stats = XbxEngineMediaRuntimeStats {
        session_target_type: Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud),
        ..base.clone()
    };
    let home_stats = XbxEngineMediaRuntimeStats {
        session_target_type: Some(xbxengine_protocol::XbxEngineTargetTypeDto::Home),
        transport_path: Some("direct".to_string()),
        ..base
    };

    assert_eq!(
        classify_supply_state_with_profile(&cloud_stats),
        crate::transport::rtc::policy::display_supply::DisplaySupplyState::Degraded
    );
    assert_eq!(
        classify_supply_state_with_profile(&home_stats),
        crate::transport::rtc::policy::display_supply::DisplaySupplyState::Degraded
    );
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
fn cloud_builder_configured_warmup_blocks_bwe_update() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    if let Ok(mut config) = runtime_config.lock() {
        config.webrtc.bwe_mode = "twcc-gcc".to_string();
        config.webrtc.remb_floor_kbps = 8_000;
        config.webrtc.remb_ceiling_kbps = 150_000;
    }
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud);
        stats.session_phase = Some("recovering".to_string());
        stats.latest_rtc_builder_observation = Some(crate::XbxEngineRtcBuilderObservation {
            observation_id: 1,
            controlled_twcc_registry: true,
            feedback_interval_ms: 100.0,
            registered_header_extensions: vec!["video:transport-cc".to_string()],
            registered_rtcp_feedback: vec!["video:transport-cc".to_string()],
            observed_at_ms: 10.0,
        });
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

    let commands = policy.on_snapshot(&snapshot);
    assert!(!commands
        .iter()
        .any(|command| matches!(command, TransportCommand::SetTargetRembKbps { .. })));
}

#[test]
fn cloud_valid_local_feedback_restores_bwe_after_warmup() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    if let Ok(mut config) = runtime_config.lock() {
        config.webrtc.bwe_mode = "twcc-gcc".to_string();
        config.webrtc.remb_floor_kbps = 8_000;
        config.webrtc.remb_ceiling_kbps = 150_000;
    }
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud);
        stats.session_phase = Some("steady".to_string());
        stats.latest_rtc_builder_observation = Some(crate::XbxEngineRtcBuilderObservation {
            observation_id: 1,
            controlled_twcc_registry: true,
            feedback_interval_ms: 100.0,
            registered_header_extensions: vec!["video:transport-cc".to_string()],
            registered_rtcp_feedback: vec!["video:transport-cc".to_string()],
            observed_at_ms: 10.0,
        });
        stats.latest_video_twcc_observation = Some(XbxEngineVideoTwccObservation {
            observation_id: 2,
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

    let commands = policy.on_snapshot(&snapshot);
    assert!(commands
        .iter()
        .any(|command| matches!(command, TransportCommand::SetTargetRembKbps { .. })));
}

#[test]
fn cloud_builder_configured_warmup_holds_media_reconnect_candidate() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud);
        stats.latest_rtc_builder_observation = Some(crate::XbxEngineRtcBuilderObservation {
            observation_id: 1,
            controlled_twcc_registry: true,
            feedback_interval_ms: 100.0,
            registered_header_extensions: vec!["video:transport-cc".to_string()],
            registered_rtcp_feedback: vec!["video:transport-cc".to_string()],
            observed_at_ms: 1_000.0,
        });
        stats.video_renderer_stalled = Some(true);
        stats.latest_video_host_present_time_ms = Some(0.0);
        stats.latest_video_decoder_reset_time_ms = Some(2_000.0);
    }
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    let first = build_snapshot(
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryKeyframe",
        1_000.0,
    );
    let _ = policy.on_snapshot(&first);

    let second = build_snapshot(
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryKeyframe",
        8_000.0,
    );
    let commands = policy.on_snapshot(&second);
    assert!(commands
        .iter()
        .all(|command| { !matches!(command, TransportCommand::RequestReconnectCandidate { .. }) }));
    let stats = runtime_stats.lock().expect("runtime stats lock");
    let ledger = stats
        .latest_recovery_decision_ledger
        .as_ref()
        .expect("recovery decision ledger");
    assert_recovery_family_hold_semantics(
        ledger.gate_result.as_str(),
        ledger.action_selected.as_str(),
    );
}

#[test]
fn cloud_builder_configured_warmup_does_not_block_lifecycle_reconnect() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud);
        stats.latest_rtc_builder_observation = Some(crate::XbxEngineRtcBuilderObservation {
            observation_id: 1,
            controlled_twcc_registry: true,
            feedback_interval_ms: 100.0,
            registered_header_extensions: vec!["video:transport-cc".to_string()],
            registered_rtcp_feedback: vec!["video:transport-cc".to_string()],
            observed_at_ms: 100.0,
        });
    }
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Connecting;
    let recovery = RecoveryProjection {
        latest_diagnosis_label: Some("none".to_string()),
        pending_action: false,
        successful_action_count: 0,
        failed_action_count: 0,
        last_observed_at_ms: Some(100.0),
    };
    let first = TransportSnapshot::new(
        1,
        100.0,
        connection.clone(),
        MediaProjection::default(),
        recovery.clone(),
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let _ = policy.on_snapshot(&first);
    let second = TransportSnapshot::new(
        2,
        35_600.0,
        connection,
        MediaProjection::default(),
        RecoveryProjection {
            last_observed_at_ms: Some(35_600.0),
            ..recovery
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let commands = policy.on_snapshot(&second);
    assert!(commands
        .iter()
        .any(|command| { matches!(command, TransportCommand::RequestReconnectCandidate { .. }) }));
}

#[test]
fn cloud_builder_configured_uses_more_relaxed_lifecycle_reconnect_interval_than_missing_feedback() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud);
        stats.latest_rtc_builder_observation = Some(crate::XbxEngineRtcBuilderObservation {
            observation_id: 1,
            controlled_twcc_registry: true,
            feedback_interval_ms: 100.0,
            registered_header_extensions: vec!["video:transport-cc".to_string()],
            registered_rtcp_feedback: vec!["video:transport-cc".to_string()],
            observed_at_ms: 100.0,
        });
    }
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Recovering;
    let media = MediaProjection {
        frame_count: 1,
        ..MediaProjection::default()
    };
    let recovery = RecoveryProjection {
        latest_diagnosis_label: Some("rtcPeerConnectionFailed".to_string()),
        pending_action: false,
        successful_action_count: 0,
        failed_action_count: 0,
        last_observed_at_ms: Some(100.0),
    };

    let first = TransportSnapshot::new(
        1,
        100.0,
        connection.clone(),
        media.clone(),
        recovery.clone(),
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let first_commands = policy.on_snapshot(&first);
    assert!(first_commands
        .iter()
        .any(|command| { matches!(command, TransportCommand::RequestReconnectCandidate { .. }) }));

    if let Ok(mut stats) = runtime_stats.lock() {
        stats.latest_twcc_remote_stream_observation =
            Some(crate::XbxEngineTwccRemoteStreamObservation {
                observation_id: 2,
                ssrc: 42,
                mime_type: "video/H264".to_string(),
                twcc_ext_id: Some(7),
                header_extensions: vec!["transport-cc#7".to_string()],
                rtcp_feedback: vec!["transport-cc:".to_string()],
                observed_at_ms: 200.0,
            });
    }
    let second = TransportSnapshot::new(
        2,
        3_200.0,
        connection.clone(),
        media.clone(),
        RecoveryProjection {
            last_observed_at_ms: Some(3_200.0),
            ..recovery.clone()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let second_commands = policy.on_snapshot(&second);
    assert!(second_commands
        .iter()
        .all(|command| { !matches!(command, TransportCommand::RequestReconnectCandidate { .. }) }));

    let third = TransportSnapshot::new(
        3,
        3_800.0,
        connection,
        media,
        RecoveryProjection {
            last_observed_at_ms: Some(3_800.0),
            ..recovery
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let third_commands = policy.on_snapshot(&third);
    assert!(third_commands
        .iter()
        .any(|command| { matches!(command, TransportCommand::RequestReconnectCandidate { .. }) }));
}

#[test]
fn cloud_local_feedback_ready_restores_default_cloud_reconnect_interval() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud);
        stats.latest_rtc_builder_observation = Some(crate::XbxEngineRtcBuilderObservation {
            observation_id: 1,
            controlled_twcc_registry: true,
            feedback_interval_ms: 100.0,
            registered_header_extensions: vec!["video:transport-cc".to_string()],
            registered_rtcp_feedback: vec!["video:transport-cc".to_string()],
            observed_at_ms: 100.0,
        });
        stats.latest_video_twcc_observation = Some(XbxEngineVideoTwccObservation {
            observation_id: 2,
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
            observed_at_ms: 100.0,
        });
    }
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Recovering;
    let media = MediaProjection {
        frame_count: 1,
        ..MediaProjection::default()
    };
    let recovery = RecoveryProjection {
        latest_diagnosis_label: Some("rtcPeerConnectionFailed".to_string()),
        pending_action: false,
        successful_action_count: 0,
        failed_action_count: 0,
        last_observed_at_ms: Some(100.0),
    };

    let first = TransportSnapshot::new(
        1,
        100.0,
        connection.clone(),
        media.clone(),
        recovery.clone(),
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let first_commands = policy.on_snapshot(&first);
    assert!(first_commands
        .iter()
        .any(|command| { matches!(command, TransportCommand::RequestReconnectCandidate { .. }) }));

    let second = TransportSnapshot::new(
        2,
        2_000.0,
        connection.clone(),
        media.clone(),
        RecoveryProjection {
            last_observed_at_ms: Some(2_000.0),
            ..recovery.clone()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let second_commands = policy.on_snapshot(&second);
    assert!(second_commands
        .iter()
        .all(|command| { !matches!(command, TransportCommand::RequestReconnectCandidate { .. }) }));

    let third = TransportSnapshot::new(
        3,
        2_700.0,
        connection,
        media,
        RecoveryProjection {
            last_observed_at_ms: Some(2_700.0),
            ..recovery
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let third_commands = policy.on_snapshot(&third);
    assert!(third_commands
        .iter()
        .any(|command| { matches!(command, TransportCommand::RequestReconnectCandidate { .. }) }));
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
        MediaProjection {
            frame_count: 1,
            ..MediaProjection::default()
        },
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
fn runtime_reconnect_reason_domain_keeps_transport_await_local() {
    assert_eq!(
        resolve_runtime_reconnect_reason_domain(
            VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            RecoveryAction::RequestReconnectCandidate,
        ),
        crate::XbxEngineRecoveryReasonDomain::Local
    );
}

#[test]
fn runtime_reconnect_reason_domain_keeps_deadline_transport_connectivity() {
    assert_eq!(
        resolve_runtime_reconnect_reason_domain(
            VideoEscalationReason::TransportExpiredDeadline,
            RecoveryAction::RequestReconnectCandidate,
        ),
        crate::XbxEngineRecoveryReasonDomain::ConnectivityTransport
    );
}

#[test]
fn runtime_reconnect_reason_domain_keeps_severe_deadline_transport_connectivity() {
    assert_eq!(
        resolve_runtime_reconnect_reason_domain(
            VideoEscalationReason::TransportSevereDeadline,
            RecoveryAction::RequestReconnectCandidate,
        ),
        crate::XbxEngineRecoveryReasonDomain::ConnectivityTransport
    );
}

#[test]
fn stale_adapter_idle_timeout_does_not_replay_during_steady_progress() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.session_phase = Some("steady".to_string());
        stats.transport_recovery_epoch = 7;
        stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
        stats.host_no_pending_pressure_level = Some("normal".to_string());
        stats.host_no_pending_streak = 0;
        stats.latest_video_host_present_time_ms = Some(988.0);
        stats.latest_video_decode_ok_time_ms = Some(992.0);
        stats.video_decoder_stalled = Some(false);
        stats.video_renderer_stalled = Some(false);
        stats.video_anchor_clean_epoch = Some(7);
        stats.video_anchor_clean_observed_at_ms = Some(994.0);
        stats.video_anchor_clean_source_event = Some("chain-clean-keyframe-submitted".to_string());
        stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
            state: "remoteTrackAttached".to_string(),
            video_width: Some(1920),
            video_height: Some(1080),
            mime_type: Some("video/H264".to_string()),
            transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
            video_bytes_total: 128_000,
            video_packet_count_total: 1_600,
            audio_bytes_total: 36_000,
            observed_at_ms: 995.0,
        });
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 1,
            source_event: "frame-observed".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "healthy".to_string(),
                reason: None,
                observed_at_ms: 995.0,
            },
            observed_at_ms: 995.0,
        });
    }
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
    connection.control_channel_open = true;
    connection.latest_transport_path = Some("Direct".to_string());
    connection.latest_rtt_ms = Some(42.0);
    connection.last_observed_at_ms = Some(1_000.0);
    let healthy_snapshot = TransportSnapshot::new(
        1,
        1_000.0,
        connection.clone(),
        MediaProjection {
            frame_count: 31,
            ..MediaProjection::default()
        },
        RecoveryProjection {
            latest_diagnosis_label: Some("none".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(999.0),
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let _ = policy.on_snapshot(&healthy_snapshot);

    let snapshot = TransportSnapshot::new(
        2,
        1_008.0,
        connection,
        MediaProjection {
            frame_count: 32,
            ..MediaProjection::default()
        },
        RecoveryProjection {
            latest_diagnosis_label: Some("adapterIdleTimeout".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(900.0),
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );

    let commands = policy.on_snapshot(&snapshot);
    assert!(commands.is_empty(), "unexpected commands: {commands:?}");
    let stats = runtime_stats.lock().expect("runtime stats lock");
    let ledger = stats
        .latest_recovery_decision_ledger
        .as_ref()
        .expect("recovery decision ledger");
    assert_eq!(ledger.gate_result, "no-signal");
    assert_eq!(ledger.action_selected, "none");
}

#[test]
fn stale_transport_await_does_not_replay_during_steady_progress() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.session_phase = Some("steady".to_string());
        stats.transport_recovery_epoch = 8;
        stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
        stats.host_no_pending_pressure_level = Some("normal".to_string());
        stats.host_no_pending_streak = 0;
        stats.latest_video_host_present_time_ms = Some(990.0);
        stats.latest_video_decode_ok_time_ms = Some(996.0);
        stats.video_decoder_stalled = Some(false);
        stats.video_renderer_stalled = Some(false);
        stats.video_anchor_clean_epoch = Some(8);
        stats.video_anchor_clean_observed_at_ms = Some(998.0);
        stats.video_anchor_clean_source_event = Some("chain-clean-keyframe-submitted".to_string());
        stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
            state: "remoteTrackAttached".to_string(),
            video_width: Some(1920),
            video_height: Some(1080),
            mime_type: Some("video/H264".to_string()),
            transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
            video_bytes_total: 144_000,
            video_packet_count_total: 1_640,
            audio_bytes_total: 36_000,
            observed_at_ms: 998.0,
        });
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 2,
            source_event: "frame-observed".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "healthy".to_string(),
                reason: None,
                observed_at_ms: 998.0,
            },
            observed_at_ms: 998.0,
        });
    }
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
    connection.control_channel_open = true;
    connection.latest_transport_path = Some("Direct".to_string());
    connection.latest_rtt_ms = Some(40.0);
    connection.last_observed_at_ms = Some(1_000.0);

    let healthy_snapshot = TransportSnapshot::new(
        1,
        1_000.0,
        connection.clone(),
        MediaProjection {
            frame_count: 32,
            ..MediaProjection::default()
        },
        RecoveryProjection {
            latest_diagnosis_label: Some("none".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(999.0),
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let _ = policy.on_snapshot(&healthy_snapshot);

    let snapshot = TransportSnapshot::new(
        2,
        1_008.0,
        connection,
        MediaProjection {
            frame_count: 33,
            ..MediaProjection::default()
        },
        RecoveryProjection {
            latest_diagnosis_label: Some("transportAwaitRecoveryKeyframe".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(700.0),
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );

    let commands = policy.on_snapshot(&snapshot);
    assert!(commands.is_empty(), "unexpected commands: {commands:?}");
    let stats = runtime_stats.lock().expect("runtime stats lock");
    let ledger = stats
        .latest_recovery_decision_ledger
        .as_ref()
        .expect("recovery decision ledger");
    assert_eq!(ledger.gate_result, "no-signal");
    assert_eq!(ledger.action_selected, "none");
    assert_eq!(ledger.input_signal, "none");
}

#[test]
fn recovery_integration_transport_await_exits_after_completion_evidence() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));
    let first = harness.apply(
        900.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryKeyframe",
        12,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.transport_recovery_epoch = 1;
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 24;
            stats.latest_video_host_present_time_ms = Some(now_ms - 260.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 180.0);
            stats.video_renderer_stalled = Some(false);
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 155_000,
                video_packet_count_total: 1_200,
                audio_bytes_total: 42_000,
                observed_at_ms: 900.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-await-recovery-keyframe".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                        observed_at_ms: 900.0,
                    },
                    observed_at_ms: 900.0,
                });
        },
    );
    assert!(first.iter().any(
        |command| matches!(command, TransportCommand::RequestKeyframe { reason, .. } if reason == "transportAwaitRecoveryKeyframe")
    ));

    let second = harness.apply(
        930.0,
        ConnectionLifecycleStateFact::Connected,
        "none",
        13,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = None;
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 9.0);
            stats.video_anchor_clean_epoch = Some(stats.transport_recovery_epoch);
            stats.video_anchor_clean_observed_at_ms = Some(now_ms - 10.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-keyframe-submitted".to_string());
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.source_event = "frame-observed".to_string();
                timeline.chain.state = "healthy".to_string();
                timeline.chain.reason = None;
                timeline.observed_at_ms = 930.0;
            }
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total = 188_000;
                track.video_packet_count_total = 1_360;
                track.observed_at_ms = 930.0;
            }
        },
    );
    assert!(
        second.is_empty(),
        "unexpected commands after recovery completion: {second:?}"
    );
    harness.with_stats(|stats| {
        assert_eq!(stats.video_owner_state.as_deref(), Some("stable-serving"));
        assert_eq!(stats.video_owner_reason.as_deref(), Some("steady"));
    });
}

#[test]
fn recovery_integration_stale_transport_await_after_completion_evidence_stays_no_signal() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));
    let _ = harness.apply(
        900.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryKeyframe",
        12,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.transport_recovery_epoch = 1;
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 24;
            stats.latest_video_host_present_time_ms = Some(now_ms - 260.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 180.0);
            stats.video_renderer_stalled = Some(false);
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 155_000,
                video_packet_count_total: 1_200,
                audio_bytes_total: 42_000,
                observed_at_ms: 900.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-await-recovery-keyframe".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                        observed_at_ms: 900.0,
                    },
                    observed_at_ms: 900.0,
                });
        },
    );

    let commands = harness.apply_with_recovery_observed_at(
        1_260.0,
        1_000.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryKeyframe",
        20,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(now_ms - 10.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 12.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = Some(stats.transport_recovery_epoch);
            stats.video_anchor_clean_observed_at_ms = Some(now_ms - 14.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-keyframe-submitted".to_string());
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.source_event = "frame-observed".to_string();
                timeline.chain.state = "healthy".to_string();
                timeline.chain.reason = None;
                timeline.observed_at_ms = 1_000.0;
            }
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total = 188_000;
                track.video_packet_count_total = 1_360;
                track.observed_at_ms = 1_000.0;
            }
        },
    );

    assert!(
        commands.is_empty(),
        "unexpected commands after stale transportAwait replay: {commands:?}"
    );
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert!(
            matches!(ledger.gate_result.as_str(), "no-signal" | "pass"),
            "unexpected gate result after committed delta continuation became available: {}",
            ledger.gate_result
        );
        assert_eq!(ledger.action_selected, "none");
        assert_eq!(ledger.input_signal, "none");
        assert_eq!(stats.video_owner_state.as_deref(), Some("stable-serving"));
    });
}

#[test]
fn recovery_integration_recent_transport_await_exits_when_non_idr_has_committed_delta_continuation()
{
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

    let first = harness.apply(
        900.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryKeyframe",
        12,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.transport_recovery_epoch = 1;
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 24;
            stats.latest_video_host_present_time_ms = Some(now_ms - 260.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 180.0);
            stats.video_renderer_stalled = Some(false);
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 155_000,
                video_packet_count_total: 1_200,
                audio_bytes_total: 42_000,
                observed_at_ms: 900.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-await-recovery-keyframe".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                        observed_at_ms: 900.0,
                    },
                    observed_at_ms: 900.0,
                });
        },
    );
    assert!(first.iter().any(
        |command| matches!(command, TransportCommand::RequestKeyframe { reason, .. } if reason == "transportAwaitRecoveryKeyframe")
    ));

    let second = harness.apply(
        930.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryKeyframe",
        13,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(now_ms - 9.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 11.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = Some(stats.transport_recovery_epoch);
            stats.video_anchor_clean_observed_at_ms = Some(now_ms - 12.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-keyframe-submitted".to_string());
            stats.latest_h264_inspection_observation =
                Some(crate::XbxEngineH264InspectionObservation {
                    observation_id: 7,
                    frame_rtp_timestamp: Some(0x1020_3040),
                    nal_types: vec!["SliceLayerWithoutPartitioningNonIdr".to_string()],
                    has_inband_sps: false,
                    has_inband_pps: false,
                    committed_sps_present: true,
                    committed_pps_present: true,
                    slice_headers_valid: true,
                    delta_continuation_ready: true,
                    parameter_sets_changed: false,
                    config_changed: false,
                    is_idr: false,
                    bootstrap_ready: false,
                    bootstrap_reject_reason: Some("NonIdrVcl".to_string()),
                    admission_accepted: true,
                    observed_at_ms: now_ms - 1.0,
                });
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.source_event = "frame-complete-candidate".to_string();
                timeline.chain.state = "healthy".to_string();
                timeline.chain.reason = None;
                timeline.observed_at_ms = 930.0;
            }
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total = 188_000;
                track.video_packet_count_total = 1_360;
                track.observed_at_ms = 930.0;
            }
        },
    );

    assert!(
        second.is_empty(),
        "unexpected commands after committed delta continuation became available: {second:?}"
    );
    harness.with_stats(|stats| {
        assert_eq!(stats.video_owner_state.as_deref(), Some("stable-serving"));
        assert_eq!(stats.video_owner_reason.as_deref(), Some("steady"));
    });
}

#[test]
fn recovery_integration_same_unresolved_gap_transport_await_reuses_in_flight_family() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

    let first = harness.apply(
        10_000.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryKeyframe",
        240,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.session_phase = Some("recovering".to_string());
            stats.transport_recovery_epoch = 71;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 160;
            stats.latest_video_host_present_time_ms = Some(now_ms - 1_400.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 380.0);
            stats.latest_video_packet_arrival_time_ms = Some(now_ms - 2.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(true);
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 420_000,
                video_packet_count_total: 3_840,
                audio_bytes_total: 64_000,
                observed_at_ms: now_ms - 2.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 41,
                    source_event: "frame-await-recovery-keyframe".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                        observed_at_ms: now_ms - 2.0,
                    },
                    observed_at_ms: now_ms - 2.0,
                });
        },
    );
    assert!(first.iter().any(|command| {
        matches!(
            command,
            TransportCommand::RequestKeyframe { reason, .. }
                if reason == "transportAwaitRecoveryKeyframe"
        ) || matches!(
            command,
            TransportCommand::RequestDecoderReset { reason, .. }
                if reason == "transportAwaitRecoveryKeyframe"
        )
    }));

    for observed_at_ms in [10_650.0, 11_100.0] {
        let commands = harness.apply(
            observed_at_ms,
            ConnectionLifecycleStateFact::Connected,
            "transportAwaitRecoveryKeyframe",
            240,
            |stats| {
                let now_ms = crate::transport::rtc::stats::now_ms_f64();
                stats.session_phase = Some("recovering".to_string());
                stats.transport_recovery_epoch = 71;
                stats.host_no_pending_pressure_level = Some("critical".to_string());
                stats.host_no_pending_streak = 224;
                stats.latest_video_host_present_time_ms = Some(now_ms - 2_200.0);
                stats.latest_video_decode_ok_time_ms = Some(now_ms - 420.0);
                stats.latest_video_packet_arrival_time_ms = Some(now_ms - 2.0);
                stats.video_decoder_stalled = Some(false);
                stats.video_renderer_stalled = Some(true);
                stats.latest_video_escalation_observation =
                    Some(crate::XbxEngineVideoEscalationObservation {
                        observation_id: 41,
                        reason: "transportAwaitRecoveryKeyframe".to_string(),
                        action: "requestDecoderReset".to_string(),
                        recovery_stage: "rebuilding-supply".to_string(),
                        recovery_chain_value: "anchor".to_string(),
                        recovery_failure_cost: "high".to_string(),
                        recovery_window_source: "transport-await-window".to_string(),
                        observed_at_ms: now_ms - 90.0,
                    });
                stats.latest_keyframe_request_episode =
                    Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                        episode_id: 41,
                        request_reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                        request_kind: Some("control".to_string()),
                        status: "sent".to_string(),
                        requested_at_ms: now_ms - 700.0,
                        sent_at_ms: Some(now_ms - 690.0),
                        deadline_at_ms: Some(now_ms + 300.0),
                        first_keyframe_packet_at_ms: None,
                        first_keyframe_decoded_at_ms: None,
                        response_rtp_timestamp: None,
                        response_frame_seq: None,
                        response_verdict: Some("pending".to_string()),
                    });
                if let Some(track) = stats.latest_video_track_status.as_mut() {
                    track.video_bytes_total += 8_000;
                    track.video_packet_count_total += 60;
                    track.observed_at_ms = now_ms - 2.0;
                }
                if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                    timeline.observation_id = 41;
                    timeline.source_event = "frame-await-recovery-keyframe".to_string();
                    timeline.chain.state = "recovering".to_string();
                    timeline.chain.reason = Some("transportAwaitRecoveryKeyframe".to_string());
                    timeline.chain.observed_at_ms = now_ms - 2.0;
                    timeline.observed_at_ms = now_ms - 2.0;
                }
            },
        );
        assert!(
            commands.is_empty(),
            "same unresolved gap should not emit another recovery command at ts={observed_at_ms}: {commands:?}"
        );
        harness.with_stats(|stats| {
            let ledger = stats
                .latest_recovery_decision_ledger
                .as_ref()
                .expect("recovery decision ledger");
            assert_eq!(
                ledger.input_signal,
                "transportAwaitRecoveryKeyframe:transportAwaitRecoveryKeyframe"
            );
            assert_recovery_family_hold_semantics(
                ledger.gate_result.as_str(),
                ledger.action_selected.as_str(),
            );
        });
    }
}

#[test]
fn recovery_integration_passive_anchor_surface_still_feeds_transport_await_family_hold() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

    let first = harness.apply(
        16_000.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryKeyframe",
        260,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.session_phase = Some("recovering".to_string());
            stats.transport_recovery_epoch = 75;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 180;
            stats.latest_video_host_present_time_ms = Some(now_ms - 1_600.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 420.0);
            stats.latest_video_packet_arrival_time_ms = Some(now_ms - 2.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(true);
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 420_000,
                video_packet_count_total: 3_840,
                audio_bytes_total: 64_000,
                observed_at_ms: now_ms - 2.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 51,
                    source_event: "frame-await-recovery-keyframe".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                        observed_at_ms: now_ms - 2.0,
                    },
                    observed_at_ms: now_ms - 2.0,
                });
        },
    );
    assert!(first.iter().any(|command| {
        matches!(
            command,
            TransportCommand::RequestKeyframe { reason, .. }
                if reason == "transportAwaitRecoveryKeyframe"
        ) || matches!(
            command,
            TransportCommand::RequestDecoderReset { reason, .. }
                if reason == "transportAwaitRecoveryKeyframe"
        )
    }));

    let second = harness.apply(
        16_030.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryKeyframe",
        260,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.session_phase = Some("recovering".to_string());
            stats.transport_recovery_epoch = 75;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 220;
            stats.latest_video_host_present_time_ms = Some(now_ms - 1_900.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 460.0);
            stats.latest_video_packet_arrival_time_ms = Some(now_ms - 2.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(true);
            stats.latest_video_escalation_observation =
                Some(crate::XbxEngineVideoEscalationObservation {
                    observation_id: 52,
                    reason: "transportAwaitRecoveryKeyframe".to_string(),
                    action: "requestKeyframe".to_string(),
                    recovery_stage: "rebuilding-supply".to_string(),
                    recovery_chain_value: "anchor".to_string(),
                    recovery_failure_cost: "high".to_string(),
                    recovery_window_source: "transport-await-window".to_string(),
                    observed_at_ms: now_ms - 10.0,
                });
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total += 4_000;
                track.video_packet_count_total += 32;
                track.observed_at_ms = now_ms - 2.0;
            }
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.observation_id = 52;
                timeline.source_event = "frame-await-recovery-keyframe".to_string();
                timeline.chain.state = "recovering".to_string();
                timeline.chain.reason = Some("transportAwaitRecoveryKeyframe".to_string());
                timeline.chain.observed_at_ms = now_ms - 2.0;
                timeline.observed_at_ms = now_ms - 2.0;
            }
        },
    );
    assert!(
        second.is_empty(),
        "passive anchor surface should feed family hold instead of emitting a duplicate command: {second:?}"
    );
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(
            ledger.input_signal,
            "transportAwaitRecoveryKeyframe:transportAwaitRecoveryKeyframe"
        );
        assert_ne!(ledger.gate_result, "no-signal");
        assert_recovery_family_hold_semantics(
            ledger.gate_result.as_str(),
            ledger.action_selected.as_str(),
        );
    });
}

#[test]
fn recovery_integration_transport_await_reopens_after_clean_anchor_and_new_recovery_epoch() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

    let _ = harness.apply(
        12_000.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryKeyframe",
        280,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.session_phase = Some("recovering".to_string());
            stats.transport_recovery_epoch = 72;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 180;
            stats.latest_video_host_present_time_ms = Some(now_ms - 1_500.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 420.0);
            stats.latest_video_packet_arrival_time_ms = Some(now_ms - 2.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(true);
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 520_000,
                video_packet_count_total: 4_420,
                audio_bytes_total: 72_000,
                observed_at_ms: now_ms - 2.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 51,
                    source_event: "frame-await-recovery-keyframe".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                        observed_at_ms: now_ms - 2.0,
                    },
                    observed_at_ms: now_ms - 2.0,
                });
        },
    );

    let held = harness.apply(
        12_700.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryKeyframe",
        280,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.transport_recovery_epoch = 72;
            stats.latest_video_escalation_observation =
                Some(crate::XbxEngineVideoEscalationObservation {
                    observation_id: 51,
                    reason: "transportAwaitRecoveryKeyframe".to_string(),
                    action: "requestDecoderReset".to_string(),
                    recovery_stage: "rebuilding-supply".to_string(),
                    recovery_chain_value: "anchor".to_string(),
                    recovery_failure_cost: "high".to_string(),
                    recovery_window_source: "transport-await-window".to_string(),
                    observed_at_ms: now_ms - 80.0,
                });
            stats.latest_keyframe_request_episode =
                Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                    episode_id: 51,
                    request_reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                    request_kind: Some("control".to_string()),
                    status: "requested".to_string(),
                    requested_at_ms: now_ms - 700.0,
                    sent_at_ms: Some(now_ms - 690.0),
                    deadline_at_ms: Some(now_ms + 300.0),
                    first_keyframe_packet_at_ms: None,
                    first_keyframe_decoded_at_ms: None,
                    response_rtp_timestamp: None,
                    response_frame_seq: None,
                    response_verdict: Some("pending".to_string()),
                });
        },
    );
    assert!(
        held.is_empty(),
        "unexpected commands while same recovery episode remains in-flight: {held:?}"
    );

    let reopened = harness.apply(
        13_020.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryKeyframe",
        289,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.session_phase = Some("recovering".to_string());
            stats.transport_recovery_epoch = 73;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 210;
            stats.latest_video_host_present_time_ms = Some(now_ms - 1_800.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 520.0);
            stats.latest_video_packet_arrival_time_ms = Some(now_ms - 2.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(true);
            stats.video_anchor_clean_epoch = Some(72);
            stats.video_anchor_clean_observed_at_ms = Some(now_ms - 220.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-keyframe-submitted".to_string());
            stats.latest_video_escalation_observation = None;
            stats.latest_keyframe_request_episode = None;
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total += 16_000;
                track.video_packet_count_total += 110;
                track.observed_at_ms = now_ms - 2.0;
            }
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 52,
                    source_event: "frame-await-recovery-keyframe".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                        observed_at_ms: now_ms - 2.0,
                    },
                    observed_at_ms: now_ms - 2.0,
                });
        },
    );
    assert!(reopened.iter().any(|command| {
        matches!(
            command,
            TransportCommand::RequestKeyframe { reason, .. }
                if reason == "transportAwaitRecoveryKeyframe"
        ) || matches!(
            command,
            TransportCommand::RequestDecoderReset { reason, .. }
                if reason == "transportAwaitRecoveryKeyframe"
        )
    }));
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(
            ledger.input_signal,
            "transportAwaitRecoveryKeyframe:transportAwaitRecoveryKeyframe"
        );
        assert_eq!(ledger.gate_result, "pass");
        assert_ne!(ledger.action_selected, "none");
    });
}

#[test]
fn recovery_integration_home_local_display_recovery_then_stale_transport_await_replay_stays_absorbed(
) {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Home));

    let baseline = harness.apply(
        7_000.0,
        ConnectionLifecycleStateFact::Connected,
        "none",
        48,
        |stats| {
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 21;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(6_992.0);
            stats.latest_video_decode_ok_time_ms = Some(6_996.0);
            stats.latest_video_packet_arrival_time_ms = Some(6_997.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = Some(21);
            stats.video_anchor_clean_observed_at_ms = Some(6_998.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-keyframe-submitted".to_string());
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 320_000,
                video_packet_count_total: 2_400,
                audio_bytes_total: 48_000,
                observed_at_ms: 6_998.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-observed".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        observed_at_ms: 6_998.0,
                    },
                    observed_at_ms: 6_998.0,
                });
        },
    );
    assert!(
        baseline.is_empty(),
        "unexpected baseline commands: {baseline:?}"
    );

    let burst = harness.apply(
        7_120.0,
        ConnectionLifecycleStateFact::Connected,
        "none",
        49,
        |stats| {
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 132;
            stats.latest_video_host_present_time_ms = Some(6_306.0);
            stats.latest_video_decode_ok_time_ms = Some(7_108.0);
            stats.latest_video_packet_arrival_time_ms = Some(7_118.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total += 18_000;
                track.video_packet_count_total += 120;
                track.observed_at_ms = 7_118.0;
            }
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.observation_id += 1;
                timeline.source_event = "frame-complete-candidate".to_string();
                timeline.chain.state = "healthy".to_string();
                timeline.chain.reason = None;
                timeline.chain.observed_at_ms = 7_118.0;
                timeline.observed_at_ms = 7_118.0;
            }
        },
    );
    assert!(burst.iter().any(|command| {
        matches!(
            command,
            TransportCommand::RequestKeyframe { reason, .. } if reason == "displaySupplyDegraded"
        )
    }));
    assert!(burst
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })));

    let recovered = harness.apply(
        7_190.0,
        ConnectionLifecycleStateFact::Connected,
        "none",
        54,
        |stats| {
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(7_182.0);
            stats.latest_video_decode_ok_time_ms = Some(7_186.0);
            stats.latest_video_packet_arrival_time_ms = Some(7_188.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = Some(21);
            stats.video_anchor_clean_observed_at_ms = Some(7_189.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-keyframe-submitted".to_string());
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total += 20_000;
                track.video_packet_count_total += 132;
                track.observed_at_ms = 7_189.0;
            }
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.observation_id += 1;
                timeline.source_event = "frame-observed".to_string();
                timeline.chain.state = "healthy".to_string();
                timeline.chain.reason = None;
                timeline.chain.observed_at_ms = 7_189.0;
                timeline.observed_at_ms = 7_189.0;
            }
        },
    );
    assert!(
        recovered.is_empty(),
        "local display recovery should settle without extra commands: {recovered:?}"
    );

    let replay = harness.apply_with_recovery_observed_at(
        7_214.0,
        6_990.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryKeyframe",
        57,
        |stats| {
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(7_205.0);
            stats.latest_video_decode_ok_time_ms = Some(7_208.0);
            stats.latest_video_packet_arrival_time_ms = Some(7_210.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = Some(21);
            stats.video_anchor_clean_observed_at_ms = Some(7_212.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-keyframe-submitted".to_string());
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total += 24_000;
                track.video_packet_count_total += 160;
                track.observed_at_ms = 7_212.0;
            }
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.observation_id += 1;
                timeline.source_event = "frame-observed".to_string();
                timeline.chain.state = "healthy".to_string();
                timeline.chain.reason = None;
                timeline.chain.observed_at_ms = 7_212.0;
                timeline.observed_at_ms = 7_212.0;
            }
        },
    );

    assert!(
        replay.is_empty(),
        "stale transportAwait replay after host recovery should stay absorbed: {replay:?}"
    );
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(ledger.input_signal, "none");
        assert_eq!(ledger.gate_result, "no-signal");
        assert_eq!(ledger.action_selected, "none");
    });
}

#[test]
fn recovery_integration_ramp_up_absorbs_display_idle_and_short_transport_await_before_stable() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Home));

    let recovering = harness.apply(
        12_000.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryKeyframe",
        300,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.session_phase = Some("recovering".to_string());
            stats.transport_recovery_epoch = 91;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 188;
            stats.latest_video_host_present_time_ms = Some(now_ms - 980.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 18.0);
            stats.latest_video_packet_arrival_time_ms = Some(now_ms - 4.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 680_000,
                video_packet_count_total: 5_640,
                audio_bytes_total: 112_000,
                observed_at_ms: now_ms - 4.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-await-recovery-keyframe".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                        observed_at_ms: now_ms - 4.0,
                    },
                    observed_at_ms: now_ms - 4.0,
                });
        },
    );
    assert!(recovering.iter().any(|command| {
        matches!(
            command,
            TransportCommand::RequestKeyframe { reason, .. }
                if reason == "transportAwaitRecoveryKeyframe"
        )
    }));

    let ramp_display = harness.apply(
        12_040.0,
        ConnectionLifecycleStateFact::Connected,
        "displaySupplyDegraded",
        304,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.session_phase = Some("recovering".to_string());
            stats.host_no_pending_pressure_level = Some("high".to_string());
            stats.host_no_pending_streak = 84;
            stats.latest_video_host_present_time_ms = Some(now_ms - 240.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 8.0);
            stats.latest_video_packet_arrival_time_ms = Some(now_ms - 2.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = Some(91);
            stats.video_anchor_clean_observed_at_ms = Some(now_ms - 1.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-keyframe-submitted".to_string());
            stats.latest_video_escalation_observation = None;
            stats.latest_keyframe_request_episode = None;
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total += 22_000;
                track.video_packet_count_total += 164;
                track.observed_at_ms = now_ms - 2.0;
            }
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.observation_id += 1;
                timeline.source_event = "frame-complete-candidate".to_string();
                timeline.chain.state = "healthy".to_string();
                timeline.chain.reason = None;
                timeline.chain.observed_at_ms = now_ms - 2.0;
                timeline.observed_at_ms = now_ms - 2.0;
            }
        },
    );
    assert!(
        ramp_display.is_empty(),
        "ramp-up display pressure should be absorbed locally: {ramp_display:?}"
    );
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_ne!(ledger.gate_result, "pass");
    });

    let ramp_idle = harness.apply(
        12_090.0,
        ConnectionLifecycleStateFact::Connected,
        "adapterIdleTimeout",
        307,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.session_phase = Some("recovering".to_string());
            stats.host_no_pending_pressure_level = Some("high".to_string());
            stats.host_no_pending_streak = 36;
            stats.latest_video_host_present_time_ms = Some(now_ms - 78.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 8.0);
            stats.latest_video_packet_arrival_time_ms = Some(now_ms - 2.0);
            stats.video_anchor_clean_epoch = Some(91);
            stats.video_anchor_clean_observed_at_ms = Some(now_ms - 2.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-keyframe-submitted".to_string());
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total += 18_000;
                track.video_packet_count_total += 136;
                track.observed_at_ms = now_ms - 2.0;
            }
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.observation_id += 1;
                timeline.source_event = "frame-observed".to_string();
                timeline.chain.state = "healthy".to_string();
                timeline.chain.reason = None;
                timeline.chain.observed_at_ms = now_ms - 2.0;
                timeline.observed_at_ms = now_ms - 2.0;
            }
        },
    );
    assert!(
        ramp_idle.is_empty(),
        "ramp-up adapter idle noise should stay absorbed: {ramp_idle:?}"
    );
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(ledger.gate_result, "no-signal");
        assert_eq!(ledger.action_selected, "none");
    });

    let short_transport = harness.apply_with_recovery_observed_at(
        12_130.0,
        12_126.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryKeyframe",
        311,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.session_phase = Some("recovering".to_string());
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 6;
            stats.latest_video_host_present_time_ms = Some(now_ms - 58.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 8.0);
            stats.latest_video_packet_arrival_time_ms = Some(now_ms - 2.0);
            stats.video_anchor_clean_epoch = Some(91);
            stats.video_anchor_clean_observed_at_ms = Some(now_ms - 2.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-keyframe-submitted".to_string());
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total += 14_000;
                track.video_packet_count_total += 104;
                track.observed_at_ms = now_ms - 2.0;
            }
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.observation_id += 1;
                timeline.source_event = "frame-observed".to_string();
                timeline.chain.state = "healthy".to_string();
                timeline.chain.reason = None;
                timeline.chain.observed_at_ms = now_ms - 2.0;
                timeline.observed_at_ms = now_ms - 2.0;
            }
        },
    );
    assert!(
        short_transport.is_empty(),
        "short transportAwait in ramp-up should not reopen recovery: {short_transport:?}"
    );
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(ledger.gate_result, "no-signal");
        assert_eq!(ledger.action_selected, "none");
    });

    let settled = harness.apply(
        12_220.0,
        ConnectionLifecycleStateFact::Connected,
        "none",
        320,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.session_phase = Some("steady".to_string());
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(now_ms - 8.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 4.0);
            stats.latest_video_packet_arrival_time_ms = Some(now_ms - 2.0);
            stats.video_anchor_clean_epoch = Some(91);
            stats.video_anchor_clean_observed_at_ms = Some(now_ms - 1.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-keyframe-submitted".to_string());
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total += 26_000;
                track.video_packet_count_total += 188;
                track.observed_at_ms = now_ms - 2.0;
            }
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.observation_id += 1;
                timeline.source_event = "frame-observed".to_string();
                timeline.chain.state = "healthy".to_string();
                timeline.chain.reason = None;
                timeline.chain.observed_at_ms = now_ms - 2.0;
                timeline.observed_at_ms = now_ms - 2.0;
            }
        },
    );
    assert!(
        settled.is_empty(),
        "ramp-up should settle into stable without extra commands: {settled:?}"
    );
    harness.with_stats(|stats| {
        assert_eq!(stats.video_owner_state.as_deref(), Some("stable-serving"));
    });
}

#[test]
fn recovery_integration_ramp_up_still_reescalates_on_severe_transport_await() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Home));

    let _ = harness.apply(
        13_000.0,
        ConnectionLifecycleStateFact::Connected,
        "none",
        360,
        |stats| {
            stats.session_phase = Some("recovering".to_string());
            stats.transport_recovery_epoch = 92;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("high".to_string());
            stats.host_no_pending_streak = 48;
            stats.latest_video_host_present_time_ms = Some(12_940.0);
            stats.latest_video_decode_ok_time_ms = Some(12_996.0);
            stats.latest_video_packet_arrival_time_ms = Some(12_998.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = Some(92);
            stats.video_anchor_clean_observed_at_ms = Some(12_999.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-keyframe-submitted".to_string());
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 820_000,
                video_packet_count_total: 6_700,
                audio_bytes_total: 140_000,
                observed_at_ms: 12_998.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-observed".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        observed_at_ms: 12_998.0,
                    },
                    observed_at_ms: 12_998.0,
                });
        },
    );

    let severe = harness.apply_with_recovery_observed_at(
        13_080.0,
        13_080.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryKeyframe",
        361,
        |stats| {
            stats.session_phase = Some("recovering".to_string());
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 210;
            stats.latest_video_host_present_time_ms = Some(11_780.0);
            stats.latest_video_decode_ok_time_ms = Some(12_700.0);
            stats.latest_video_packet_arrival_time_ms = Some(13_078.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(true);
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total += 8_000;
                track.video_packet_count_total += 42;
                track.observed_at_ms = 13_078.0;
            }
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.observation_id += 1;
                timeline.source_event = "frame-await-recovery-keyframe".to_string();
                timeline.chain.state = "recovering".to_string();
                timeline.chain.reason = Some("transportAwaitRecoveryKeyframe".to_string());
                timeline.chain.observed_at_ms = 13_078.0;
                timeline.observed_at_ms = 13_078.0;
            }
        },
    );

    assert!(severe.iter().any(|command| {
        matches!(
            command,
            TransportCommand::RequestKeyframe { reason, .. }
                if reason == "transportAwaitRecoveryKeyframe"
        ) || matches!(
            command,
            TransportCommand::RequestDecoderReset { reason, .. }
                if reason == "transportAwaitRecoveryKeyframe"
        )
    }));
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(
            ledger.input_signal,
            "transportAwaitRecoveryKeyframe:transportAwaitRecoveryKeyframe"
        );
        assert_eq!(ledger.gate_result, "pass");
        assert_ne!(ledger.action_selected, "none");
    });
}

#[test]
fn recovery_integration_degraded_serving_does_not_close_as_recovered() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Home));

    let baseline = harness.apply(
        8_000.0,
        ConnectionLifecycleStateFact::Connected,
        "none",
        120,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 31;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(now_ms - 8.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 4.0);
            stats.latest_video_packet_arrival_time_ms = Some(now_ms - 2.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = Some(31);
            stats.video_anchor_clean_observed_at_ms = Some(now_ms - 3.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-keyframe-submitted".to_string());
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 360_000,
                video_packet_count_total: 3_840,
                audio_bytes_total: 64_000,
                observed_at_ms: now_ms - 2.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-observed".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        observed_at_ms: now_ms - 2.0,
                    },
                    observed_at_ms: now_ms - 2.0,
                });
        },
    );
    assert!(
        baseline.is_empty(),
        "unexpected baseline commands: {baseline:?}"
    );

    let degraded = harness.apply(
        8_120.0,
        ConnectionLifecycleStateFact::Connected,
        "none",
        121,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.host_no_pending_pressure_level = Some("high".to_string());
            stats.host_no_pending_streak = 96;
            stats.latest_video_host_present_time_ms = Some(now_ms - 720.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 280.0);
            stats.latest_video_packet_arrival_time_ms = Some(now_ms - 180.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total += 18_000;
                track.video_packet_count_total += 120;
                track.observed_at_ms = now_ms - 2.0;
            }
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.observation_id += 1;
                timeline.source_event = "frame-complete-candidate".to_string();
                timeline.chain.state = "healthy".to_string();
                timeline.chain.reason = None;
                timeline.chain.observed_at_ms = now_ms - 2.0;
                timeline.observed_at_ms = now_ms - 2.0;
            }
        },
    );
    assert!(degraded
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })));
    harness.with_stats(|stats| {
        assert_ne!(stats.video_owner_state.as_deref(), Some("stable-serving"));
    });

    let replay = harness.apply_with_recovery_observed_at(
        8_260.0,
        8_000.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryKeyframe",
        122,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.host_no_pending_pressure_level = Some("high".to_string());
            stats.host_no_pending_streak = 110;
            stats.latest_video_host_present_time_ms = Some(now_ms - 760.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 300.0);
            stats.latest_video_packet_arrival_time_ms = Some(now_ms - 190.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total += 16_000;
                track.video_packet_count_total += 108;
                track.observed_at_ms = now_ms - 2.0;
            }
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.observation_id += 1;
                timeline.source_event = "frame-complete-candidate".to_string();
                timeline.chain.state = "healthy".to_string();
                timeline.chain.reason = None;
                timeline.chain.observed_at_ms = now_ms - 2.0;
                timeline.observed_at_ms = now_ms - 2.0;
            }
        },
    );

    assert!(replay
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })));
    harness.with_stats(|stats| {
        assert_ne!(stats.video_owner_state.as_deref(), Some("stable-serving"));
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_ne!(ledger.state_after, "stable");
    });
}

#[test]
fn recovery_integration_steady_serving_ignores_stale_transport_await_diagnosis() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

    let baseline = harness.apply(
        2_000.0,
        ConnectionLifecycleStateFact::Connected,
        "none",
        90,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 18;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(now_ms - 8.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 4.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = Some(18);
            stats.video_anchor_clean_observed_at_ms = Some(now_ms - 3.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-keyframe-submitted".to_string());
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 420_000,
                video_packet_count_total: 5_040,
                audio_bytes_total: 110_000,
                observed_at_ms: now_ms - 2.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-observed".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        observed_at_ms: now_ms - 2.0,
                    },
                    observed_at_ms: now_ms - 2.0,
                });
        },
    );
    assert!(
        baseline.is_empty(),
        "unexpected baseline commands: {baseline:?}"
    );

    let commands = harness.apply_with_recovery_observed_at(
        2_620.0,
        2_000.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryKeyframe",
        91,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(now_ms - 8.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 4.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = Some(18);
            stats.video_anchor_clean_observed_at_ms = Some(now_ms - 3.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-keyframe-submitted".to_string());
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total = 468_000;
                track.video_packet_count_total = 5_480;
                track.observed_at_ms = now_ms - 2.0;
            }
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.source_event = "frame-observed".to_string();
                timeline.chain.state = "healthy".to_string();
                timeline.chain.reason = None;
                timeline.chain.observed_at_ms = now_ms - 2.0;
                timeline.observed_at_ms = now_ms - 2.0;
            }
        },
    );

    assert!(
        commands.is_empty(),
        "unexpected commands when stale transportAwait tries to reopen recovery: {commands:?}"
    );
    harness.with_stats(|stats| {
        assert_eq!(stats.video_owner_state.as_deref(), Some("stable-serving"));
        assert_eq!(stats.video_owner_reason.as_deref(), Some("steady"));
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(ledger.input_signal, "none");
        assert_eq!(ledger.gate_result, "no-signal");
        assert_eq!(ledger.action_selected, "none");
    });
}

#[test]
fn recovery_integration_fresh_transport_await_does_not_override_stable_owner_without_clean_anchor()
{
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

    let _ = harness.apply(
        900.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryKeyframe",
        12,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.transport_recovery_epoch = 19;
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 24;
            stats.latest_video_host_present_time_ms = Some(now_ms - 260.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 180.0);
            stats.video_renderer_stalled = Some(false);
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 155_000,
                video_packet_count_total: 1_200,
                audio_bytes_total: 42_000,
                observed_at_ms: 900.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-await-recovery-keyframe".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                        observed_at_ms: 900.0,
                    },
                    observed_at_ms: 900.0,
                });
        },
    );

    let healed = harness.apply(
        930.0,
        ConnectionLifecycleStateFact::Connected,
        "none",
        13,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(now_ms - 8.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 5.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.source_event = "frame-complete-candidate".to_string();
                timeline.chain.state = "healthy".to_string();
                timeline.chain.reason = None;
                timeline.chain.observed_at_ms = 930.0;
                timeline.observed_at_ms = 930.0;
            }
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total = 188_000;
                track.video_packet_count_total = 1_360;
                track.observed_at_ms = 930.0;
            }
        },
    );
    assert!(
        healed.is_empty(),
        "unexpected commands after owner healed: {healed:?}"
    );
    harness.with_stats(|stats| {
        assert_eq!(stats.video_owner_state.as_deref(), Some("stable-serving"));
        assert_eq!(stats.video_owner_reason.as_deref(), Some("steady"));
    });

    let commands = harness.apply_with_recovery_observed_at(
        960.0,
        960.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryKeyframe",
        14,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(now_ms - 7.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 4.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.source_event = "frame-complete-candidate".to_string();
                timeline.chain.state = "healthy".to_string();
                timeline.chain.reason = None;
                timeline.chain.observed_at_ms = 960.0;
                timeline.observed_at_ms = 960.0;
            }
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total = 220_000;
                track.video_packet_count_total = 1_520;
                track.observed_at_ms = 960.0;
            }
        },
    );

    assert!(
        commands.is_empty(),
        "unexpected commands when fresh transportAwait conflicts with stable owner: {commands:?}"
    );
    harness.with_stats(|stats| {
        assert_eq!(stats.video_owner_state.as_deref(), Some("stable-serving"));
        assert_eq!(stats.video_owner_reason.as_deref(), Some("steady"));
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(ledger.input_signal, "none");
        assert_eq!(ledger.gate_result, "no-signal");
        assert_eq!(ledger.action_selected, "none");
    });
}

#[test]
fn recovery_integration_local_display_stays_local_recovery() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));
    let commands = harness.apply(
        760.0,
        ConnectionLifecycleStateFact::Connected,
        "none",
        24,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.transport_recovery_epoch = 3;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 260;
            stats.latest_video_host_present_time_ms = Some(now_ms - 1_200.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 600.0);
            stats.video_renderer_stalled = Some(true);
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 128_000,
                video_packet_count_total: 1_024,
                audio_bytes_total: 32_000,
                observed_at_ms: 760.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-observed".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        observed_at_ms: 760.0,
                    },
                    observed_at_ms: 760.0,
                });
        },
    );
    assert!(commands.iter().any(
        |command| matches!(command, TransportCommand::RequestKeyframe { reason, .. } if reason == "displaySupplyCritical")
    ));
    assert!(commands
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })));
    harness.with_stats(|stats| {
        assert_eq!(stats.video_owner_state.as_deref(), Some("supply-starved"));
        assert_eq!(
            stats.video_owner_reason.as_deref(),
            Some("displaySupplyCritical")
        );
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(
            ledger.input_signal,
            "displaySupplyCritical:displaySupplyCritical"
        );
        assert_eq!(ledger.action_selected, "requestKeyframe");
    });
}

#[test]
fn recovery_integration_stale_idle_is_absorbed_but_transport_failure_still_reconnects() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

    let healthy = harness.apply(
        1_000.0,
        ConnectionLifecycleStateFact::Connected,
        "none",
        31,
        |stats| {
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 7;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(988.0);
            stats.latest_video_decode_ok_time_ms = Some(992.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = Some(7);
            stats.video_anchor_clean_observed_at_ms = Some(994.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-keyframe-submitted".to_string());
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 128_000,
                video_packet_count_total: 1_600,
                audio_bytes_total: 36_000,
                observed_at_ms: 995.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-observed".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        observed_at_ms: 995.0,
                    },
                    observed_at_ms: 995.0,
                });
        },
    );
    assert!(healthy.is_empty());

    let stale_idle = harness.apply(
        1_008.0,
        ConnectionLifecycleStateFact::Connected,
        "adapterIdleTimeout",
        32,
        |_stats| {},
    );
    assert!(
        stale_idle.is_empty(),
        "unexpected stale idle commands: {stale_idle:?}"
    );
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(ledger.gate_result, "no-signal");
        assert_eq!(ledger.action_selected, "none");
    });

    let reconnect = harness.apply(
        1_200.0,
        ConnectionLifecycleStateFact::Disconnected,
        "rtcConnectionDisconnected",
        32,
        |stats| {
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            stats.video_renderer_stalled = Some(true);
        },
    );
    assert!(reconnect.iter().any(|command| {
        matches!(
            command,
            TransportCommand::RequestReconnectCandidate {
                reason,
                reason_domain,
                ..
            } if reason == "rtcConnectionDisconnected"
                && *reason_domain == crate::XbxEngineRecoveryReasonDomain::ConnectivityTransport
        )
    }));
}

#[test]
fn recovery_integration_fresh_transport_await_absorption_does_not_block_following_transport_disconnect(
) {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

    let baseline = harness.apply(
        2_000.0,
        ConnectionLifecycleStateFact::Connected,
        "none",
        90,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 20;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(now_ms - 8.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 4.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 420_000,
                video_packet_count_total: 5_040,
                audio_bytes_total: 110_000,
                observed_at_ms: now_ms - 2.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-complete-candidate".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        observed_at_ms: now_ms - 2.0,
                    },
                    observed_at_ms: now_ms - 2.0,
                });
        },
    );
    assert!(
        baseline.is_empty(),
        "unexpected baseline commands: {baseline:?}"
    );

    let absorbed = harness.apply_with_recovery_observed_at(
        2_030.0,
        2_030.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryKeyframe",
        91,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(now_ms - 8.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 4.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total = 452_000;
                track.video_packet_count_total = 5_260;
                track.observed_at_ms = now_ms - 2.0;
            }
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.source_event = "frame-complete-candidate".to_string();
                timeline.chain.state = "healthy".to_string();
                timeline.chain.reason = None;
                timeline.chain.observed_at_ms = now_ms - 2.0;
                timeline.observed_at_ms = now_ms - 2.0;
            }
        },
    );
    assert!(
        absorbed.is_empty(),
        "unexpected commands when fresh transportAwait should be absorbed: {absorbed:?}"
    );
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(ledger.input_signal, "none");
        assert_eq!(ledger.gate_result, "no-signal");
        assert_eq!(ledger.action_selected, "none");
        assert_eq!(stats.video_owner_state.as_deref(), Some("stable-serving"));
    });

    let reconnect = harness.apply(
        2_120.0,
        ConnectionLifecycleStateFact::Disconnected,
        "rtcConnectionDisconnected",
        91,
        |stats| {
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Disconnected;
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            stats.video_renderer_stalled = Some(true);
        },
    );
    assert!(reconnect.iter().any(|command| {
        matches!(
            command,
            TransportCommand::RequestReconnectCandidate {
                reason,
                reason_domain,
                ..
            } if reason == "rtcConnectionDisconnected"
                && *reason_domain == crate::XbxEngineRecoveryReasonDomain::ConnectivityTransport
        )
    }));
}

#[test]
fn recovery_integration_recovering_lifecycle_overrides_fresh_transport_await_absorption() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

    let commands = harness.apply_with_recovery_observed_at(
        2_400.0,
        2_400.0,
        ConnectionLifecycleStateFact::Recovering,
        "transportAwaitRecoveryKeyframe",
        64,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 21;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(now_ms - 8.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 4.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 320_000,
                video_packet_count_total: 3_400,
                audio_bytes_total: 88_000,
                observed_at_ms: now_ms - 2.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-complete-candidate".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        observed_at_ms: now_ms - 2.0,
                    },
                    observed_at_ms: now_ms - 2.0,
                });
        },
    );

    assert!(commands.iter().any(|command| {
        matches!(
            command,
            TransportCommand::RequestReconnectCandidate {
                reason,
                reason_domain,
                ..
            } if reason == "rtcConnectionRecovering"
                && *reason_domain == crate::XbxEngineRecoveryReasonDomain::ConnectivityTransport
        )
    }));
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(
            ledger.input_signal,
            "rtcConnectionRecovering:rtcConnectionRecovering"
        );
        assert_eq!(ledger.gate_result, "pass");
    });
}

#[test]
fn recovery_integration_transport_deadline_overrides_same_tick_local_display_recovery() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

    let commands = harness.apply(
        2_800.0,
        ConnectionLifecycleStateFact::Connected,
        "transportExpiredDeadline",
        72,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 22;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Disconnected;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 280;
            stats.latest_video_host_present_time_ms = Some(now_ms - 1_300.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 620.0);
            stats.video_renderer_stalled = Some(true);
            stats.video_decoder_stalled = Some(false);
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Disconnected,
                video_bytes_total: 128_000,
                video_packet_count_total: 1_024,
                audio_bytes_total: 32_000,
                observed_at_ms: now_ms - 4.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-observed".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        observed_at_ms: now_ms - 4.0,
                    },
                    observed_at_ms: now_ms - 4.0,
                });
        },
    );

    assert!(commands.iter().all(|command| {
        !matches!(
            command,
            TransportCommand::RequestKeyframe { reason, .. }
                if reason == "displaySupplyCritical"
        )
    }));
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(
            ledger.input_signal,
            "transportExpiredDeadline:transportExpiredDeadline"
        );
        assert_eq!(ledger.gate_result, "pass");
    });
}

#[test]
fn recovery_integration_transport_severe_deadline_overrides_same_tick_local_display_recovery() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

    let commands = harness.apply(
        2_820.0,
        ConnectionLifecycleStateFact::Connected,
        "transportSevereDeadline",
        73,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 23;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Disconnected;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 320;
            stats.latest_video_host_present_time_ms = Some(now_ms - 1_180.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 700.0);
            stats.video_renderer_stalled = Some(true);
            stats.video_decoder_stalled = Some(false);
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Disconnected,
                video_bytes_total: 160_000,
                video_packet_count_total: 1_280,
                audio_bytes_total: 38_000,
                observed_at_ms: now_ms - 5.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 2,
                    source_event: "frame-observed".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        observed_at_ms: now_ms - 5.0,
                    },
                    observed_at_ms: now_ms - 5.0,
                });
        },
    );

    assert!(commands.iter().all(|command| {
        !matches!(
            command,
            TransportCommand::RequestKeyframe { reason, .. }
                if reason == "displaySupplyCritical"
        )
    }));
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(
            ledger.input_signal,
            "transportSevereDeadline:transportSevereDeadline"
        );
        assert_recovery_family_hold_semantics(
            ledger.gate_result.as_str(),
            ledger.action_selected.as_str(),
        );
    });
}

#[test]
fn recovery_integration_transport_deadline_overrides_same_tick_local_transport_await() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

    let commands = harness.apply(
        2_860.0,
        ConnectionLifecycleStateFact::Connected,
        "transportExpiredDeadline",
        74,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 24;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Disconnected;
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 12;
            stats.latest_video_host_present_time_ms = Some(now_ms - 220.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 180.0);
            stats.video_renderer_stalled = Some(false);
            stats.video_decoder_stalled = Some(false);
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Disconnected,
                video_bytes_total: 192_000,
                video_packet_count_total: 1_540,
                audio_bytes_total: 44_000,
                observed_at_ms: now_ms - 4.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 3,
                    source_event: "frame-await-recovery-keyframe".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                        observed_at_ms: now_ms - 3.0,
                    },
                    observed_at_ms: now_ms - 3.0,
                });
        },
    );

    assert!(commands.iter().all(|command| {
        !matches!(
            command,
            TransportCommand::RequestKeyframe { reason, .. }
                if reason == "transportAwaitRecoveryKeyframe"
        )
    }));
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(
            ledger.input_signal,
            "transportExpiredDeadline:transportExpiredDeadline"
        );
        assert_eq!(ledger.gate_result, "pass");
    });
}

#[test]
fn recovery_integration_transport_severe_deadline_overrides_same_tick_local_transport_await() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

    let commands = harness.apply(
        2_900.0,
        ConnectionLifecycleStateFact::Connected,
        "transportSevereDeadline",
        75,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 25;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Disconnected;
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 8;
            stats.latest_video_host_present_time_ms = Some(now_ms - 260.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 210.0);
            stats.video_renderer_stalled = Some(false);
            stats.video_decoder_stalled = Some(false);
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Disconnected,
                video_bytes_total: 205_000,
                video_packet_count_total: 1_620,
                audio_bytes_total: 46_000,
                observed_at_ms: now_ms - 5.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 4,
                    source_event: "frame-await-recovery-keyframe".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                        observed_at_ms: now_ms - 4.0,
                    },
                    observed_at_ms: now_ms - 4.0,
                });
        },
    );

    assert!(commands.iter().all(|command| {
        !matches!(
            command,
            TransportCommand::RequestKeyframe { reason, .. }
                if reason == "transportAwaitRecoveryKeyframe"
        )
    }));
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(
            ledger.input_signal,
            "transportSevereDeadline:transportSevereDeadline"
        );
        assert_recovery_family_hold_semantics(
            ledger.gate_result.as_str(),
            ledger.action_selected.as_str(),
        );
    });
}

#[test]
fn recovery_integration_repeated_transport_severe_deadline_upgrades_to_connectivity_reconnect() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

    let first = harness.apply(
        2_940.0,
        ConnectionLifecycleStateFact::Connected,
        "transportSevereDeadline",
        76,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 26;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Disconnected;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 340;
            stats.latest_video_host_present_time_ms = Some(now_ms - 1_260.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 760.0);
            stats.video_renderer_stalled = Some(true);
            stats.video_decoder_stalled = Some(false);
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Disconnected,
                video_bytes_total: 212_000,
                video_packet_count_total: 1_700,
                audio_bytes_total: 48_000,
                observed_at_ms: now_ms - 5.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 5,
                    source_event: "frame-await-recovery-keyframe".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                        observed_at_ms: now_ms - 5.0,
                    },
                    observed_at_ms: now_ms - 5.0,
                });
        },
    );
    assert!(
        first.is_empty(),
        "first severe signal should stay in cooldown on first hit: {first:?}"
    );

    let second = harness.apply(
        2_970.0,
        ConnectionLifecycleStateFact::Connected,
        "transportSevereDeadline",
        76,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 360;
            stats.latest_video_host_present_time_ms = Some(now_ms - 1_320.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 810.0);
            stats.video_renderer_stalled = Some(true);
            stats.video_decoder_stalled = Some(false);
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.observation_id = 6;
                timeline.source_event = "frame-await-recovery-keyframe".to_string();
                timeline.chain.state = "recovering".to_string();
                timeline.chain.reason = Some("transportAwaitRecoveryKeyframe".to_string());
                timeline.chain.observed_at_ms = now_ms - 4.0;
                timeline.observed_at_ms = now_ms - 4.0;
            }
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.observed_at_ms = now_ms - 4.0;
            }
        },
    );

    assert!(second.iter().any(|command| {
        matches!(
            command,
            TransportCommand::RequestReconnectCandidate {
                reason,
                reason_domain,
                ..
            } if reason == "transportSevereDeadline"
                && *reason_domain == crate::XbxEngineRecoveryReasonDomain::ConnectivityTransport
        )
    }));
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(
            ledger.input_signal,
            "transportSevereDeadline:transportSevereDeadline"
        );
        assert_eq!(ledger.gate_result, "pass");
        assert_eq!(ledger.action_selected, "requestReconnectCandidate");
    });
}

#[test]
fn recovery_integration_repeated_transport_expired_deadline_upgrades_to_connectivity_reconnect() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

    let mut last_commands = Vec::new();
    for idx in 0..3 {
        last_commands = harness.apply(
            3_020.0 + (idx as f64) * 420.0,
            ConnectionLifecycleStateFact::Connected,
            "transportExpiredDeadline",
            80,
            |stats| {
                let now_ms = crate::transport::rtc::stats::now_ms_f64();
                stats.session_phase = Some("steady".to_string());
                stats.transport_recovery_epoch = 27;
                stats.transport_state =
                    xbxengine_protocol::XbxEngineTransportStateDto::Disconnected;
                stats.host_no_pending_pressure_level = Some("critical".to_string());
                stats.host_no_pending_streak = 380;
                stats.latest_video_host_present_time_ms = Some(now_ms - 1_420.0);
                stats.latest_video_decode_ok_time_ms = Some(now_ms - 920.0);
                stats.video_renderer_stalled = Some(true);
                stats.video_decoder_stalled = Some(false);
                stats.video_anchor_clean_epoch = None;
                stats.video_anchor_clean_observed_at_ms = None;
                stats.video_anchor_clean_source_event = None;
                stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                    state: "remoteTrackAttached".to_string(),
                    video_width: Some(1920),
                    video_height: Some(1080),
                    mime_type: Some("video/H264".to_string()),
                    transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Disconnected,
                    video_bytes_total: 240_000,
                    video_packet_count_total: 1_900,
                    audio_bytes_total: 60_000,
                    observed_at_ms: now_ms - 5.0,
                });
                stats.latest_video_timeline_observation =
                    Some(crate::XbxEngineVideoTimelineObservation {
                        observation_id: 7 + idx as u64,
                        source_event: "frame-await-recovery-keyframe".to_string(),
                        gap: None,
                        frame: None,
                        chain: crate::XbxEngineVideoTimelineChainSnapshot {
                            state: "recovering".to_string(),
                            reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                            observed_at_ms: now_ms - 5.0,
                        },
                        observed_at_ms: now_ms - 5.0,
                    });
            },
        );
        if idx < 2 {
            sleep(Duration::from_millis(450));
        }
    }

    assert!(last_commands.iter().any(|command| {
        matches!(
            command,
            TransportCommand::RequestReconnectCandidate {
                reason,
                reason_domain,
                ..
            } if reason == "transportExpiredDeadline"
                && *reason_domain == crate::XbxEngineRecoveryReasonDomain::ConnectivityTransport
        )
    }));
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(
            ledger.input_signal,
            "transportExpiredDeadline:transportExpiredDeadline"
        );
        assert_eq!(ledger.gate_result, "pass");
        assert_eq!(ledger.action_selected, "requestReconnectCandidate");
    });
}

#[test]
fn recovery_integration_transport_sample_loss_overrides_same_tick_local_display_recovery() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

    let commands = harness.apply(
        3_120.0,
        ConnectionLifecycleStateFact::Connected,
        "transportSampleLoss",
        82,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 28;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Disconnected;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 340;
            stats.latest_video_host_present_time_ms = Some(now_ms - 1_280.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 760.0);
            stats.video_renderer_stalled = Some(true);
            stats.video_decoder_stalled = Some(false);
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Disconnected,
                video_bytes_total: 208_000,
                video_packet_count_total: 1_650,
                audio_bytes_total: 50_000,
                observed_at_ms: now_ms - 4.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 10,
                    source_event: "frame-observed".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        observed_at_ms: now_ms - 4.0,
                    },
                    observed_at_ms: now_ms - 4.0,
                });
        },
    );

    assert!(commands.iter().all(|command| {
        !matches!(
            command,
            TransportCommand::RequestKeyframe { reason, .. } if reason == "displaySupplyCritical"
        )
    }));
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(
            ledger.input_signal,
            "transportSampleLoss:transportSampleLoss"
        );
    });
}

#[test]
fn recovery_integration_transport_sample_loss_overrides_same_tick_local_transport_await() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

    let commands = harness.apply(
        3_180.0,
        ConnectionLifecycleStateFact::Connected,
        "transportSampleLoss",
        83,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 29;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Disconnected;
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 12;
            stats.latest_video_host_present_time_ms = Some(now_ms - 260.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 220.0);
            stats.video_renderer_stalled = Some(false);
            stats.video_decoder_stalled = Some(false);
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Disconnected,
                video_bytes_total: 214_000,
                video_packet_count_total: 1_710,
                audio_bytes_total: 52_000,
                observed_at_ms: now_ms - 4.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 11,
                    source_event: "frame-await-recovery-keyframe".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                        observed_at_ms: now_ms - 3.0,
                    },
                    observed_at_ms: now_ms - 3.0,
                });
        },
    );

    assert!(commands.iter().all(|command| {
        !matches!(
            command,
            TransportCommand::RequestKeyframe { reason, .. }
                if reason == "transportAwaitRecoveryKeyframe"
        )
    }));
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(
            ledger.input_signal,
            "transportSampleLoss:transportSampleLoss"
        );
    });
}

#[test]
fn recovery_integration_transport_recovered_late_overrides_same_tick_local_display_recovery() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

    let commands = harness.apply(
        3_240.0,
        ConnectionLifecycleStateFact::Connected,
        "transportRecoveredLate",
        84,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 30;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Disconnected;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 300;
            stats.latest_video_host_present_time_ms = Some(now_ms - 1_160.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 700.0);
            stats.video_renderer_stalled = Some(true);
            stats.video_decoder_stalled = Some(false);
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Disconnected,
                video_bytes_total: 206_000,
                video_packet_count_total: 1_600,
                audio_bytes_total: 48_000,
                observed_at_ms: now_ms - 4.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 12,
                    source_event: "frame-observed".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        observed_at_ms: now_ms - 4.0,
                    },
                    observed_at_ms: now_ms - 4.0,
                });
        },
    );

    assert!(commands.iter().all(|command| {
        !matches!(
            command,
            TransportCommand::RequestKeyframe { reason, .. } if reason == "displaySupplyCritical"
        )
    }));
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(
            ledger.input_signal,
            "transportRecoveredLate:transportRecoveredLate"
        );
    });
}

#[test]
fn recovery_integration_transport_recovered_late_overrides_same_tick_local_transport_await() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

    let commands = harness.apply(
        3_300.0,
        ConnectionLifecycleStateFact::Connected,
        "transportRecoveredLate",
        85,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 31;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Disconnected;
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 16;
            stats.latest_video_host_present_time_ms = Some(now_ms - 240.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 210.0);
            stats.video_renderer_stalled = Some(false);
            stats.video_decoder_stalled = Some(false);
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Disconnected,
                video_bytes_total: 218_000,
                video_packet_count_total: 1_730,
                audio_bytes_total: 54_000,
                observed_at_ms: now_ms - 4.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 13,
                    source_event: "frame-await-recovery-keyframe".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                        observed_at_ms: now_ms - 3.0,
                    },
                    observed_at_ms: now_ms - 3.0,
                });
        },
    );

    assert!(commands.iter().all(|command| {
        !matches!(
            command,
            TransportCommand::RequestKeyframe { reason, .. }
                if reason == "transportAwaitRecoveryKeyframe"
        )
    }));
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(
            ledger.input_signal,
            "transportRecoveredLate:transportRecoveredLate"
        );
    });
}

#[test]
fn active_adapter_idle_timeout_is_suppressed_when_render_output_is_still_fresh() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.session_phase = Some("steady".to_string());
        stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
        stats.transport_recovery_epoch = 4;
        stats.latest_video_host_present_time_ms = Some(930.0);
        stats.latest_video_decode_ok_time_ms = Some(948.0);
        stats.video_decoder_stalled = Some(false);
        stats.video_renderer_stalled = Some(false);
        stats.video_anchor_clean_epoch = Some(4);
        stats.video_anchor_clean_observed_at_ms = Some(940.0);
        stats.video_anchor_clean_source_event = Some("chain-clean-keyframe-submitted".to_string());
    }
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
    connection.last_observed_at_ms = Some(1_000.0);
    let snapshot = TransportSnapshot::new(
        1,
        1_000.0,
        connection,
        MediaProjection {
            frame_count: 24,
            ..MediaProjection::default()
        },
        RecoveryProjection {
            latest_diagnosis_label: Some("adapterIdleTimeout".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(1_000.0),
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );

    let commands = policy.on_snapshot(&snapshot);
    assert!(commands.is_empty(), "unexpected commands: {commands:?}");
    let stats = runtime_stats.lock().expect("runtime stats lock");
    let ledger = stats
        .latest_recovery_decision_ledger
        .as_ref()
        .expect("recovery decision ledger");
    assert_eq!(ledger.input_signal, "none");
    assert_eq!(ledger.gate_result, "no-signal");
    assert_eq!(ledger.action_selected, "none");
}

#[test]
fn active_adapter_idle_timeout_still_reaches_recovery_path() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
    }
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
    connection.last_observed_at_ms = Some(1_000.0);
    let snapshot = TransportSnapshot::new(
        1,
        1_000.0,
        connection,
        MediaProjection::default(),
        RecoveryProjection {
            latest_diagnosis_label: Some("adapterIdleTimeout".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(1_000.0),
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );

    let commands = policy.on_snapshot(&snapshot);
    assert!(commands
        .iter()
        .any(|command| { matches!(command, TransportCommand::RequestDecoderReset { .. }) }));
    let stats = runtime_stats.lock().expect("runtime stats lock");
    let ledger = stats
        .latest_recovery_decision_ledger
        .as_ref()
        .expect("recovery decision ledger");
    assert_eq!(ledger.input_signal, "adapterIdleTimeout:adapterIdleTimeout");
    assert_eq!(ledger.gate_result, "pass");
}

#[test]
fn realtime_adapter_idle_timeout_is_absorbed_when_render_output_is_fresh() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.session_phase = Some("steady".to_string());
        stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
        stats.transport_recovery_epoch = 7;
        stats.host_no_pending_pressure_level = Some("normal".to_string());
        stats.host_no_pending_streak = 0;
        stats.latest_video_host_present_time_ms = Some(996.0);
        stats.latest_video_decode_ok_time_ms = Some(997.0);
        stats.video_decoder_stalled = Some(false);
        stats.video_renderer_stalled = Some(false);
        stats.video_anchor_clean_epoch = Some(7);
        stats.video_anchor_clean_observed_at_ms = Some(995.0);
        stats.video_anchor_clean_source_event = Some("chain-clean-keyframe-submitted".to_string());
        stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
            state: "remoteTrackAttached".to_string(),
            video_width: Some(1920),
            video_height: Some(1080),
            mime_type: Some("video/H264".to_string()),
            transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
            video_bytes_total: 256_000,
            video_packet_count_total: 3_200,
            audio_bytes_total: 64_000,
            observed_at_ms: 998.0,
        });
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 1,
            source_event: "frame-observed".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "healthy".to_string(),
                reason: None,
                observed_at_ms: 998.0,
            },
            observed_at_ms: 998.0,
        });
    }
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
    connection.control_channel_open = true;
    connection.latest_transport_path = Some("Direct".to_string());
    connection.latest_rtt_ms = Some(18.0);
    connection.last_observed_at_ms = Some(1_000.0);

    let healthy_snapshot = TransportSnapshot::new(
        1,
        1_000.0,
        connection.clone(),
        MediaProjection {
            frame_count: 48,
            ..MediaProjection::default()
        },
        RecoveryProjection {
            latest_diagnosis_label: Some("none".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(999.0),
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let _ = policy.on_snapshot(&healthy_snapshot);

    let snapshot = TransportSnapshot::new(
        2,
        1_000.0,
        connection,
        MediaProjection {
            frame_count: 49,
            ..MediaProjection::default()
        },
        RecoveryProjection {
            latest_diagnosis_label: Some("adapterIdleTimeout".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(1_000.0),
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );

    let commands = policy.on_snapshot(&snapshot);
    assert!(commands.is_empty(), "unexpected commands: {commands:?}");
    let stats = runtime_stats.lock().expect("runtime stats lock");
    let ledger = stats
        .latest_recovery_decision_ledger
        .as_ref()
        .expect("recovery decision ledger");
    assert_eq!(ledger.gate_result, "no-signal");
    assert_eq!(ledger.action_selected, "none");
}

#[test]
fn recovery_integration_local_ingress_drop_stays_no_signal_under_healthy_baseline() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

    let baseline = harness.apply(
        1_200.0,
        ConnectionLifecycleStateFact::Connected,
        "none",
        40,
        |stats| {
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 9;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(1_190.0);
            stats.latest_video_decode_ok_time_ms = Some(1_194.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = Some(9);
            stats.video_anchor_clean_observed_at_ms = Some(1_196.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-keyframe-submitted".to_string());
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 256_000,
                video_packet_count_total: 2_400,
                audio_bytes_total: 64_000,
                observed_at_ms: 1_198.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-observed".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        observed_at_ms: 1_198.0,
                    },
                    observed_at_ms: 1_198.0,
                });
        },
    );
    assert!(
        baseline.is_empty(),
        "unexpected baseline commands: {baseline:?}"
    );

    let commands = harness.apply(
        1_230.0,
        ConnectionLifecycleStateFact::Connected,
        "none",
        41,
        |stats| {
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(1_220.0);
            stats.latest_video_decode_ok_time_ms = Some(1_224.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = Some(9);
            stats.video_anchor_clean_observed_at_ms = Some(1_226.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-keyframe-submitted".to_string());
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total = 288_000;
                track.video_packet_count_total = 2_640;
                track.observed_at_ms = 1_228.0;
            }
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.source_event = "frame-observed".to_string();
                timeline.chain.state = "healthy".to_string();
                timeline.chain.reason = None;
                timeline.chain.observed_at_ms = 1_228.0;
                timeline.observed_at_ms = 1_228.0;
            }
            stats.latest_video_frame_drop = Some(crate::XbxEngineVideoFrameDropObservation {
                observation_id: 77,
                reason: "localBackpressureRepairOverflow".to_string(),
                stage: Some("ingress".to_string()),
                action: Some("drop".to_string()),
                detail: Some("repairQueueDropOldest:priority-repair".to_string()),
                frame_rtp_timestamp: Some(90_000),
                frame_seq: None,
                frame_recovery_disposition: None,
                frame_unrecoverable_reason: Some("localBackpressure".to_string()),
                frame_budget: None,
                observed_at_ms: 1_229.0,
                width: 0,
                height: 0,
                is_keyframe: false,
                queue_depth: 6,
            });
        },
    );

    assert!(commands.is_empty(), "unexpected commands: {commands:?}");
    assert!(commands
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })));
    harness.with_stats(|stats| {
        assert_eq!(
            stats
                .latest_video_frame_drop
                .as_ref()
                .map(|drop| drop.reason.as_str()),
            Some("localBackpressureRepairOverflow")
        );
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(ledger.input_signal, "none");
        assert_eq!(ledger.gate_result, "no-signal");
        assert_eq!(ledger.action_selected, "none");
    });
}

#[test]
fn recovery_integration_local_ingress_and_stale_idle_stay_no_signal_under_healthy_baseline() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

    let baseline = harness.apply(
        1_300.0,
        ConnectionLifecycleStateFact::Connected,
        "none",
        52,
        |stats| {
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 11;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(1_292.0);
            stats.latest_video_decode_ok_time_ms = Some(1_296.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = Some(11);
            stats.video_anchor_clean_observed_at_ms = Some(1_297.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-keyframe-submitted".to_string());
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 320_000,
                video_packet_count_total: 3_040,
                audio_bytes_total: 80_000,
                observed_at_ms: 1_298.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-observed".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        observed_at_ms: 1_298.0,
                    },
                    observed_at_ms: 1_298.0,
                });
        },
    );
    assert!(
        baseline.is_empty(),
        "unexpected baseline commands: {baseline:?}"
    );

    let commands = harness.apply(
        1_332.0,
        ConnectionLifecycleStateFact::Connected,
        "adapterIdleTimeout",
        53,
        |stats| {
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(1_324.0);
            stats.latest_video_decode_ok_time_ms = Some(1_328.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = Some(11);
            stats.video_anchor_clean_observed_at_ms = Some(1_327.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-keyframe-submitted".to_string());
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total = 352_000;
                track.video_packet_count_total = 3_280;
                track.observed_at_ms = 1_330.0;
            }
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.source_event = "frame-observed".to_string();
                timeline.chain.state = "healthy".to_string();
                timeline.chain.reason = None;
                timeline.chain.observed_at_ms = 1_330.0;
                timeline.observed_at_ms = 1_330.0;
            }
            stats.latest_video_frame_drop = Some(crate::XbxEngineVideoFrameDropObservation {
                observation_id: 117,
                reason: "localBackpressureRepairOverflow".to_string(),
                stage: Some("ingress".to_string()),
                action: Some("drop".to_string()),
                detail: Some("repairQueueDropOldest:priority-repair".to_string()),
                frame_rtp_timestamp: Some(91_000),
                frame_seq: None,
                frame_recovery_disposition: None,
                frame_unrecoverable_reason: Some("localBackpressure".to_string()),
                frame_budget: None,
                observed_at_ms: 1_331.0,
                width: 0,
                height: 0,
                is_keyframe: false,
                queue_depth: 6,
            });
        },
    );

    assert!(
        commands.is_empty(),
        "unexpected stale idle + local ingress commands: {commands:?}"
    );
    assert!(commands
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })));
    harness.with_stats(|stats| {
        assert_eq!(
            stats
                .latest_video_frame_drop
                .as_ref()
                .map(|drop| drop.reason.as_str()),
            Some("localBackpressureRepairOverflow")
        );
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(ledger.input_signal, "none");
        assert_eq!(ledger.gate_result, "no-signal");
        assert_eq!(ledger.action_selected, "none");
    });
}

#[test]
fn connected_track_attached_without_host_feedback_does_not_escalate_adapter_idle_timeout_during_priming_window(
) {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud);
        stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
        stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
            state: "remoteTrackAttached".to_string(),
            video_width: Some(1920),
            video_height: Some(1080),
            mime_type: Some("video/H264".to_string()),
            transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
            video_bytes_total: 64_000,
            video_packet_count_total: 800,
            audio_bytes_total: 16_000,
            observed_at_ms: 1_000.0,
        });
        stats.video_decoder_stalled = Some(false);
        stats.video_renderer_stalled = Some(false);
    }
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
    connection.control_channel_open = true;
    connection.latest_transport_path = Some("Direct".to_string());
    connection.latest_rtt_ms = Some(24.0);
    connection.last_observed_at_ms = Some(5_000.0);
    let snapshot = TransportSnapshot::new(
        1,
        5_000.0,
        connection,
        MediaProjection::default(),
        RecoveryProjection {
            latest_diagnosis_label: Some("adapterIdleTimeout".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(5_000.0),
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );

    let commands = policy.on_snapshot(&snapshot);
    assert!(commands.is_empty(), "unexpected commands: {commands:?}");
    let stats = runtime_stats.lock().expect("runtime stats lock");
    let ledger = stats
        .latest_recovery_decision_ledger
        .as_ref()
        .expect("recovery decision ledger");
    assert_eq!(ledger.gate_result, "no-signal");
    assert_eq!(ledger.action_selected, "none");
}

#[test]
fn connected_track_attached_without_host_feedback_eventually_escalates_after_priming_window_expires(
) {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud);
        stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
        stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
            state: "remoteTrackAttached".to_string(),
            video_width: Some(1920),
            video_height: Some(1080),
            mime_type: Some("video/H264".to_string()),
            transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
            video_bytes_total: 96_000,
            video_packet_count_total: 1_200,
            audio_bytes_total: 24_000,
            observed_at_ms: 1_000.0,
        });
        stats.video_decoder_stalled = Some(false);
        stats.video_renderer_stalled = Some(false);
    }
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
    connection.control_channel_open = true;
    connection.latest_transport_path = Some("Direct".to_string());
    connection.latest_rtt_ms = Some(24.0);
    connection.last_observed_at_ms = Some(37_000.0);
    let snapshot = TransportSnapshot::new(
        1,
        37_000.0,
        connection,
        MediaProjection::default(),
        RecoveryProjection {
            latest_diagnosis_label: Some("adapterIdleTimeout".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(37_000.0),
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );

    let commands = policy.on_snapshot(&snapshot);
    assert!(
        !commands.is_empty(),
        "priming bad window expired should enter recovery path"
    );
    let stats = runtime_stats.lock().expect("runtime stats lock");
    let ledger = stats
        .latest_recovery_decision_ledger
        .as_ref()
        .expect("recovery decision ledger");
    assert_eq!(ledger.input_signal, "adapterIdleTimeout:adapterIdleTimeout");
    assert_eq!(ledger.gate_result, "pass");
    assert_ne!(ledger.action_selected, "none");
}

#[test]
fn connected_track_attached_without_first_frame_feedback_does_not_escalate_transport_await_during_priming_window(
) {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.session_phase = Some("priming".to_string());
        stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud);
        stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
        stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
            state: "remoteTrackAttached".to_string(),
            video_width: Some(2560),
            video_height: Some(1440),
            mime_type: Some("video/H264".to_string()),
            transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
            video_bytes_total: 20_662,
            video_packet_count_total: 23,
            audio_bytes_total: 989,
            observed_at_ms: 1_000.0,
        });
        stats.video_decoder_stalled = Some(false);
        stats.video_renderer_stalled = Some(false);
    }
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
    connection.control_channel_open = true;
    connection.latest_transport_path = Some("Direct".to_string());
    connection.latest_rtt_ms = Some(24.0);
    connection.last_observed_at_ms = Some(1_100.0);
    let snapshot = TransportSnapshot::new(
        1,
        1_100.0,
        connection,
        MediaProjection::default(),
        RecoveryProjection {
            latest_diagnosis_label: Some("transportAwaitRecoveryKeyframe".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(1_100.0),
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );

    let commands = policy.on_snapshot(&snapshot);
    assert!(commands.is_empty(), "unexpected commands: {commands:?}");
    let stats = runtime_stats.lock().expect("runtime stats lock");
    let ledger = stats
        .latest_recovery_decision_ledger
        .as_ref()
        .expect("recovery decision ledger");
    assert_eq!(ledger.gate_result, "no-signal");
    assert_eq!(ledger.action_selected, "none");
}

#[test]
fn connected_track_attached_without_first_frame_feedback_does_not_escalate_display_supply_degraded_during_priming_window(
) {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

    let commands = harness.apply(
        1_100.0,
        ConnectionLifecycleStateFact::Connected,
        "displaySupplyDegraded",
        0,
        |stats| {
            stats.session_phase = Some("priming".to_string());
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 132;
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(2560),
                video_height: Some(1440),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 20_662,
                video_packet_count_total: 23,
                audio_bytes_total: 989,
                observed_at_ms: 1_000.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-complete-candidate".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        observed_at_ms: 1_098.0,
                    },
                    observed_at_ms: 1_098.0,
                });
        },
    );

    assert!(commands.is_empty(), "unexpected commands: {commands:?}");
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(ledger.gate_result, "no-signal");
        assert_eq!(ledger.action_selected, "none");
    });
}

#[test]
fn connected_track_attached_without_first_frame_feedback_eventually_escalates_transport_await_after_priming_window(
) {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.session_phase = Some("priming".to_string());
        stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud);
        stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
        stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
            state: "remoteTrackAttached".to_string(),
            video_width: Some(2560),
            video_height: Some(1440),
            mime_type: Some("video/H264".to_string()),
            transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
            video_bytes_total: 20_662,
            video_packet_count_total: 23,
            audio_bytes_total: 989,
            observed_at_ms: 1_000.0,
        });
        stats.video_decoder_stalled = Some(false);
        stats.video_renderer_stalled = Some(false);
    }
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
    connection.control_channel_open = true;
    connection.latest_transport_path = Some("Direct".to_string());
    connection.latest_rtt_ms = Some(24.0);
    connection.last_observed_at_ms = Some(37_000.0);
    let snapshot = TransportSnapshot::new(
        1,
        37_000.0,
        connection,
        MediaProjection::default(),
        RecoveryProjection {
            latest_diagnosis_label: Some("transportAwaitRecoveryKeyframe".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(37_000.0),
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );

    let commands = policy.on_snapshot(&snapshot);
    assert!(
        !commands.is_empty(),
        "priming wait-keyframe window expired should enter recovery path"
    );
    let stats = runtime_stats.lock().expect("runtime stats lock");
    let ledger = stats
        .latest_recovery_decision_ledger
        .as_ref()
        .expect("recovery decision ledger");
    assert_eq!(
        ledger.input_signal,
        "transportAwaitRecoveryKeyframe:transportAwaitRecoveryKeyframe"
    );
    assert_eq!(ledger.gate_result, "pass");
    assert_ne!(ledger.action_selected, "none");
}

#[test]
fn trace_1775292592042_short_idle_timeout_burst_is_absorbed_while_present_progress_continues() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.session_phase = Some("steady".to_string());
        stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud);
        stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
        stats.transport_recovery_epoch = 11;
        stats.host_no_pending_pressure_level = Some("high".to_string());
        stats.host_no_pending_streak = 3;
        stats.latest_video_host_present_time_ms = Some(1_000.0);
        stats.latest_video_decode_ok_time_ms = Some(1_001.0);
        stats.video_decoder_stalled = Some(false);
        stats.video_renderer_stalled = Some(false);
        stats.video_anchor_clean_epoch = Some(11);
        stats.video_anchor_clean_observed_at_ms = Some(999.0);
        stats.video_anchor_clean_source_event = Some("chain-clean-keyframe-submitted".to_string());
        stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
            state: "remoteTrackAttached".to_string(),
            video_width: Some(1920),
            video_height: Some(1080),
            mime_type: Some("video/H264".to_string()),
            transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
            video_bytes_total: 320_000,
            video_packet_count_total: 3_600,
            audio_bytes_total: 48_000,
            observed_at_ms: 1_002.0,
        });
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 1,
            source_event: "frame-observed".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "healthy".to_string(),
                reason: None,
                observed_at_ms: 1_002.0,
            },
            observed_at_ms: 1_002.0,
        });
    }
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
    connection.control_channel_open = true;
    connection.latest_transport_path = Some("Direct".to_string());
    connection.latest_rtt_ms = Some(21.0);
    connection.last_observed_at_ms = Some(1_008.0);

    let first = TransportSnapshot::new(
        1,
        1_008.0,
        connection.clone(),
        MediaProjection {
            frame_count: 120,
            ..MediaProjection::default()
        },
        RecoveryProjection {
            latest_diagnosis_label: Some("adapterIdleTimeout".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(1_008.0),
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let first_commands = policy.on_snapshot(&first);
    assert!(
        first_commands.is_empty(),
        "trace 1775292592042 的轻抖动首拍不应升级: {first_commands:?}"
    );

    if let Ok(mut stats) = runtime_stats.lock() {
        stats.host_no_pending_streak = 5;
        stats.latest_video_host_present_time_ms = Some(1_036.0);
        stats.latest_video_decode_ok_time_ms = Some(1_038.0);
        if let Some(track) = stats.latest_video_track_status.as_mut() {
            track.video_bytes_total += 24_000;
            track.video_packet_count_total += 220;
            track.observed_at_ms = 1_039.0;
        }
        if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
            timeline.observation_id += 1;
            timeline.observed_at_ms = 1_039.0;
            timeline.chain.observed_at_ms = 1_039.0;
        }
    }
    connection.last_observed_at_ms = Some(1_040.0);
    let second = TransportSnapshot::new(
        2,
        1_040.0,
        connection,
        MediaProjection {
            frame_count: 121,
            ..MediaProjection::default()
        },
        RecoveryProjection {
            latest_diagnosis_label: Some("adapterIdleTimeout".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(1_040.0),
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let second_commands = policy.on_snapshot(&second);
    assert!(
            second_commands.is_empty(),
            "trace 1775292592042 的短促 idle burst 在 present/decode 持续前进时应继续吸收: {second_commands:?}"
        );

    let stats = runtime_stats.lock().expect("runtime stats lock");
    let ledger = stats
        .latest_recovery_decision_ledger
        .as_ref()
        .expect("recovery decision ledger");
    assert_eq!(ledger.input_signal, "none");
    assert_eq!(ledger.gate_result, "no-signal");
    assert_eq!(ledger.action_selected, "none");
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
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        stats.host_no_pending_pressure_level = Some("normal".to_string());
        stats.host_no_pending_streak = 0;
        stats.latest_video_host_present_time_ms = Some(now_ms - 12.0);
        stats.latest_video_decode_ok_time_ms = Some(now_ms - 10.0);
        stats.video_anchor_clean_epoch = Some(0);
        stats.video_anchor_clean_observed_at_ms = Some(now_ms - 8.0);
        stats.video_anchor_clean_source_event = Some("chain-clean-keyframe-submitted".to_string());
        stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
            state: "remoteTrackAttached".to_string(),
            video_width: Some(1920),
            video_height: Some(1080),
            mime_type: None,
            transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
            video_bytes_total: 64_000,
            video_packet_count_total: 1_200,
            audio_bytes_total: 32_000,
            observed_at_ms: now_ms,
        });
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 1,
            source_event: "frame-observed".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "healthy".to_string(),
                reason: None,
                observed_at_ms: now_ms - 6.0,
            },
            observed_at_ms: now_ms - 6.0,
        });
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

#[test]
fn recovery_integration_home_clean_anchor_short_jitter_keeps_steady_serving() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Home));

    let baseline = harness.apply(
        2_000.0,
        ConnectionLifecycleStateFact::Connected,
        "none",
        180,
        |stats| {
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 21;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(1_992.0);
            stats.latest_video_decode_ok_time_ms = Some(1_996.0);
            stats.latest_video_packet_arrival_time_ms = Some(1_997.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = Some(21);
            stats.video_anchor_clean_observed_at_ms = Some(1_998.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-keyframe-submitted".to_string());
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1280),
                video_height: Some(720),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 180_000,
                video_packet_count_total: 1_600,
                audio_bytes_total: 32_000,
                observed_at_ms: 1_998.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-observed".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        observed_at_ms: 1_998.0,
                    },
                    observed_at_ms: 1_998.0,
                });
        },
    );
    assert!(
        baseline.is_empty(),
        "unexpected baseline commands: {baseline:?}"
    );

    let jitter = harness.apply(
        2_030.0,
        ConnectionLifecycleStateFact::Connected,
        "adapterIdleTimeout",
        181,
        |stats| {
            stats.host_no_pending_pressure_level = Some("high".to_string());
            stats.host_no_pending_streak = 2;
            stats.latest_video_host_present_time_ms = Some(2_022.0);
            stats.latest_video_decode_ok_time_ms = Some(2_025.0);
            stats.latest_video_packet_arrival_time_ms = Some(2_026.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = Some(21);
            stats.video_anchor_clean_observed_at_ms = Some(2_024.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-keyframe-submitted".to_string());
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total += 18_000;
                track.video_packet_count_total += 120;
                track.observed_at_ms = 2_028.0;
            }
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.observation_id += 1;
                timeline.source_event = "frame-observed".to_string();
                timeline.chain.state = "healthy".to_string();
                timeline.chain.reason = None;
                timeline.chain.observed_at_ms = 2_028.0;
                timeline.observed_at_ms = 2_028.0;
            }
        },
    );

    assert!(
        jitter.is_empty(),
        "home clean-anchor short jitter should stay absorbed: {jitter:?}"
    );
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(ledger.input_signal, "none");
        assert_eq!(ledger.gate_result, "no-signal");
        assert_eq!(ledger.action_selected, "none");
    });
}

#[test]
fn recovery_integration_home_render_deadline_jitter_stays_local_display_path() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Home));

    let commands = harness.apply(
        2_400.0,
        ConnectionLifecycleStateFact::Connected,
        "displaySupplyCritical",
        96,
        |stats| {
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 7;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 5;
            stats.latest_video_host_present_time_ms = Some(2_080.0);
            stats.latest_video_decode_ok_time_ms = Some(2_388.0);
            stats.latest_video_packet_arrival_time_ms = Some(2_392.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(true);
            stats.video_present_submit_count_total = 300;
            stats.video_present_drop_count_total = 26;
            stats.video_pacer_submit_count_total = 320;
            stats.video_pacer_drop_count_total = 12;
            stats.video_renderer_submit_count_total = 300;
            stats.video_renderer_drop_count_total = 18;
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1280),
                video_height: Some(720),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 512_000,
                video_packet_count_total: 4_000,
                audio_bytes_total: 96_000,
                observed_at_ms: 2_395.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-observed".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        observed_at_ms: 2_395.0,
                    },
                    observed_at_ms: 2_395.0,
                });
        },
    );

    assert!(commands.iter().any(|command| {
        matches!(
            command,
            TransportCommand::RequestKeyframe { reason, .. } if reason == "displaySupplyCritical"
        )
    }));
    assert!(commands
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })));
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(
            ledger.input_signal,
            "displaySupplyCritical:displaySupplyCritical"
        );
        assert_eq!(ledger.gate_result, "pass");
        assert_ne!(ledger.action_selected, "requestReconnectCandidate");
    });
}

#[test]
fn cloud_startup_transport_progress_does_not_reconnect_before_first_frame() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud);
        stats.session_phase = Some("startup".to_string());
        stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connecting;
        stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
            state: "remoteTrackAttached".to_string(),
            video_width: Some(1920),
            video_height: Some(1080),
            mime_type: Some("video/H264".to_string()),
            transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connecting,
            video_bytes_total: 48_000,
            video_packet_count_total: 320,
            audio_bytes_total: 8_000,
            observed_at_ms: 9_800.0,
        });
        stats.video_decoder_stalled = Some(false);
        stats.video_renderer_stalled = Some(false);
    }
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Connecting;
    connection.control_channel_open = true;
    connection.latest_transport_path = Some("Direct".to_string());
    connection.latest_rtt_ms = Some(185.0);
    connection.last_observed_at_ms = Some(10_000.0);

    let first = TransportSnapshot::new(
        1,
        10_000.0,
        connection.clone(),
        MediaProjection::default(),
        RecoveryProjection {
            latest_diagnosis_label: Some("none".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(10_000.0),
        },
        BweProjection {
            latest_rtt_ms: Some(185.0),
            latest_loss_ratio_1s: Some(0.0),
            latest_actual_video_bitrate_kbps: Some(8_500.0),
            latest_observed_remb_kbps: Some(12_000),
            latest_transport_path: Some("Direct".to_string()),
            latest_sample_tick_ms: Some(10_000.0),
            target_remb_kbps: Some(12_000),
            last_observed_at_ms: Some(10_000.0),
        },
        DiagnosticsProjection::default(),
    );
    let first_commands = policy.on_snapshot(&first);
    assert!(first_commands
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })));

    if let Ok(mut stats) = runtime_stats.lock() {
        if let Some(track) = stats.latest_video_track_status.as_mut() {
            track.video_bytes_total += 12_000;
            track.video_packet_count_total += 80;
            track.observed_at_ms = 21_500.0;
        }
    }
    connection.last_observed_at_ms = Some(21_600.0);
    let second = TransportSnapshot::new(
        2,
        21_600.0,
        connection,
        MediaProjection::default(),
        RecoveryProjection {
            latest_diagnosis_label: Some("none".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(21_600.0),
        },
        BweProjection {
            latest_rtt_ms: Some(190.0),
            latest_loss_ratio_1s: Some(0.0),
            latest_actual_video_bitrate_kbps: Some(8_200.0),
            latest_observed_remb_kbps: Some(12_000),
            latest_transport_path: Some("Direct".to_string()),
            latest_sample_tick_ms: Some(21_600.0),
            target_remb_kbps: Some(12_000),
            last_observed_at_ms: Some(21_600.0),
        },
        DiagnosticsProjection::default(),
    );
    let second_commands = policy.on_snapshot(&second);
    assert!(
        second_commands
            .iter()
            .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })),
        "cloud startup with transport progress should not reconnect before first frame: {second_commands:?}"
    );
}

#[test]
fn recovery_integration_cloud_reconnect_then_clean_recovery_exit_does_not_reenter_storm() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

    let reconnect = harness.apply(
        4_000.0,
        ConnectionLifecycleStateFact::Disconnected,
        "rtcControlChannelClosed",
        64,
        |stats| {
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 32;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Disconnected;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 320;
            stats.latest_video_host_present_time_ms = Some(2_400.0);
            stats.latest_video_decode_ok_time_ms = Some(3_100.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(true);
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Disconnected,
                video_bytes_total: 260_000,
                video_packet_count_total: 2_100,
                audio_bytes_total: 48_000,
                observed_at_ms: 3_995.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-await-recovery-keyframe".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                        observed_at_ms: 3_995.0,
                    },
                    observed_at_ms: 3_995.0,
                });
        },
    );
    assert!(reconnect
        .iter()
        .any(|command| { matches!(command, TransportCommand::RequestReconnectCandidate { .. }) }));

    let recovered = harness.apply(
        4_220.0,
        ConnectionLifecycleStateFact::Connected,
        "none",
        82,
        |stats| {
            stats.transport_recovery_epoch = 33;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(4_210.0);
            stats.latest_video_decode_ok_time_ms = Some(4_214.0);
            stats.latest_video_packet_arrival_time_ms = Some(4_216.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = Some(33);
            stats.video_anchor_clean_observed_at_ms = Some(4_218.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-keyframe-submitted".to_string());
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
                track.video_bytes_total += 42_000;
                track.video_packet_count_total += 240;
                track.observed_at_ms = 4_218.0;
            }
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.observation_id += 1;
                timeline.source_event = "frame-observed".to_string();
                timeline.chain.state = "healthy".to_string();
                timeline.chain.reason = None;
                timeline.chain.observed_at_ms = 4_218.0;
                timeline.observed_at_ms = 4_218.0;
            }
        },
    );
    assert!(
        recovered.is_empty(),
        "clean recovery exit should not emit extra commands: {recovered:?}"
    );

    let replay = harness.apply_with_recovery_observed_at(
        4_260.0,
        4_020.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryKeyframe",
        86,
        |stats| {
            stats.transport_recovery_epoch = 33;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(4_252.0);
            stats.latest_video_decode_ok_time_ms = Some(4_255.0);
            stats.latest_video_packet_arrival_time_ms = Some(4_256.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = Some(33);
            stats.video_anchor_clean_observed_at_ms = Some(4_258.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-keyframe-submitted".to_string());
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.observed_at_ms = 4_258.0;
            }
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.observation_id += 1;
                timeline.source_event = "frame-observed".to_string();
                timeline.chain.state = "healthy".to_string();
                timeline.chain.reason = None;
                timeline.chain.observed_at_ms = 4_258.0;
                timeline.observed_at_ms = 4_258.0;
            }
        },
    );
    assert!(
        replay.is_empty(),
        "stale transportAwait replay after clean recovery should stay absorbed: {replay:?}"
    );
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(ledger.input_signal, "none");
        assert_eq!(ledger.gate_result, "no-signal");
        assert_eq!(ledger.action_selected, "none");
    });
}

#[test]
fn recovery_integration_home_short_idle_blackhole_is_absorbed_until_progress_returns() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Home));

    let first = harness.apply(
        5_000.0,
        ConnectionLifecycleStateFact::Connected,
        "adapterIdleTimeout",
        144,
        |stats| {
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 12;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("high".to_string());
            stats.host_no_pending_streak = 3;
            stats.latest_video_host_present_time_ms = Some(4_992.0);
            stats.latest_video_decode_ok_time_ms = Some(4_995.0);
            stats.latest_video_packet_arrival_time_ms = Some(4_996.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = Some(12);
            stats.video_anchor_clean_observed_at_ms = Some(4_997.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-keyframe-submitted".to_string());
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1280),
                video_height: Some(720),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 240_000,
                video_packet_count_total: 1_920,
                audio_bytes_total: 32_000,
                observed_at_ms: 4_997.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-observed".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        observed_at_ms: 4_997.0,
                    },
                    observed_at_ms: 4_997.0,
                });
        },
    );
    assert!(
        first.is_empty(),
        "home short idle burst first hit should be absorbed: {first:?}"
    );

    let second = harness.apply(
        5_038.0,
        ConnectionLifecycleStateFact::Connected,
        "adapterIdleTimeout",
        145,
        |stats| {
            stats.host_no_pending_pressure_level = Some("high".to_string());
            stats.host_no_pending_streak = 5;
            stats.latest_video_host_present_time_ms = Some(5_032.0);
            stats.latest_video_decode_ok_time_ms = Some(5_034.0);
            stats.latest_video_packet_arrival_time_ms = Some(5_035.0);
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total += 14_000;
                track.video_packet_count_total += 96;
                track.observed_at_ms = 5_036.0;
            }
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.observation_id += 1;
                timeline.observed_at_ms = 5_036.0;
                timeline.chain.observed_at_ms = 5_036.0;
            }
        },
    );
    assert!(
        second.is_empty(),
        "home short idle burst should stay absorbed while progress resumes: {second:?}"
    );
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(ledger.input_signal, "none");
        assert_eq!(ledger.gate_result, "no-signal");
        assert_eq!(ledger.action_selected, "none");
    });
}

#[test]
fn recovery_integration_home_connected_ingress_without_output_progress_reenters_local_transport_recovery(
) {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Home));

    let baseline = harness.apply(
        5_400.0,
        ConnectionLifecycleStateFact::Connected,
        "adapterIdleTimeout",
        152,
        |stats| {
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 18;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("high".to_string());
            stats.host_no_pending_streak = 2;
            stats.latest_video_host_present_time_ms = Some(5_392.0);
            stats.latest_video_decode_ok_time_ms = Some(5_395.0);
            stats.latest_video_packet_arrival_time_ms = Some(5_396.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = Some(18);
            stats.video_anchor_clean_observed_at_ms = Some(5_397.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-keyframe-submitted".to_string());
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1280),
                video_height: Some(720),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 260_000,
                video_packet_count_total: 2_020,
                audio_bytes_total: 36_000,
                observed_at_ms: 5_397.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-observed".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        observed_at_ms: 5_397.0,
                    },
                    observed_at_ms: 5_397.0,
                });
        },
    );
    assert!(
        baseline.is_empty(),
        "unexpected baseline commands: {baseline:?}"
    );

    let stalled = harness.apply(
        6_120.0,
        ConnectionLifecycleStateFact::Connected,
        "adapterIdleTimeout",
        153,
        |stats| {
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 380;
            stats.latest_video_host_present_time_ms = Some(5_392.0);
            stats.latest_video_decode_ok_time_ms = Some(5_395.0);
            stats.latest_video_packet_arrival_time_ms = Some(6_116.0);
            stats.video_decoder_stalled = Some(true);
            stats.video_renderer_stalled = Some(true);
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total += 24_000;
                track.video_packet_count_total += 160;
                track.observed_at_ms = 6_118.0;
            }
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 2,
                    source_event: "frame-await-recovery-keyframe".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                        observed_at_ms: 6_118.0,
                    },
                    observed_at_ms: 6_118.0,
                });
        },
    );

    assert!(stalled.iter().any(|command| {
        matches!(
            command,
            TransportCommand::RequestKeyframe { reason, .. }
                if reason == "transportAwaitRecoveryKeyframe"
        )
    }));
    assert!(stalled
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })));
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(
            ledger.input_signal,
            "transportAwaitRecoveryKeyframe:transportAwaitRecoveryKeyframe"
        );
        assert_eq!(ledger.gate_result, "pass");
        assert_eq!(ledger.action_selected, "requestKeyframe");
    });
}

#[test]
fn recovery_integration_cloud_stale_transport_await_replay_reenters_local_recovery_when_output_stalls(
) {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

    let reconnect = harness.apply(
        8_000.0,
        ConnectionLifecycleStateFact::Disconnected,
        "rtcControlChannelClosed",
        120,
        |stats| {
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 52;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Disconnected;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 300;
            stats.latest_video_host_present_time_ms = Some(6_600.0);
            stats.latest_video_decode_ok_time_ms = Some(7_200.0);
            stats.latest_video_packet_arrival_time_ms = Some(7_250.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(true);
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Disconnected,
                video_bytes_total: 280_000,
                video_packet_count_total: 2_180,
                audio_bytes_total: 48_000,
                observed_at_ms: 7_995.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-await-recovery-keyframe".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                        observed_at_ms: 7_995.0,
                    },
                    observed_at_ms: 7_995.0,
                });
        },
    );
    assert!(reconnect
        .iter()
        .any(|command| matches!(command, TransportCommand::RequestReconnectCandidate { .. })));

    let replay = harness.apply_with_recovery_observed_at(
        8_740.0,
        8_020.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryKeyframe",
        124,
        |stats| {
            stats.transport_recovery_epoch = 53;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 420;
            stats.latest_video_host_present_time_ms = Some(8_110.0);
            stats.latest_video_decode_ok_time_ms = Some(8_130.0);
            stats.latest_video_packet_arrival_time_ms = Some(8_736.0);
            stats.video_decoder_stalled = Some(true);
            stats.video_renderer_stalled = Some(true);
            stats.video_anchor_clean_epoch = Some(53);
            stats.video_anchor_clean_observed_at_ms = Some(8_160.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-keyframe-submitted".to_string());
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
                track.video_bytes_total += 18_000;
                track.video_packet_count_total += 120;
                track.observed_at_ms = 8_738.0;
            }
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 2,
                    source_event: "frame-await-recovery-keyframe".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                        observed_at_ms: 8_738.0,
                    },
                    observed_at_ms: 8_738.0,
                });
        },
    );

    assert!(replay.iter().any(|command| {
        matches!(
            command,
            TransportCommand::RequestKeyframe { reason, .. }
                if reason == "transportAwaitRecoveryKeyframe"
        )
    }));
    assert!(replay
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })));
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(
            ledger.input_signal,
            "transportAwaitRecoveryKeyframe:transportAwaitRecoveryKeyframe"
        );
        assert_eq!(ledger.gate_result, "pass");
        assert_eq!(ledger.action_selected, "requestKeyframe");
    });
}

#[test]
fn recovery_integration_fresh_transport_await_absorption_expires_once_output_stalls() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

    let baseline = harness.apply(
        9_200.0,
        ConnectionLifecycleStateFact::Connected,
        "none",
        188,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 61;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(now_ms - 8.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 4.0);
            stats.latest_video_packet_arrival_time_ms = Some(now_ms - 2.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = Some(61);
            stats.video_anchor_clean_observed_at_ms = Some(now_ms - 2.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-keyframe-submitted".to_string());
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 520_000,
                video_packet_count_total: 4_320,
                audio_bytes_total: 82_000,
                observed_at_ms: now_ms - 2.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-observed".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        observed_at_ms: now_ms - 2.0,
                    },
                    observed_at_ms: now_ms - 2.0,
                });
        },
    );
    assert!(
        baseline.is_empty(),
        "unexpected baseline commands: {baseline:?}"
    );

    let absorbed = harness.apply_with_recovery_observed_at(
        9_240.0,
        9_240.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryKeyframe",
        189,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(now_ms - 8.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 4.0);
            stats.latest_video_packet_arrival_time_ms = Some(now_ms - 2.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total += 18_000;
                track.video_packet_count_total += 140;
                track.observed_at_ms = now_ms - 2.0;
            }
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.observation_id = 2;
                timeline.source_event = "frame-observed".to_string();
                timeline.chain.state = "healthy".to_string();
                timeline.chain.reason = None;
                timeline.chain.observed_at_ms = now_ms - 2.0;
                timeline.observed_at_ms = now_ms - 2.0;
            }
        },
    );
    assert!(
        absorbed.is_empty(),
        "fresh transportAwait with healthy output should stay absorbed: {absorbed:?}"
    );
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("absorbed decision ledger");
        assert_eq!(ledger.input_signal, "none");
        assert_eq!(ledger.gate_result, "no-signal");
        assert_eq!(ledger.action_selected, "none");
    });

    let stalled = harness.apply(
        9_620.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryKeyframe",
        193,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 360;
            stats.latest_video_host_present_time_ms = Some(now_ms - 1_320.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 820.0);
            stats.latest_video_packet_arrival_time_ms = Some(now_ms - 2.0);
            stats.video_decoder_stalled = Some(true);
            stats.video_renderer_stalled = Some(true);
            stats.video_anchor_clean_epoch = Some(61);
            stats.video_anchor_clean_observed_at_ms = Some(now_ms - 420.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-keyframe-submitted".to_string());
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total += 20_000;
                track.video_packet_count_total += 160;
                track.observed_at_ms = now_ms - 2.0;
            }
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 3,
                    source_event: "frame-await-recovery-keyframe".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                        observed_at_ms: now_ms - 2.0,
                    },
                    observed_at_ms: now_ms - 2.0,
                });
        },
    );

    assert!(stalled.iter().any(|command| {
        matches!(
            command,
            TransportCommand::RequestKeyframe { reason, .. }
                if reason == "transportAwaitRecoveryKeyframe"
        )
    }));
    assert!(stalled
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })));
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("stalled decision ledger");
        assert_eq!(
            ledger.input_signal,
            "transportAwaitRecoveryKeyframe:transportAwaitRecoveryKeyframe"
        );
        assert_eq!(ledger.gate_result, "pass");
        assert_eq!(ledger.action_selected, "requestKeyframe");
        assert_ne!(stats.video_owner_state.as_deref(), Some("stable-serving"));
    });
}

#[test]
fn recovery_integration_home_stale_transport_await_absorption_expires_once_output_stalls() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Home));

    let baseline = harness.apply(
        9_800.0,
        ConnectionLifecycleStateFact::Connected,
        "none",
        210,
        |stats| {
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 72;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(9_792.0);
            stats.latest_video_decode_ok_time_ms = Some(9_796.0);
            stats.latest_video_packet_arrival_time_ms = Some(9_797.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = Some(72);
            stats.video_anchor_clean_observed_at_ms = Some(9_798.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-keyframe-submitted".to_string());
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 560_000,
                video_packet_count_total: 4_860,
                audio_bytes_total: 96_000,
                observed_at_ms: 9_798.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-observed".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        observed_at_ms: 9_798.0,
                    },
                    observed_at_ms: 9_798.0,
                });
        },
    );
    assert!(
        baseline.is_empty(),
        "unexpected baseline commands: {baseline:?}"
    );

    let absorbed = harness.apply_with_recovery_observed_at(
        10_040.0,
        9_800.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryKeyframe",
        211,
        |stats| {
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(10_032.0);
            stats.latest_video_decode_ok_time_ms = Some(10_036.0);
            stats.latest_video_packet_arrival_time_ms = Some(10_038.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = Some(72);
            stats.video_anchor_clean_observed_at_ms = Some(10_038.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-keyframe-submitted".to_string());
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total += 20_000;
                track.video_packet_count_total += 150;
                track.observed_at_ms = 10_038.0;
            }
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.observation_id += 1;
                timeline.source_event = "frame-observed".to_string();
                timeline.chain.state = "healthy".to_string();
                timeline.chain.reason = None;
                timeline.chain.observed_at_ms = 10_038.0;
                timeline.observed_at_ms = 10_038.0;
            }
        },
    );
    assert!(
        absorbed.is_empty(),
        "stale transportAwait should stay absorbed while output is still fresh: {absorbed:?}"
    );

    let replay = harness.apply_with_recovery_observed_at(
        10_720.0,
        10_300.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryKeyframe",
        212,
        |stats| {
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 260;
            stats.latest_video_host_present_time_ms = Some(10_032.0);
            stats.latest_video_decode_ok_time_ms = Some(10_060.0);
            stats.latest_video_packet_arrival_time_ms = Some(10_718.0);
            stats.video_decoder_stalled = Some(true);
            stats.video_renderer_stalled = Some(true);
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total += 18_000;
                track.video_packet_count_total += 140;
                track.observed_at_ms = 10_718.0;
            }
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 3,
                    source_event: "frame-await-recovery-keyframe".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                        observed_at_ms: 10_718.0,
                    },
                    observed_at_ms: 10_718.0,
                });
        },
    );
    assert!(replay.iter().any(|command| {
        matches!(
            command,
            TransportCommand::RequestKeyframe { reason, .. }
                if reason == "transportAwaitRecoveryKeyframe"
        )
    }));
    assert!(replay
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })));
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(
            ledger.input_signal,
            "transportAwaitRecoveryKeyframe:transportAwaitRecoveryKeyframe"
        );
        assert_eq!(ledger.gate_result, "pass");
        assert_eq!(ledger.action_selected, "requestKeyframe");
    });
}

#[test]
fn recovery_integration_home_burst_input_rumble_display_pressure_then_stale_transport_await_replay_stays_absorbed(
) {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Home));

    let baseline = harness.apply(
        11_000.0,
        ConnectionLifecycleStateFact::Connected,
        "none",
        240,
        |stats| {
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 81;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(10_992.0);
            stats.latest_video_decode_ok_time_ms = Some(10_996.0);
            stats.latest_video_packet_arrival_time_ms = Some(10_997.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_present_submit_count_total = 956;
            stats.video_present_overwrite_count_total = 27;
            stats.video_present_drop_count_total = 2;
            stats.video_pacer_submit_count_total = 960;
            stats.video_pacer_drop_count_total = 1;
            stats.video_renderer_submit_count_total = 956;
            stats.video_renderer_drop_count_total = 0;
            stats.video_anchor_clean_epoch = Some(81);
            stats.video_anchor_clean_observed_at_ms = Some(10_998.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-keyframe-submitted".to_string());
            set_input_rumble_burst(stats, 1, 10_998.0, 0);
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 620_000,
                video_packet_count_total: 5_320,
                audio_bytes_total: 110_000,
                observed_at_ms: 10_998.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-observed".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        observed_at_ms: 10_998.0,
                    },
                    observed_at_ms: 10_998.0,
                });
        },
    );
    assert!(
        baseline.is_empty(),
        "unexpected baseline commands: {baseline:?}"
    );

    let burst = harness.apply(
        11_120.0,
        ConnectionLifecycleStateFact::Connected,
        "none",
        241,
        |stats| {
            stats.session_phase = Some("recovering".to_string());
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 97;
            stats.latest_video_host_present_time_ms = Some(10_306.0);
            stats.latest_video_decode_ok_time_ms = Some(11_116.0);
            stats.latest_video_packet_arrival_time_ms = Some(11_118.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_present_submit_count_total = 964;
            stats.video_present_overwrite_count_total = 27;
            stats.video_present_drop_count_total = 3;
            stats.video_pacer_submit_count_total = 969;
            stats.video_pacer_drop_count_total = 1;
            stats.video_renderer_submit_count_total = 964;
            stats.video_renderer_drop_count_total = 0;
            stats.latest_video_escalation_observation =
                Some(crate::XbxEngineVideoEscalationObservation {
                    observation_id: 284,
                    reason: "transportAwaitRecoveryKeyframe".to_string(),
                    action: "requestDecoderReset".to_string(),
                    recovery_stage: "rebuilding-supply".to_string(),
                    recovery_chain_value: "anchor".to_string(),
                    recovery_failure_cost: "high".to_string(),
                    recovery_window_source: "transport-await-window".to_string(),
                    observed_at_ms: 11_014.0,
                });
            stats.latest_keyframe_request_episode =
                Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                    episode_id: 284,
                    request_reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                    request_kind: Some("pli".to_string()),
                    status: "packet-seen".to_string(),
                    requested_at_ms: 11_010.0,
                    sent_at_ms: Some(11_010.0),
                    deadline_at_ms: Some(11_090.0),
                    first_keyframe_packet_at_ms: None,
                    first_keyframe_decoded_at_ms: None,
                    response_rtp_timestamp: None,
                    response_frame_seq: None,
                    response_verdict: None,
                });
            set_input_rumble_burst(stats, 2, 11_118.0, 32);
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total += 22_000;
                track.video_packet_count_total += 160;
                track.observed_at_ms = 11_118.0;
            }
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.observation_id += 1;
                timeline.source_event = "frame-complete-candidate".to_string();
                timeline.chain.state = "healthy".to_string();
                timeline.chain.reason = None;
                timeline.chain.observed_at_ms = 11_118.0;
                timeline.observed_at_ms = 11_118.0;
            }
        },
    );
    assert!(
        burst.is_empty(),
        "stale transport-await overlap should absorb display pressure burst locally: {burst:?}"
    );
    assert!(burst
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })));
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(
            ledger.input_signal,
            "adapterThinStream:displaySupplyDegraded"
        );
        assert_recovery_family_hold_semantics(
            ledger.gate_result.as_str(),
            ledger.action_selected.as_str(),
        );
    });

    let recovered = harness.apply(
        11_190.0,
        ConnectionLifecycleStateFact::Connected,
        "none",
        247,
        |stats| {
            stats.session_phase = Some("steady".to_string());
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(11_182.0);
            stats.latest_video_decode_ok_time_ms = Some(11_186.0);
            stats.latest_video_packet_arrival_time_ms = Some(11_188.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.latest_video_escalation_observation = None;
            stats.latest_keyframe_request_episode = None;
            stats.video_anchor_clean_epoch = Some(81);
            stats.video_anchor_clean_observed_at_ms = Some(11_189.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-keyframe-submitted".to_string());
            stats.video_present_submit_count_total = 972;
            stats.video_present_overwrite_count_total = 27;
            stats.video_present_drop_count_total = 3;
            stats.video_pacer_submit_count_total = 977;
            stats.video_pacer_drop_count_total = 1;
            stats.video_renderer_submit_count_total = 972;
            stats.video_renderer_drop_count_total = 0;
            set_input_rumble_burst(stats, 3, 11_188.0, 16);
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total += 24_000;
                track.video_packet_count_total += 170;
                track.observed_at_ms = 11_188.0;
            }
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.observation_id += 1;
                timeline.source_event = "frame-observed".to_string();
                timeline.chain.state = "healthy".to_string();
                timeline.chain.reason = None;
                timeline.chain.observed_at_ms = 11_188.0;
                timeline.observed_at_ms = 11_188.0;
            }
        },
    );
    assert!(
        recovered.is_empty(),
        "local display pressure should settle once output catches up: {recovered:?}"
    );

    let replay = harness.apply_with_recovery_observed_at(
        11_214.0,
        10_990.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryKeyframe",
        248,
        |stats| {
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(11_206.0);
            stats.latest_video_decode_ok_time_ms = Some(11_210.0);
            stats.latest_video_packet_arrival_time_ms = Some(11_212.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = Some(81);
            stats.video_anchor_clean_observed_at_ms = Some(11_212.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-keyframe-submitted".to_string());
            stats.video_present_submit_count_total = 974;
            stats.video_present_overwrite_count_total = 27;
            stats.video_present_drop_count_total = 3;
            stats.video_pacer_submit_count_total = 979;
            stats.video_pacer_drop_count_total = 1;
            stats.video_renderer_submit_count_total = 974;
            stats.video_renderer_drop_count_total = 0;
            set_input_rumble_burst(stats, 4, 11_212.0, 8);
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total += 12_000;
                track.video_packet_count_total += 88;
                track.observed_at_ms = 11_212.0;
            }
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.observation_id += 1;
                timeline.source_event = "frame-observed".to_string();
                timeline.chain.state = "healthy".to_string();
                timeline.chain.reason = None;
                timeline.chain.observed_at_ms = 11_212.0;
                timeline.observed_at_ms = 11_212.0;
            }
        },
    );
    assert!(
        replay.is_empty(),
        "stale transportAwait replay should stay absorbed after burst input pressure settles: {replay:?}"
    );
    harness.with_stats(|stats| {
        let latest_input = stats
            .latest_data_channel_message_catalog_observation
            .as_ref()
            .expect("latest input observation");
        assert_eq!(latest_input.channel, "input");
        assert_eq!(latest_input.direction, "inbound");
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(ledger.input_signal, "none");
        assert_eq!(ledger.gate_result, "no-signal");
        assert_eq!(ledger.action_selected, "none");
    });
}

#[test]
fn recovery_integration_home_burst_input_rumble_submit_gap_and_latest_slot_overwrite_stays_local() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Home));

    let commands = harness.apply(
        12_600.0,
        ConnectionLifecycleStateFact::Connected,
        "none",
        540,
        |stats| {
            stats.session_phase = Some("recovering".to_string());
            stats.transport_recovery_epoch = 91;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("high".to_string());
            stats.host_no_pending_streak = 96;
            stats.host_display_tick_epoch = 1_139;
            stats.video_present_epoch = 456;
            stats.host_cadence_phase = Some("starved".to_string());
            stats.latest_video_host_present_time_ms = Some(7_991.0);
            stats.latest_video_decode_ok_time_ms = Some(12_598.0);
            stats.latest_video_packet_arrival_time_ms = Some(12_599.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(true);
            stats.video_present_submit_count_total = 458;
            stats.video_present_overwrite_count_total = 1;
            stats.video_present_drop_count_total = 1;
            stats.video_pacer_submit_count_total = 533;
            stats.video_pacer_drop_count_total = 2;
            stats.video_renderer_submit_count_total = 531;
            stats.video_renderer_drop_count_total = 0;
            stats.latest_render_candidate_decision =
                Some(crate::XbxEnginePipelineCandidateDecisionObservation {
                    decision_id: 102,
                    state: "latest-overwrite".to_string(),
                    action: "replace".to_string(),
                    detail: "latestSlotOverwrite".to_string(),
                    frame_seq: Some(532),
                    observed_at_ms: 12_599.0,
                });
            set_input_rumble_burst(stats, 9, 12_599.0, 32);
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 17_361_017,
                video_packet_count_total: 15_170,
                audio_bytes_total: 204_946,
                observed_at_ms: 12_599.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1_196,
                    source_event: "frame-complete-candidate".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        observed_at_ms: 12_599.0,
                    },
                    observed_at_ms: 12_599.0,
                });
        },
    );
    assert!(
        commands
            .iter()
            .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })),
        "local burst-input stall should not escalate to reconnect: {commands:?}"
    );
    harness.with_stats(|stats| {
        let latest_input = stats
            .latest_data_channel_message_catalog_observation
            .as_ref()
            .expect("latest input observation");
        assert_eq!(latest_input.channel, "input");
        let render = stats
            .latest_render_candidate_decision
            .as_ref()
            .expect("latest render candidate");
        assert_eq!(render.detail, "latestSlotOverwrite");
    });
}

#[test]
fn recovery_integration_home_hard_disconnect_emits_connectivity_reconnect_without_local_absorption()
{
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Home));

    let baseline = harness.apply(
        11_900.0,
        ConnectionLifecycleStateFact::Connected,
        "none",
        240,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 51;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(now_ms - 10.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 6.0);
            stats.latest_video_packet_arrival_time_ms = Some(now_ms - 4.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = Some(51);
            stats.video_anchor_clean_observed_at_ms = Some(now_ms - 3.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-keyframe-submitted".to_string());
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1280),
                video_height: Some(720),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 320_000,
                video_packet_count_total: 2_400,
                audio_bytes_total: 64_000,
                observed_at_ms: now_ms - 2.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 21,
                    source_event: "frame-observed".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        observed_at_ms: now_ms - 2.0,
                    },
                    observed_at_ms: now_ms - 2.0,
                });
        },
    );
    assert!(
        baseline.is_empty(),
        "unexpected baseline commands: {baseline:?}"
    );

    let disconnect = harness.apply(
        12_020.0,
        ConnectionLifecycleStateFact::Disconnected,
        "rtcControlChannelClosed",
        240,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Disconnected;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 420;
            stats.latest_video_host_present_time_ms = Some(now_ms - 1_200.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 800.0);
            stats.latest_video_packet_arrival_time_ms = Some(now_ms - 700.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(true);
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.transport_state =
                    xbxengine_protocol::XbxEngineTransportStateDto::Disconnected;
                track.observed_at_ms = now_ms - 2.0;
            }
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.observation_id = 22;
                timeline.source_event = "frame-await-recovery-keyframe".to_string();
                timeline.chain.state = "recovering".to_string();
                timeline.chain.reason = Some("transportAwaitRecoveryKeyframe".to_string());
                timeline.chain.observed_at_ms = now_ms - 2.0;
                timeline.observed_at_ms = now_ms - 2.0;
            }
        },
    );

    assert!(disconnect.iter().any(|command| {
        matches!(
            command,
            TransportCommand::RequestReconnectCandidate {
                reason_domain,
                ..
            } if *reason_domain
                == crate::XbxEngineRecoveryReasonDomain::ConnectivityTransport
        )
    }));
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("home hard disconnect decision ledger");
        assert_eq!(
            ledger.input_signal,
            "rtcConnectionRecovering:rtcConnectionDisconnected"
        );
        assert_eq!(ledger.gate_result, "pass");
        assert_eq!(ledger.action_selected, "requestReconnectCandidate");
    });
}

#[test]
fn recovery_integration_cloud_media_loss_prefers_transport_await_before_reconnect() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

    let local_recover = harness.apply(
        6_000.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryKeyframe",
        220,
        |stats| {
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 18;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 12;
            stats.latest_video_host_present_time_ms = Some(5_760.0);
            stats.latest_video_decode_ok_time_ms = Some(5_785.0);
            stats.latest_video_packet_arrival_time_ms = Some(5_998.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 410_000,
                video_packet_count_total: 3_100,
                audio_bytes_total: 64_000,
                observed_at_ms: 5_998.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-await-recovery-keyframe".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                        observed_at_ms: 5_998.0,
                    },
                    observed_at_ms: 5_998.0,
                });
        },
    );
    assert!(local_recover.iter().any(|command| {
        matches!(command, TransportCommand::RequestKeyframe { reason, .. } if reason == "transportAwaitRecoveryKeyframe")
    }));
    assert!(local_recover
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })));

    let reconnect = harness.apply(
        6_420.0,
        ConnectionLifecycleStateFact::Recovering,
        "rtcConnectionRecovering",
        220,
        |stats| {
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Disconnected;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 220;
            stats.latest_video_host_present_time_ms = Some(5_760.0);
            stats.latest_video_decode_ok_time_ms = Some(5_785.0);
            stats.latest_video_packet_arrival_time_ms = Some(6_000.0);
            stats.video_renderer_stalled = Some(true);
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.transport_state =
                    xbxengine_protocol::XbxEngineTransportStateDto::Disconnected;
                track.observed_at_ms = 6_418.0;
            }
        },
    );
    assert!(reconnect.iter().any(|command| {
        matches!(command, TransportCommand::RequestReconnectCandidate { reason, .. } if reason == "rtcConnectionRecovering")
    }));
}

#[test]
fn cloud_high_rtt_repeated_transport_severe_deadline_second_hit_reconnects() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud);
        stats.session_phase = Some("steady".to_string());
        stats.transport_recovery_epoch = 41;
        stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Disconnected;
        stats.host_no_pending_pressure_level = Some("critical".to_string());
        stats.host_no_pending_streak = 360;
        stats.latest_video_host_present_time_ms = Some(7_000.0);
        stats.latest_video_decode_ok_time_ms = Some(7_400.0);
        stats.latest_video_packet_arrival_time_ms = Some(7_520.0);
        stats.video_decoder_stalled = Some(false);
        stats.video_renderer_stalled = Some(true);
        stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
            state: "remoteTrackAttached".to_string(),
            video_width: Some(1920),
            video_height: Some(1080),
            mime_type: Some("video/H264".to_string()),
            transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Disconnected,
            video_bytes_total: 512_000,
            video_packet_count_total: 4_200,
            audio_bytes_total: 96_000,
            observed_at_ms: 7_540.0,
        });
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 1,
            source_event: "frame-await-recovery-keyframe".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "recovering".to_string(),
                reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                observed_at_ms: 7_540.0,
            },
            observed_at_ms: 7_540.0,
        });
    }
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());

    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
    connection.control_channel_open = true;
    connection.latest_transport_path = Some("Direct".to_string());
    connection.latest_rtt_ms = Some(240.0);
    connection.latest_loss_ratio_1s = Some(0.06);
    connection.last_observed_at_ms = Some(8_000.0);

    let first = TransportSnapshot::new(
        1,
        8_000.0,
        connection.clone(),
        MediaProjection {
            frame_count: 220,
            ..MediaProjection::default()
        },
        RecoveryProjection {
            latest_diagnosis_label: Some("transportSevereDeadline".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(8_000.0),
        },
        BweProjection {
            latest_rtt_ms: Some(240.0),
            latest_loss_ratio_1s: Some(0.06),
            latest_actual_video_bitrate_kbps: Some(6_000.0),
            latest_observed_remb_kbps: Some(8_000),
            latest_transport_path: Some("Direct".to_string()),
            latest_sample_tick_ms: Some(8_000.0),
            target_remb_kbps: Some(8_000),
            last_observed_at_ms: Some(8_000.0),
        },
        DiagnosticsProjection::default(),
    );
    let first_commands = policy.on_snapshot(&first);
    assert!(first_commands
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })));

    let second = TransportSnapshot::new(
        2,
        8_040.0,
        connection,
        MediaProjection {
            frame_count: 220,
            ..MediaProjection::default()
        },
        RecoveryProjection {
            latest_diagnosis_label: Some("transportSevereDeadline".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(8_040.0),
        },
        BweProjection {
            latest_rtt_ms: Some(240.0),
            latest_loss_ratio_1s: Some(0.06),
            latest_actual_video_bitrate_kbps: Some(5_600.0),
            latest_observed_remb_kbps: Some(7_000),
            latest_transport_path: Some("Direct".to_string()),
            latest_sample_tick_ms: Some(8_040.0),
            target_remb_kbps: Some(7_000),
            last_observed_at_ms: Some(8_040.0),
        },
        DiagnosticsProjection::default(),
    );
    let second_commands = policy.on_snapshot(&second);
    assert!(second_commands.iter().any(|command| {
        matches!(
            command,
            TransportCommand::RequestReconnectCandidate {
                reason,
                reason_domain,
                ..
            } if reason == "transportSevereDeadline"
                && *reason_domain == crate::XbxEngineRecoveryReasonDomain::ConnectivityTransport
        )
    }));
}

#[test]
fn cloud_high_rtt_repeated_transport_expired_deadline_third_hit_reconnects() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud);
        stats.session_phase = Some("steady".to_string());
        stats.transport_recovery_epoch = 42;
        stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Disconnected;
        stats.host_no_pending_pressure_level = Some("critical".to_string());
        stats.host_no_pending_streak = 380;
        stats.latest_video_host_present_time_ms = Some(9_000.0);
        stats.latest_video_decode_ok_time_ms = Some(9_350.0);
        stats.latest_video_packet_arrival_time_ms = Some(9_480.0);
        stats.video_decoder_stalled = Some(false);
        stats.video_renderer_stalled = Some(true);
        stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
            state: "remoteTrackAttached".to_string(),
            video_width: Some(1920),
            video_height: Some(1080),
            mime_type: Some("video/H264".to_string()),
            transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Disconnected,
            video_bytes_total: 520_000,
            video_packet_count_total: 4_400,
            audio_bytes_total: 96_000,
            observed_at_ms: 9_500.0,
        });
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 1,
            source_event: "frame-await-recovery-keyframe".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "recovering".to_string(),
                reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                observed_at_ms: 9_500.0,
            },
            observed_at_ms: 9_500.0,
        });
    }
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);

    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
    connection.control_channel_open = true;
    connection.latest_transport_path = Some("Direct".to_string());
    connection.latest_rtt_ms = Some(260.0);
    connection.latest_loss_ratio_1s = Some(0.04);
    connection.last_observed_at_ms = Some(10_000.0);

    for (version, now_ms, expect_reconnect) in [
        (1, 10_000.0, false),
        (2, 10_460.0, false),
        (3, 10_920.0, true),
    ] {
        let snapshot = TransportSnapshot::new(
            version,
            now_ms,
            connection.clone(),
            MediaProjection {
                frame_count: 220,
                ..MediaProjection::default()
            },
            RecoveryProjection {
                latest_diagnosis_label: Some("transportExpiredDeadline".to_string()),
                pending_action: false,
                successful_action_count: 0,
                failed_action_count: 0,
                last_observed_at_ms: Some(now_ms),
            },
            BweProjection {
                latest_rtt_ms: Some(260.0),
                latest_loss_ratio_1s: Some(0.04),
                latest_actual_video_bitrate_kbps: Some(5_200.0),
                latest_observed_remb_kbps: Some(6_500),
                latest_transport_path: Some("Direct".to_string()),
                latest_sample_tick_ms: Some(now_ms),
                target_remb_kbps: Some(6_500),
                last_observed_at_ms: Some(now_ms),
            },
            DiagnosticsProjection::default(),
        );
        let commands = policy.on_snapshot(&snapshot);
        if expect_reconnect {
            assert!(commands.iter().any(|command| {
                matches!(
                    command,
                    TransportCommand::RequestReconnectCandidate {
                        reason,
                        reason_domain,
                        ..
                    } if reason == "transportExpiredDeadline"
                        && *reason_domain == crate::XbxEngineRecoveryReasonDomain::ConnectivityTransport
                )
            }));
        } else {
            assert!(commands.iter().all(|command| !matches!(
                command,
                TransportCommand::RequestReconnectCandidate { .. }
            )));
        }
        if version < 3 {
            sleep(Duration::from_millis(450));
        }
    }
}

#[test]
fn cloud_high_rtt_sample_loss_then_recovered_late_stays_local_until_severe_deadline_reconnect() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

    let sample_loss = harness.apply(
        10_800.0,
        ConnectionLifecycleStateFact::Connected,
        "transportSampleLoss",
        220,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 43;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Disconnected;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 320;
            stats.latest_video_host_present_time_ms = Some(now_ms - 1_180.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 740.0);
            stats.latest_video_packet_arrival_time_ms = Some(now_ms - 120.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(true);
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Disconnected,
                video_bytes_total: 540_000,
                video_packet_count_total: 4_500,
                audio_bytes_total: 96_000,
                observed_at_ms: now_ms - 4.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 14,
                    source_event: "frame-await-recovery-keyframe".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                        observed_at_ms: now_ms - 4.0,
                    },
                    observed_at_ms: now_ms - 4.0,
                });
        },
    );
    assert!(sample_loss
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })));
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("sample loss decision ledger");
        assert_eq!(
            ledger.input_signal,
            "transportSampleLoss:transportSampleLoss"
        );
        assert_ne!(ledger.action_selected, "requestReconnectCandidate");
    });

    let recovered_late = harness.apply(
        10_920.0,
        ConnectionLifecycleStateFact::Connected,
        "transportRecoveredLate",
        220,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.latest_video_host_present_time_ms = Some(now_ms - 1_220.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 780.0);
            stats.latest_video_packet_arrival_time_ms = Some(now_ms - 160.0);
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total += 12_000;
                track.video_packet_count_total += 80;
                track.observed_at_ms = now_ms - 4.0;
            }
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.observation_id = 15;
                timeline.chain.observed_at_ms = now_ms - 4.0;
                timeline.observed_at_ms = now_ms - 4.0;
            }
        },
    );
    assert!(recovered_late
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })));
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("transport recovered late decision ledger");
        assert_eq!(
            ledger.input_signal,
            "transportRecoveredLate:transportRecoveredLate"
        );
        assert_ne!(ledger.action_selected, "requestReconnectCandidate");
    });

    let severe_first = harness.apply(
        10_960.0,
        ConnectionLifecycleStateFact::Connected,
        "transportSevereDeadline",
        220,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.latest_video_host_present_time_ms = Some(now_ms - 1_260.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 820.0);
            stats.latest_video_packet_arrival_time_ms = Some(now_ms - 220.0);
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.observed_at_ms = now_ms - 4.0;
            }
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.observation_id = 16;
                timeline.chain.observed_at_ms = now_ms - 4.0;
                timeline.observed_at_ms = now_ms - 4.0;
            }
        },
    );
    assert!(severe_first
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })));

    let severe_second = harness.apply(
        11_000.0,
        ConnectionLifecycleStateFact::Connected,
        "transportSevereDeadline",
        220,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.latest_video_host_present_time_ms = Some(now_ms - 1_300.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 860.0);
            stats.latest_video_packet_arrival_time_ms = Some(now_ms - 260.0);
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.observed_at_ms = now_ms - 4.0;
            }
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.observation_id = 17;
                timeline.chain.observed_at_ms = now_ms - 4.0;
                timeline.observed_at_ms = now_ms - 4.0;
            }
        },
    );
    assert!(severe_second.iter().any(|command| {
        matches!(
            command,
            TransportCommand::RequestReconnectCandidate {
                reason,
                reason_domain,
                ..
            } if reason == "transportSevereDeadline"
                && *reason_domain == crate::XbxEngineRecoveryReasonDomain::ConnectivityTransport
        )
    }));
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("severe deadline decision ledger");
        assert_eq!(
            ledger.input_signal,
            "transportSevereDeadline:transportSevereDeadline"
        );
        assert_eq!(ledger.action_selected, "requestReconnectCandidate");
    });
}
