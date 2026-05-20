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

use super::harness::transport_commands;

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
        stats.video_anchor_clean_source_event = Some("chain-clean-anchor-submitted".to_string());
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
                state: "receiving".to_string(),
                reason: None,
                chain_break_evidence: None,

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
            ..Default::default()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let _ = transport_commands(policy.on_snapshot(&healthy_snapshot));

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
            ..Default::default()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );

    let commands = transport_commands(policy.on_snapshot(&snapshot));
    assert!(commands
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestDecoderReset { .. })));
    assert!(commands
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })));
    let stats = runtime_stats.lock().expect("runtime stats lock");
    let ledger = stats
        .latest_recovery_decision_ledger
        .as_ref()
        .expect("recovery decision ledger");
    assert_ne!(ledger.action_selected, "requestDecoderReset");
    assert_ne!(ledger.action_selected, "requestReconnectCandidate");
    assert_ne!(ledger.state_after, "active-recovery");
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
        stats.video_anchor_clean_source_event = Some("chain-clean-anchor-submitted".to_string());
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
                state: "receiving".to_string(),
                reason: None,
                chain_break_evidence: None,

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
            ..Default::default()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let _ = transport_commands(policy.on_snapshot(&healthy_snapshot));
    // 与 `recovery_decision_ledger_allows_no_signal_to_be_latest_after_pending_command_is_resolved` 一致：
    // 待回填的 request* headline 需先被视作已决，吸收 tick 的 no-signal 才能作为 latest。
    RuntimeStatsSink::update_shared(runtime_stats.as_ref(), |stats| {
        if let Some(ledger) = stats.latest_recovery_decision_ledger.as_mut() {
            ledger.command_result = Some("succeeded".to_string());
        }
        if let Some(ledger) = stats.recent_recovery_decision_ledgers.last_mut() {
            ledger.command_result = Some("succeeded".to_string());
        }
    });

    let snapshot = TransportSnapshot::new(
        2,
        1_008.0,
        connection,
        MediaProjection {
            frame_count: 33,
            ..MediaProjection::default()
        },
        RecoveryProjection {
            latest_diagnosis_label: Some("receiverWaitingKeyframe".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(700.0),
            ..Default::default()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );

    let commands = transport_commands(policy.on_snapshot(&snapshot));
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
fn stale_transport_await_replay_is_absorbed_after_terminal_deferred_invalid_response() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.session_phase = Some("steady".to_string());
        stats.transport_recovery_epoch = 18;
        stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
        stats.host_no_pending_pressure_level = Some("normal".to_string());
        stats.host_no_pending_streak = 0;
        stats.video_decoder_stalled = Some(false);
        stats.video_renderer_stalled = Some(false);
        stats.video_anchor_clean_epoch = Some(18);
        stats.video_anchor_clean_observed_at_ms = Some(998.0);
        stats.video_anchor_clean_source_event = Some("chain-clean-anchor-submitted".to_string());
        stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
            state: "remoteTrackAttached".to_string(),
            video_width: Some(1920),
            video_height: Some(1080),
            mime_type: Some("video/H264".to_string()),
            transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
            video_bytes_total: 188_000,
            video_packet_count_total: 1_920,
            audio_bytes_total: 36_000,
            observed_at_ms: 1_000.0,
        });
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 2,
            source_event: "frame-await-recovery-anchor".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "waiting-keyframe".to_string(),
                reason: Some("receiverWaitingKeyframe".to_string()),
                chain_break_evidence: None,

                observed_at_ms: 1_000.0,
            },
            observed_at_ms: 1_000.0,
        });
        stats.latest_keyframe_request_episode =
            Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                episode_id: 88,
                request_reason: Some("receiverWaitingKeyframe".to_string()),
                request_kind: None,
                status: "deferred".to_string(),
                status_detail: None,
                requested_at_ms: 995.0,
                sent_at_ms: None,
                deadline_at_ms: None,
                transport_detail: None,
                first_video_packet_at_ms: None,
                first_video_packet_rtp_timestamp: None,
                first_video_packet_is_keyframe: None,
                first_keyframe_packet_at_ms: None,
                first_keyframe_decoded_at_ms: None,
                response_rtp_timestamp: None,
                response_frame_seq: None,
                response_verdict: Some("transportDeferred".to_string()),
                lifecycle_phase: None,
                retired_at_ms: None,
            });
        stats.latest_h264_inspection_observation =
            Some(crate::XbxEngineH264InspectionObservation {
                observation_id: 3,
                frame_rtp_timestamp: None,
                nal_types: vec!["SliceLayerWithoutPartitioningNonIdr".to_string()],
                nal_count: 1,
                vcl_nal_count: 1,
                has_inband_sps: false,
                has_inband_pps: false,
                committed_sps_present: true,
                committed_pps_present: true,
                slice_headers_valid: true,
                delta_continuation_ready: true,
                parameter_sets_changed: false,
                config_changed: false,
                is_idr: false,
                sample_width: Some(1920),
                sample_height: Some(1080),
                bootstrap_ready: false,
                bootstrap_reject_reason: Some("NonIdrVcl".to_string()),
                admission_accepted: true,
                observed_at_ms: 1_000.0,

                ..Default::default()
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
            ..Default::default()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let _ = transport_commands(policy.on_snapshot(&healthy_snapshot));
    RuntimeStatsSink::update_shared(runtime_stats.as_ref(), |stats| {
        if let Some(ledger) = stats.latest_recovery_decision_ledger.as_mut() {
            ledger.command_result = Some("succeeded".to_string());
        }
        if let Some(ledger) = stats.recent_recovery_decision_ledgers.last_mut() {
            ledger.command_result = Some("succeeded".to_string());
        }
    });

    let snapshot = TransportSnapshot::new(
        2,
        1_008.0,
        connection,
        MediaProjection {
            frame_count: 33,
            ..MediaProjection::default()
        },
        RecoveryProjection {
            latest_diagnosis_label: Some("receiverWaitingKeyframe".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(700.0),
            ..Default::default()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );

    let commands = transport_commands(policy.on_snapshot(&snapshot));
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
