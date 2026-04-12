use super::super::RtcSessionPolicy;
use crate::api::backend::XbxEngineMediaRuntimeStats;
use crate::api::runtime::XbxEngineRuntimeConfig;
use crate::transport::rtc::facts::{ConnectionLifecycleStateFact, TransportCommand};
use crate::transport::rtc::projection::{
    BweProjection, ConnectionProjection, DiagnosticsProjection, MediaProjection,
    RecoveryProjection, TransportSnapshot,
};
use crate::transport::rtc::session::actor::SessionPolicyHook;
use std::sync::{Arc, Mutex};

use super::harness::{
    assert_recovery_family_hold_semantics, build_snapshot, transport_commands,
    RecoveryIntegrationHarness,
};

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
    let first_commands = transport_commands(policy.on_snapshot(&first));
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
    let second_commands = transport_commands(policy.on_snapshot(&second));
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
    let third_commands = transport_commands(policy.on_snapshot(&third));
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
            ..Default::default()
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
                transport_commands(policy.on_snapshot(&snapshot))
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
        ..Default::default()
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
    let first_commands = transport_commands(policy.on_snapshot(&first));
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
    let commands = transport_commands(policy.on_snapshot(&snapshot));
    assert!(commands
        .iter()
        .any(|command| matches!(command, TransportCommand::RequestKeyframe { .. })));
}

#[test]
fn pre_first_frame_bootstrap_missing_sps_emits_local_keyframe_probe() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
        stats.session_phase = Some("priming".to_string());
        stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
            state: "remoteTrackAttached".to_string(),
            video_width: Some(1920),
            video_height: Some(1080),
            mime_type: Some("video/H264".to_string()),
            transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
            video_bytes_total: 128_000,
            video_packet_count_total: 96,
            audio_bytes_total: 16_000,
            observed_at_ms: 100.0,
        });
        stats.latest_video_decode_ok_time_ms = None;
        stats.latest_video_host_present_time_ms = None;
        stats.first_video_packet_arrival_time_ms = Some(10.0);
    }
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    let snapshot = build_snapshot(
        ConnectionLifecycleStateFact::Connected,
        "bootstrapMissingSps",
        100.0,
    );
    let commands = transport_commands(policy.on_snapshot(&snapshot));
    assert!(commands.iter().any(
        |command| matches!(command, TransportCommand::RequestKeyframe { reason, .. } if reason == "bootstrapMissingSps")
    ));

    let stats = runtime_stats.lock().expect("runtime stats lock");
    let ledger = stats
        .latest_recovery_decision_ledger
        .as_ref()
        .expect("recovery decision ledger");
    assert_eq!(
        ledger.input_signal,
        "transportAwaitRecoveryKeyframe:bootstrapMissingSps"
    );
    assert_eq!(ledger.action_selected, "requestKeyframe");
}

#[test]
fn pre_first_frame_bootstrap_missing_sps_with_recent_episode_coalesces_probe() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
        stats.session_phase = Some("priming".to_string());
        stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
            state: "remoteTrackAttached".to_string(),
            video_width: Some(1920),
            video_height: Some(1080),
            mime_type: Some("video/H264".to_string()),
            transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
            video_bytes_total: 128_000,
            video_packet_count_total: 96,
            audio_bytes_total: 16_000,
            observed_at_ms: 100.0,
        });
        stats.first_video_packet_arrival_time_ms = Some(10.0);
        stats.latest_keyframe_request_episode =
            Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                episode_id: 7,
                request_reason: Some("bootstrapMissingSps".to_string()),
                request_kind: Some("pli".to_string()),
                status: "requested".to_string(),
                status_detail: None,
                requested_at_ms: 99.0,
                sent_at_ms: None,
                deadline_at_ms: Some(299.0),
                transport_detail: None,
                first_video_packet_at_ms: None,
                first_video_packet_rtp_timestamp: None,
                first_video_packet_is_keyframe: None,
                first_keyframe_packet_at_ms: None,
                first_keyframe_decoded_at_ms: None,
                response_rtp_timestamp: None,
                response_frame_seq: None,
                response_verdict: None,
                lifecycle_phase: None,
            });
    }
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    let snapshot = build_snapshot(
        ConnectionLifecycleStateFact::Connected,
        "bootstrapMissingSps",
        100.0,
    );
    let commands = transport_commands(policy.on_snapshot(&snapshot));
    assert!(
        commands
            .iter()
            .all(|command| !matches!(command, TransportCommand::RequestKeyframe { .. })),
        "recent first-frame keyframe episode should stay coalesced locally: {commands:?}"
    );

    let stats = runtime_stats.lock().expect("runtime stats lock");
    let ledger = stats
        .latest_recovery_decision_ledger
        .as_ref()
        .expect("recovery decision ledger");
    assert_eq!(
        ledger.input_signal,
        "transportAwaitRecoveryKeyframe:bootstrapMissingSps"
    );
    assert_recovery_family_hold_semantics(
        ledger.gate_result.as_str(),
        ledger.action_selected.as_str(),
    );
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
        ..Default::default()
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
    let first_commands = transport_commands(policy.on_snapshot(&first));
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
    let second_commands = transport_commands(policy.on_snapshot(&second));
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
    let third_commands = transport_commands(policy.on_snapshot(&third));
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
    let fourth_commands = transport_commands(policy.on_snapshot(&fourth));
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
    let fifth_commands = transport_commands(policy.on_snapshot(&fifth));
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
        ..Default::default()
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
    let first_commands = transport_commands(policy.on_snapshot(&first));
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
    let second_commands = transport_commands(policy.on_snapshot(&second));
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
        ..Default::default()
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
    let first_commands = transport_commands(policy.on_snapshot(&first));
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
    let second_commands = transport_commands(policy.on_snapshot(&second));
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
    let third_commands = transport_commands(policy.on_snapshot(&third));
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
        ..Default::default()
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
    let _ = transport_commands(policy.on_snapshot(&first));

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
    let second_commands = transport_commands(policy.on_snapshot(&second));
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
    let third_commands = transport_commands(policy.on_snapshot(&third));
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
        ..Default::default()
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
    let first_commands = transport_commands(policy.on_snapshot(&first));
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
    let second_commands = transport_commands(policy.on_snapshot(&second));
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
    let third_commands = transport_commands(policy.on_snapshot(&third));
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
    let fourth_commands = transport_commands(policy.on_snapshot(&fourth));
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
    let fifth_commands = transport_commands(policy.on_snapshot(&fifth));
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
        ..Default::default()
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
        let _ = transport_commands(policy.on_snapshot(&snapshot));
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
    let resumed_commands = transport_commands(policy.on_snapshot(&resumed));
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
fn connected_ingress_without_success_output_can_enter_failed_terminal_after_reconnect_exhaustion() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    let now_ms = 15_000.0;
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
        stats.inbound_primary_video_bytes_total = 10_000;
        stats.latest_video_packet_arrival_time_ms = Some(now_ms - 100.0);
        stats.latest_video_decode_ok_time_ms = Some(now_ms - 15_000.0);
        stats.latest_video_host_present_time_ms = Some(now_ms - 15_000.0);
    }
    policy.reconnect_grants_without_success_edge = policy.liveness_reconnect_attempt_limit();

    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
    let snapshot = TransportSnapshot::new(
        1,
        now_ms,
        connection,
        MediaProjection {
            frame_count: 180,
            ..MediaProjection::default()
        },
        RecoveryProjection {
            latest_diagnosis_label: Some("transportAwaitRecoveryKeyframe".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(now_ms),
            ..Default::default()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );

    assert!(policy.should_enter_connected_ingress_without_success_output_failed_terminal(
        &snapshot,
        crate::transport::rtc::policy::video_scheduling_owner::VideoSchedulingOwnerState::RebuildingSupply,
        now_ms,
    ));
}

#[test]
fn same_tick_failed_terminal_does_not_forward_original_reconnect_proposal() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud);
        stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Disconnected;
        stats.session_phase = Some("steady".to_string());
        stats.transport_recovery_epoch = 41;
        stats.host_no_pending_pressure_level = Some("critical".to_string());
        stats.host_no_pending_streak = 360;
        stats.latest_video_host_present_time_ms = Some(2_000.0);
        stats.latest_video_decode_ok_time_ms = Some(2_400.0);
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
                chain_break_evidence: None,

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
            ..Default::default()
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
    let first_commands = transport_commands(policy.on_snapshot(&first));
    assert!(first_commands
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })));

    if let Ok(mut stats) = runtime_stats.lock() {
        stats.inbound_primary_video_bytes_total = 10_000;
        stats.latest_video_packet_arrival_time_ms = Some(15_000.0 - 80.0);
        stats.latest_video_host_present_time_ms = Some(2_000.0);
        stats.latest_video_decode_ok_time_ms = Some(2_400.0);
    }
    policy.reconnect_grants_without_success_edge = policy.liveness_reconnect_attempt_limit();

    let snapshot = TransportSnapshot::new(
        2,
        15_000.0,
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
            last_observed_at_ms: Some(15_000.0),
            ..Default::default()
        },
        BweProjection {
            latest_rtt_ms: Some(240.0),
            latest_loss_ratio_1s: Some(0.06),
            latest_actual_video_bitrate_kbps: Some(5_600.0),
            latest_observed_remb_kbps: Some(7_000),
            latest_transport_path: Some("Direct".to_string()),
            latest_sample_tick_ms: Some(15_000.0),
            target_remb_kbps: Some(7_000),
            last_observed_at_ms: Some(15_000.0),
        },
        DiagnosticsProjection::default(),
    );

    let commands = transport_commands(policy.on_snapshot(&snapshot));
    assert!(
        commands
            .iter()
            .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })),
        "failed-terminal same tick must not forward reconnect commands: {commands:?}"
    );
    let stats = runtime_stats.lock().expect("runtime stats lock");
    let ledger = stats
        .latest_recovery_decision_ledger
        .as_ref()
        .expect("recovery decision ledger");
    assert_eq!(ledger.state_after, "failed-terminal");
    assert_eq!(
        ledger.gate_result,
        "terminal:connectedIngressWithoutSuccessfulOutput"
    );
    assert_eq!(ledger.action_selected, "failed-terminal");
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
            ..Default::default()
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
        let _ = transport_commands(policy.on_snapshot(&first));
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
        let second_commands = transport_commands(policy.on_snapshot(&second));
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
        ..Default::default()
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
    let _ = transport_commands(policy.on_snapshot(&first));

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
    let second_commands = transport_commands(policy.on_snapshot(&second));
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
    let third_commands = transport_commands(policy.on_snapshot(&third));
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
        ..Default::default()
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
    let _ = transport_commands(policy.on_snapshot(&first));

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
    let second_commands = transport_commands(policy.on_snapshot(&second));
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
    let third_commands = transport_commands(policy.on_snapshot(&third));
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
                        chain_break_evidence: None,

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
        ..Default::default()
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
    let first_commands = transport_commands(policy.on_snapshot(&first));
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
    let second_commands = transport_commands(policy.on_snapshot(&second));
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
    let third_commands = transport_commands(policy.on_snapshot(&third));
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
    let fourth_commands = transport_commands(policy.on_snapshot(&fourth));
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
        let commands = transport_commands(policy.on_snapshot(&snapshot));
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
    let terminal_commands = transport_commands(policy.on_snapshot(&terminal));
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
        ..Default::default()
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
    let first_commands = transport_commands(policy.on_snapshot(&first));
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
        let commands = transport_commands(policy.on_snapshot(&snapshot));
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
    let pre_terminal_commands = transport_commands(policy.on_snapshot(&pre_terminal));
    assert!(
        pre_terminal_commands.iter().all(|command| {
            !matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }),
        "New 首窗应彻底禁止 liveness reconnect 候选"
    );
    {
        let stats = runtime_stats.lock().expect("runtime stats lock");
        assert!(
            stats.latest_rtc_builder_observation.is_none(),
            "cloud new 首窗 soft hold 应在 builder 尚未出现时就生效"
        );
    }

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
    let long_new_commands = transport_commands(policy.on_snapshot(&long_new));
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
        ..Default::default()
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
    let _ = transport_commands(policy.on_snapshot(&first));

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
    let second_commands = transport_commands(policy.on_snapshot(&second));
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
    let third_commands = transport_commands(policy.on_snapshot(&third));
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
        let commands = transport_commands(policy.on_snapshot(&snapshot));
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
    let terminal_commands = transport_commands(policy.on_snapshot(&terminal));
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
        ..Default::default()
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
        transport_commands(policy.on_snapshot(&warmup)).is_empty(),
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
    let first_reconnect_commands = transport_commands(policy.on_snapshot(&first_reconnect));
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
        let commands = transport_commands(policy.on_snapshot(&snapshot));
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
    let terminal_commands = transport_commands(policy.on_snapshot(&terminal));
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
    let post_terminal_commands = transport_commands(policy.on_snapshot(&post_terminal));
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
        ..Default::default()
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
        let commands = transport_commands(policy.on_snapshot(&snapshot));
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
    let terminal_commands = transport_commands(policy.on_snapshot(&terminal));
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
        ..Default::default()
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
    let _ = transport_commands(policy.on_snapshot(&first));

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
    let second_commands = transport_commands(policy.on_snapshot(&second));
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
        ..Default::default()
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
    let _ = transport_commands(policy.on_snapshot(&first));

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
    let second_commands = transport_commands(policy.on_snapshot(&second));
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
        ..Default::default()
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
    let first_commands = transport_commands(policy.on_snapshot(&first));
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
    let second_commands = transport_commands(policy.on_snapshot(&second));
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
    let third_commands = transport_commands(policy.on_snapshot(&third));
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
    let fourth_commands = transport_commands(policy.on_snapshot(&fourth));
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
    let fifth_commands = transport_commands(policy.on_snapshot(&fifth));
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
        ..Default::default()
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
    let first_commands = transport_commands(policy.on_snapshot(&first));
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
    let second_commands = transport_commands(policy.on_snapshot(&second));
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
    let third_commands = transport_commands(policy.on_snapshot(&third));
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
