use super::super::{RecoveryObservationSnapshot, RtcSessionPolicy};
use crate::api::backend::{XbxEngineMediaRuntimeStats, XbxEngineVideoTwccObservation};
use crate::api::runtime::XbxEngineRuntimeConfig;
use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::facts::{
    ConnectionLifecycleStateFact, SessionCommand, TransportCommand,
};
use crate::transport::rtc::policy::display_supply::SchedulingDemandSignal;
use crate::transport::rtc::policy::recovery::resolve_runtime_reconnect_reason_domain;
use crate::transport::rtc::projection::{
    BweProjection, ConnectionProjection, DiagnosticsProjection, MediaProjection,
    RecoveryProjection, TransportSnapshot,
};
use crate::transport::rtc::recovery::escalation::{RecoveryAction, VideoEscalationReason};
use crate::transport::rtc::session::actor::SessionPolicyHook;
use std::sync::{Arc, Mutex};
use std::thread::sleep;
use std::time::{Duration, Instant};

use super::harness::{
    assert_recovery_family_hold_semantics, build_demand_for_stats, build_snapshot,
    classify_supply_state_with_profile, set_input_rumble_burst, transport_commands,
    RecoveryIntegrationHarness,
};

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
    let commands = transport_commands(policy.on_snapshot(&snapshot));
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
    let target = transport_commands(policy.on_snapshot(&snapshot))
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
            if let SessionCommand::Transport(TransportCommand::SetTargetRembKbps {
                reason, ..
            }) = command
            {
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

    let commands = transport_commands(policy.on_snapshot(&snapshot));
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

    let commands = transport_commands(policy.on_snapshot(&snapshot));
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
    let _ = transport_commands(policy.on_snapshot(&first));

    let second = build_snapshot(
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryKeyframe",
        8_000.0,
    );
    let commands = transport_commands(policy.on_snapshot(&second));
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
    let commands = transport_commands(policy.on_snapshot(&second));
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
    let first_commands = transport_commands(policy.on_snapshot(&first));
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
    let second_commands = transport_commands(policy.on_snapshot(&second));
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
    let third_commands = transport_commands(policy.on_snapshot(&third));
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
    let third_commands = transport_commands(policy.on_snapshot(&third));
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
            if let SessionCommand::Transport(TransportCommand::SetTargetRembKbps {
                reason, ..
            }) = command
            {
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
        ..Default::default()
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
    let commands = transport_commands(policy.on_snapshot(&snapshot));
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

