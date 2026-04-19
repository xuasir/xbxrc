use super::super::RtcSessionPolicy;
use crate::api::backend::XbxEngineMediaRuntimeStats;
use crate::api::runtime::XbxEngineRuntimeConfig;
use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::facts::{ConnectionLifecycleStateFact, TransportCommand};
use crate::transport::rtc::projection::{
    BweProjection, ConnectionProjection, DiagnosticsProjection, MediaProjection,
    RecoveryProjection, TransportSnapshot,
};
use crate::transport::rtc::session::actor::SessionPolicyHook;
use std::sync::{Arc, Mutex};

use super::harness::{build_snapshot, classify_supply_state_with_profile, transport_commands};

#[test]
fn recovery_decision_ledger_is_written_with_budget_snapshot() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    let snapshot = build_snapshot(
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryAnchor",
        320.0,
    );
    let commands = transport_commands(policy.on_snapshot(&snapshot));
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
        "transportAwaitRecoveryAnchor:transportAwaitRecoveryAnchor"
    );
    assert_eq!(ledger.action_selected, "requestKeyframe");
    assert_eq!(ledger.gate_result, "pass:localProbe");
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
        "transportAwaitRecoveryAnchor",
        320.0,
    );
    let first_commands = transport_commands(policy.on_snapshot(&first));
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
    let _ = transport_commands(policy.on_snapshot(&second));
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
        "transportAwaitRecoveryAnchor",
        320.0,
    );
    let first_commands = transport_commands(policy.on_snapshot(&first));
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
    let _ = transport_commands(policy.on_snapshot(&second));
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
    let commands = transport_commands(policy.on_snapshot(&snapshot));
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
    let first = transport_commands(policy.on_snapshot(&snapshot));
    assert!(
        first.iter().all(|command| {
            !matches!(
                command,
                TransportCommand::RequestKeyframe { .. }
                    | TransportCommand::RequestDecoderReset { .. }
                    | TransportCommand::RequestReconnectCandidate { .. }
            )
        }),
        "display supply critical should be absorbed locally (no media commands): {first:?}"
    );

    snapshot.version = 2;
    snapshot.now_ms = 361.0;
    snapshot.recovery.last_observed_at_ms = Some(361.0);
    let second = transport_commands(policy.on_snapshot(&snapshot));
    assert!(
        second
            .iter()
            .all(|command| !matches!(command, TransportCommand::RequestKeyframe { .. })),
        "display supply signal should not emit keyframe commands"
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
    let commands = transport_commands(policy.on_snapshot(&snapshot));

    assert!(commands.iter().all(|command| {
        !matches!(
            command,
            TransportCommand::RequestKeyframe { .. }
                | TransportCommand::RequestDecoderReset { .. }
                | TransportCommand::RequestReconnectCandidate { .. }
        )
    }));
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
    let _ = transport_commands(policy.on_snapshot(&snapshot));
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
    let first_cmds = transport_commands(policy.on_snapshot(&first));
    assert!(
        first_cmds.iter().all(|command| {
            !matches!(
                command,
                TransportCommand::RequestKeyframe { .. }
                    | TransportCommand::RequestDecoderReset { .. }
                    | TransportCommand::RequestReconnectCandidate { .. }
            )
        }),
        "display supply critical should not feed coordinator chain: {first_cmds:?}"
    );

    let mut second = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 501.0);
    second.version = 2;
    let second_cmds = transport_commands(policy.on_snapshot(&second));
    assert!(second_cmds.iter().all(|command| {
        !matches!(
            command,
            TransportCommand::RequestKeyframe { .. }
                | TransportCommand::RequestDecoderReset { .. }
                | TransportCommand::RequestReconnectCandidate { .. }
        )
    }));
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
    let first_cmds = transport_commands(policy.on_snapshot(&first));
    assert!(first_cmds.iter().all(|command| {
        !matches!(
            command,
            TransportCommand::RequestKeyframe { .. }
                | TransportCommand::RequestDecoderReset { .. }
                | TransportCommand::RequestReconnectCandidate { .. }
        )
    }));

    let second = build_snapshot(
        ConnectionLifecycleStateFact::Connected,
        "adapterIdleTimeout",
        501.0,
    );
    let second_cmds = transport_commands(policy.on_snapshot(&second));
    assert!(second_cmds.iter().all(|command| {
        !matches!(
            command,
            TransportCommand::RequestKeyframe { .. }
                | TransportCommand::RequestDecoderReset { .. }
                | TransportCommand::RequestReconnectCandidate { .. }
        )
    }));
    let stats = runtime_stats.lock().expect("runtime stats lock");
    let ledger = stats
        .latest_recovery_decision_ledger
        .as_ref()
        .expect("recovery decision ledger");
    assert!(
        ledger.gate_result == "no-signal",
        "display domain signals should not enter media recovery analysis: {}",
        ledger.gate_result
    );
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
    let first_cmds = transport_commands(policy.on_snapshot(&first));
    assert!(first_cmds.iter().all(|command| {
        !matches!(
            command,
            TransportCommand::RequestKeyframe { .. }
                | TransportCommand::RequestDecoderReset { .. }
                | TransportCommand::RequestReconnectCandidate { .. }
        )
    }));

    let mut second = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 701.0);
    second.version = 2;
    let second_cmds = transport_commands(policy.on_snapshot(&second));
    assert!(second_cmds.iter().all(|command| {
        !matches!(
            command,
            TransportCommand::RequestKeyframe { .. }
                | TransportCommand::RequestDecoderReset { .. }
                | TransportCommand::RequestReconnectCandidate { .. }
        )
    }));

    if let Ok(mut stats) = runtime_stats.lock() {
        stats.transport_recovery_epoch = 4;
        stats.transport_recovery_epoch_at_last_escalation = 3;
    }
    let mut third = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 703.0);
    third.version = 3;
    let third_cmds = transport_commands(policy.on_snapshot(&third));
    assert!(third_cmds.iter().all(|command| {
        !matches!(
            command,
            TransportCommand::RequestKeyframe { .. }
                | TransportCommand::RequestDecoderReset { .. }
                | TransportCommand::RequestReconnectCandidate { .. }
        )
    }));
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
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    let snapshot = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 400.0);
    let _ = transport_commands(policy.on_snapshot(&snapshot));
    let stats = runtime_stats.lock().expect("runtime stats lock");
    assert_eq!(
        stats.video_owner_reason.as_deref(),
        Some("displaySupplyCritical")
    );
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
        stats.video_anchor_clean_source_event = Some("chain-clean-anchor-submitted".to_string());
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
                chain_break_evidence: None,

                observed_at_ms: 390.0,
            },
            observed_at_ms: 390.0,
        });
    }

    let first = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 400.0);
    let _ = transport_commands(policy.on_snapshot(&first));
    let second = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 420.0);
    let second_cmds = transport_commands(policy.on_snapshot(&second));
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
    let third_cmds = transport_commands(policy.on_snapshot(&third));
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
    let fourth_cmds = transport_commands(policy.on_snapshot(&fourth));
    assert!(
        fourth_cmds.iter().all(|command| {
            !matches!(
                command,
                TransportCommand::RequestKeyframe { .. }
                    | TransportCommand::RequestDecoderReset { .. }
                    | TransportCommand::RequestReconnectCandidate { .. }
            )
        }),
        "soft critical should remain local even after renderer stall: {fourth_cmds:?}"
    );
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
    let first_commands = transport_commands(policy.on_snapshot(&first));
    assert!(first_commands.iter().all(|command| {
        !matches!(
            command,
            TransportCommand::RequestKeyframe { .. }
                | TransportCommand::RequestDecoderReset { .. }
                | TransportCommand::RequestReconnectCandidate { .. }
        )
    }));

    let mut second = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 820.0);
    second.version = 2;
    second.recovery.last_observed_at_ms = Some(820.0);
    let second_commands = transport_commands(policy.on_snapshot(&second));
    assert!(second_commands.iter().all(|command| {
        !matches!(
            command,
            TransportCommand::RequestKeyframe { .. }
                | TransportCommand::RequestDecoderReset { .. }
                | TransportCommand::RequestReconnectCandidate { .. }
        )
    }));
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
    let _ = transport_commands(policy.on_snapshot(&snapshot));
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
            source_event: "frame-await-recovery-anchor".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "healthy".to_string(),
                reason: None,
                chain_break_evidence: None,

                observed_at_ms: 810.0,
            },
            observed_at_ms: 810.0,
        });
    }
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    let first = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 810.0);
    let _ = transport_commands(policy.on_snapshot(&first));

    let second = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 820.0);
    let _ = transport_commands(policy.on_snapshot(&second));
    let stats = runtime_stats.lock().expect("runtime stats lock");
    assert_eq!(
        stats.video_owner_state.as_deref(),
        Some("rebuilding-supply")
    );
    assert_eq!(
        stats.video_owner_reason.as_deref(),
        Some("transportAwaitRecoveryAnchor")
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
            source_event: "frame-await-recovery-anchor".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "recovering".to_string(),
                reason: Some("awaitRecoveryAnchor".to_string()),
                chain_break_evidence: None,

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
    let _ = transport_commands(policy.on_snapshot(&snapshot));
    let stats = runtime_stats.lock().expect("runtime stats lock");
    assert_eq!(
        stats.video_owner_state.as_deref(),
        Some("rebuilding-supply")
    );
    assert_eq!(
        stats.video_owner_reason.as_deref(),
        Some("transportAwaitRecoveryAnchor")
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
                chain_break_evidence: None,

                observed_at_ms: 900.0,
            },
            observed_at_ms: 900.0,
        });
    }
    let recovering = build_snapshot(
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryAnchor",
        900.0,
    );
    let _ = transport_commands(policy.on_snapshot(&recovering));

    if let Ok(mut stats) = runtime_stats.lock() {
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        stats.host_no_pending_streak = 0;
        stats.latest_video_host_present_time_ms = Some(now_ms - 18.0);
        stats.latest_video_decode_ok_time_ms = Some(now_ms - 12.0);
        stats.video_anchor_clean_epoch = Some(stats.transport_recovery_epoch);
        stats.video_anchor_clean_observed_at_ms = Some(now_ms - 15.0);
        stats.video_anchor_clean_source_event = Some("chain-clean-anchor-submitted".to_string());
        if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
            timeline.source_event = "frame-observed".to_string();
            timeline.chain.state = "healthy".to_string();
        }
    }
    let healed = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 930.0);
    let _ = transport_commands(policy.on_snapshot(&healed));
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
            source_event: "frame-await-recovery-anchor".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "recovering".to_string(),
                reason: Some("transportAwaitRecoveryAnchor".to_string()),
                chain_break_evidence: None,

                observed_at_ms: 900.0,
            },
            observed_at_ms: 900.0,
        });
    }
    let recovering = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 900.0);
    let _ = transport_commands(policy.on_snapshot(&recovering));

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
    let _ = transport_commands(policy.on_snapshot(&healed));
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
            source_event: "frame-await-recovery-anchor".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "recovering".to_string(),
                reason: Some("transportAwaitRecoveryAnchor".to_string()),
                chain_break_evidence: None,

                observed_at_ms: 900.0,
            },
            observed_at_ms: 900.0,
        });
    }
    let recovering = build_snapshot(
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryAnchor",
        900.0,
    );
    let _ = transport_commands(policy.on_snapshot(&recovering));

    if let Ok(mut stats) = runtime_stats.lock() {
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        stats.host_no_pending_streak = 0;
        stats.latest_video_host_present_time_ms = None;
        stats.latest_video_decode_ok_time_ms = Some(now_ms - 9.0);
        stats.video_anchor_clean_epoch = Some(stats.transport_recovery_epoch);
        stats.video_anchor_clean_observed_at_ms = Some(now_ms - 10.0);
        stats.video_anchor_clean_source_event = Some("chain-clean-anchor-submitted".to_string());
        if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
            timeline.source_event = "frame-observed".to_string();
            timeline.chain.state = "healthy".to_string();
            timeline.chain.reason = None;
        }
    }
    let waiting_present = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 930.0);
    let _ = transport_commands(policy.on_snapshot(&waiting_present));
    {
        let stats = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(
            stats.video_owner_state.as_deref(),
            Some("rebuilding-supply")
        );
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
            source_event: "frame-await-recovery-anchor".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "recovering".to_string(),
                reason: Some("transportAwaitRecoveryAnchor".to_string()),
                chain_break_evidence: None,

                observed_at_ms: 900.0,
            },
            observed_at_ms: 900.0,
        });
    }
    let recovering = build_snapshot(
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryAnchor",
        900.0,
    );
    let _ = transport_commands(policy.on_snapshot(&recovering));

    if let Ok(mut stats) = runtime_stats.lock() {
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        stats.host_no_pending_pressure_level = Some("high".to_string());
        stats.host_no_pending_streak = 6;
        stats.latest_video_host_present_time_ms = None;
        stats.latest_video_decode_ok_time_ms = Some(now_ms - 9.0);
        stats.video_anchor_clean_epoch = Some(stats.transport_recovery_epoch);
        stats.video_anchor_clean_observed_at_ms = Some(now_ms - 10.0);
        stats.video_anchor_clean_source_event = Some("chain-clean-anchor-submitted".to_string());
        if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
            timeline.source_event = "frame-observed".to_string();
            timeline.chain.state = "healthy".to_string();
            timeline.chain.reason = None;
        }
    }
    let waiting_present = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 930.0);
    let _ = transport_commands(policy.on_snapshot(&waiting_present));
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
    let _ = transport_commands(policy.on_snapshot(&healed));
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
            source_event: "frame-await-recovery-anchor".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "recovering".to_string(),
                reason: Some("transportAwaitRecoveryAnchor".to_string()),
                chain_break_evidence: None,

                observed_at_ms: 900.0,
            },
            observed_at_ms: 900.0,
        });
    }
    let recovering = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 900.0);
    let _ = transport_commands(policy.on_snapshot(&recovering));

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
    let _ = transport_commands(policy.on_snapshot(&healed));
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
        stats.video_anchor_clean_source_event = Some("chain-clean-anchor-submitted".to_string());
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
            ..Default::default()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let _ = transport_commands(policy.on_snapshot(&snapshot));
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
            source_event: "frame-await-recovery-anchor".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "recovering".to_string(),
                reason: Some("transportAwaitRecoveryAnchor".to_string()),
                chain_break_evidence: None,

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
            ..Default::default()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let _ = transport_commands(policy.on_snapshot(&first_snapshot));
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
        stats.video_anchor_clean_source_event = Some("chain-clean-anchor-submitted".to_string());
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
                chain_break_evidence: None,

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
            ..Default::default()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let _ = transport_commands(policy.on_snapshot(&second_snapshot));
    let stats = runtime_stats.lock().expect("runtime stats lock");
    assert_eq!(stats.transport_recovery_epoch, current_epoch);
    assert_eq!(stats.video_anchor_clean_epoch, Some(current_epoch));
    assert_eq!(stats.video_anchor_clean_observed_at_ms, Some(1200.0));
    assert_eq!(
        stats.video_anchor_clean_source_event.as_deref(),
        Some("chain-clean-anchor-submitted")
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
fn decoder_backend_failure_no_longer_maps_to_transport_decoder_reset_command() {
    let mut policy = RtcSessionPolicy::default();
    let snapshot = build_snapshot(
        ConnectionLifecycleStateFact::Connected,
        "decoderBackendFailure",
        180.0,
    );
    let commands = transport_commands(policy.on_snapshot(&snapshot));
    assert!(commands
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestDecoderReset { .. })));
    let ledger = policy
        .runtime_stats
        .lock()
        .expect("runtime stats lock")
        .latest_recovery_decision_ledger
        .clone()
        .expect("recovery decision ledger");
    assert!(
        matches!(
            ledger.action_selected.as_str(),
            "requestDecoderReset" | "requestKeyframe"
        ),
        "active adapter idle timeout should stay in local recovery family: {}",
        ledger.action_selected
    );
}
