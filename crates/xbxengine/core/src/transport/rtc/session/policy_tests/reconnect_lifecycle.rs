use super::super::RtcSessionPolicy;
use crate::api::backend::{
    XbxEngineMediaRuntimeStats, XbxEngineRemoteAnswerObservation, XbxEngineRtcBuilderObservation,
    XbxEngineVideoTwccObservation,
};
use crate::api::runtime::XbxEngineRuntimeConfig;
use crate::transport::rtc::facts::{ConnectionLifecycleStateFact, TransportCommand};
use crate::transport::rtc::policy::recovery::RecoveryPolicyProposal;
use crate::transport::rtc::policy::scheduling::TwccWarmupState;
use crate::transport::rtc::policy::video_scheduling_owner::VideoSchedulingOwnerState;
use crate::transport::rtc::projection::{
    BweProjection, ConnectionProjection, DiagnosticsProjection, MediaProjection,
    RecoveryProjection, TransportSnapshot,
};
use crate::transport::rtc::recovery::coordinator::{CoordinatorProposal, RecoveryOwnerSignal};
use crate::transport::rtc::recovery::escalation::{
    RecoveryAction, RecoveryActionBudgetState, VideoEscalationDecision, VideoEscalationReason,
};
use crate::transport::rtc::session::actor::SessionPolicyHook;
use crate::transport::rtc::session::expensive_recovery_gate::ExpensiveRecoveryGate;
use std::sync::{Arc, Mutex};

use super::harness::{
    assert_recovery_family_hold_semantics, build_snapshot, seed_pre_first_frame_acquisition_stats,
    seed_structured_recovery_label, transport_commands, RecoveryIntegrationHarness,
};

fn reconnect_candidate_proposal() -> CoordinatorProposal {
    let budget = RecoveryActionBudgetState {
        recovery_epoch: 0,
        keyframe_budget_used: 0,
        keyframe_budget_limit: 255,
        decoder_reset_budget_used: 0,
        decoder_reset_budget_limit: 255,
        reconnect_budget_used: 0,
        reconnect_budget_limit: 3,
    };
    CoordinatorProposal {
        decision: VideoEscalationDecision {
            observation_id: 1,
            action: RecoveryAction::RequestReconnectCandidate,
        },
        coalescing_mode: None,
        unlock_reason: None,
        preempt_reason: None,
        budget_before: budget,
        budget_after: budget,
    }
}

fn transport_await_owner_signal(observed_at_ms: f64) -> RecoveryOwnerSignal {
    RecoveryOwnerSignal {
        reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
        reason_label: "receiverWaitingKeyframe".to_string(),
        observed_at_ms,
        gap_severity: None,
        repairability: None,
    }
}

fn lifecycle_liveness_owner_signal(observed_at_ms: f64) -> RecoveryOwnerSignal {
    RecoveryOwnerSignal {
        reason: VideoEscalationReason::LifecycleRecovering,
        reason_label: "livenessNoProgressTimeout".to_string(),
        observed_at_ms,
        gap_severity: None,
        repairability: None,
    }
}

fn stable_video_twcc_observation(
    observation_id: u64,
    observed_at_ms: f64,
) -> XbxEngineVideoTwccObservation {
    XbxEngineVideoTwccObservation {
        observation_id,
        source: "local-feedback".to_string(),
        feedback_packet_count: 9,
        covered_sequence_start: 9166,
        covered_sequence_end: 9359,
        covered_sequence_span: 194,
        observed_packet_count: 194,
        observed_byte_count: 225_875,
        coverage_ratio: Some(1.0),
        ledger_hit_ratio: Some(1.0),
        feedback_interval_ms: Some(102.0),
        arrival_span_ms: Some(129.0),
        receive_bitrate_kbps: Some(17_715.0),
        twcc_sample_valid: true,
        twcc_invalid_reason: None,
        quality: crate::XbxEngineTwccObservationQuality::Stable,
        delivery_ratio: 1.0,
        packet_loss_ratio: 0.0,
        observed_at_ms,
    }
}

fn seed_transport_await_hard_evidence(stats: &mut XbxEngineMediaRuntimeStats, observed_at_ms: f64) {
    stats.latest_h264_inspection_observation = Some(crate::XbxEngineH264InspectionObservation {
        observed_at_ms,
        admission_accepted: false,
        bootstrap_ready: false,
        bootstrap_reject_reason: Some("bootstrapMissingSps".to_string()),
        ..Default::default()
    });
    stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
        observation_id: 10,
        source_event: "frame-await-recovery-anchor".to_string(),
        gap: None,
        frame: None,
        chain: crate::XbxEngineVideoTimelineChainSnapshot {
            state: "waiting-keyframe".to_string(),
            reason: Some("receiverWaitingKeyframe".to_string()),
            chain_break_evidence: None,
            observed_at_ms,
        },
        observed_at_ms,
    });
}

#[test]
fn reconnect_command_is_throttled_and_re_emitted_during_continuous_recovering() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        seed_structured_recovery_label(&mut stats, "rtcPeerConnectionFailed");
    }
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
            seed_structured_recovery_label(&mut stats, "rtcPeerConnectionFailed");
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
fn pre_first_frame_connecting_remote_answer_progress_resets_liveness_timeout() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud);
        stats.latest_rtc_builder_observation = Some(XbxEngineRtcBuilderObservation {
            observation_id: 1,
            controlled_twcc_registry: true,
            feedback_interval_ms: 100.0,
            registered_header_extensions: vec![],
            registered_rtcp_feedback: vec![],
            observed_at_ms: 100.0,
        });
    }
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Connecting;
    let media = MediaProjection::default();
    let recovery = RecoveryProjection::default();

    let first = TransportSnapshot::new(
        1,
        100.0,
        connection.clone(),
        media.clone(),
        recovery.clone(),
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    assert!(
        transport_commands(policy.on_snapshot(&first)).is_empty(),
        "初始 connecting 采样只应建立 no-progress 基线"
    );

    if let Ok(mut stats) = runtime_stats.lock() {
        stats.latest_remote_answer_observation = Some(XbxEngineRemoteAnswerObservation {
            observation_id: 9,
            video_payload_order: vec![124],
            selected_video_payload_type: Some(124),
            selected_video_mime_type: Some("video/h264".to_string()),
            selected_video_profile_level_id: Some("4d002a".to_string()),
            selected_video_h264_sprop_parameter_sets: None,
            accepted_video_rtcp_feedback: vec!["transport-cc".to_string(), "nack:pli".to_string()],
            accepted_audio_rtcp_feedback: vec![],
            accepted_video_header_extensions: vec![],
            accepted_audio_header_extensions: vec![],
            observed_at_ms: 4_700.0,
        });
    }
    let second = TransportSnapshot::new(
        2,
        4_700.0,
        connection.clone(),
        media.clone(),
        recovery.clone(),
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    assert!(
        transport_commands(policy.on_snapshot(&second)).is_empty(),
        "远端 answer 刚接受时应视为协商进展，继续等待当前 rebuild 完成"
    );

    let third = TransportSnapshot::new(
        3,
        40_000.0,
        connection,
        media,
        recovery,
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    assert!(transport_commands(policy.on_snapshot(&third))
        .iter()
        .any(|command| matches!(command, TransportCommand::RequestReconnectCandidate { .. })));
}

#[test]
fn direct_ice_zero_response_probe_accelerates_pre_first_frame_reconnect() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Home);
    }
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
    let mut connection = ConnectionProjection {
        lifecycle_state: ConnectionLifecycleStateFact::Connecting,
        ice_candidate_pair_count: 4,
        ice_max_requests_sent: 35,
        ice_responses_received_total: 0,
        ice_has_selected_or_nominated_pair: false,
        ice_direct_checks_without_response: true,
        ice_probe_observed_at_ms: Some(100.0),
        ..ConnectionProjection::default()
    };
    let media = MediaProjection::default();
    let recovery = RecoveryProjection::default();

    let first = TransportSnapshot::new(
        1,
        100.0,
        connection.clone(),
        media.clone(),
        recovery.clone(),
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    assert!(transport_commands(policy.on_snapshot(&first)).is_empty());

    connection.ice_probe_observed_at_ms = Some(11_900.0);
    let second = TransportSnapshot::new(
        2,
        11_900.0,
        connection.clone(),
        media.clone(),
        recovery.clone(),
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    assert!(
        transport_commands(policy.on_snapshot(&second))
            .iter()
            .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })),
        "official zero-response evidence is still held before the 12s direct ICE window"
    );

    connection.ice_probe_observed_at_ms = Some(12_200.0);
    let third = TransportSnapshot::new(
        3,
        12_200.0,
        connection,
        media,
        recovery,
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    assert!(
        transport_commands(policy.on_snapshot(&third))
            .iter()
            .any(|command| matches!(command, TransportCommand::RequestReconnectCandidate { .. })),
        "official zero-response candidate-pair evidence should accelerate reconnect"
    );
}

#[test]
fn direct_ice_probe_with_responses_keeps_waiting_for_first_frame() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Home);
    }
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
    let connection = ConnectionProjection {
        lifecycle_state: ConnectionLifecycleStateFact::Connecting,
        ice_candidate_pair_count: 1,
        ice_nominated_pair_count: 1,
        ice_succeeded_pair_count: 1,
        ice_max_requests_sent: 133,
        ice_max_responses_received: 133,
        ice_responses_received_total: 133,
        ice_has_selected_or_nominated_pair: true,
        ice_direct_checks_without_response: false,
        ice_probe_observed_at_ms: Some(12_200.0),
        ..ConnectionProjection::default()
    };
    let media = MediaProjection::default();
    let recovery = RecoveryProjection::default();

    let first = TransportSnapshot::new(
        1,
        100.0,
        connection.clone(),
        media.clone(),
        recovery.clone(),
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    assert!(transport_commands(policy.on_snapshot(&first)).is_empty());

    let second = TransportSnapshot::new(
        2,
        12_200.0,
        connection,
        media,
        recovery,
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    assert!(
        transport_commands(policy.on_snapshot(&second))
            .iter()
            .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })),
        "candidate-pair responses and nomination are valid connectivity progress"
    );
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
    if let Ok(mut stats) = runtime_stats.lock() {
        seed_pre_first_frame_acquisition_stats(&mut stats, "receiverWaitingKeyframe", 100.0);
    }
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    let snapshot = build_snapshot(
        ConnectionLifecycleStateFact::Connected,
        "receiverWaitingKeyframe",
        100.0,
    );
    let commands = transport_commands(policy.on_snapshot(&snapshot));
    assert!(
        commands.iter().all(|command| !matches!(
            command,
            TransportCommand::RequestPli { .. } | TransportCommand::RequestFir { .. }
        )),
        "session policy 不再下发 PLI/FIR；关键帧由 RtcReceiveCore 本地执行: {commands:?}"
    );
    let stats = runtime_stats.lock().expect("runtime stats lock");
    let ledger = stats
        .latest_recovery_decision_ledger
        .as_ref()
        .expect("coordinator 合同应写入 recovery decision ledger");
    assert_eq!(
        ledger.input_signal, "waitKeyframe:receiverWaitingKeyframe",
        "fallback 诊断经 map_label 进入 WaitKeyframe，仍应到达 coordinator 并写 ledger"
    );
    assert!(
        matches!(
            ledger.action_selected.as_str(),
            "cooldownSuppressed" | "delegatedToReceive" | "coalesced:keyframeInFlight"
        ),
        "unexpected action_selected: {}",
        ledger.action_selected
    );
}

#[test]
fn pre_first_frame_remote_terminal_waits_for_receive_recovery() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Home));
    let commands = harness.apply(
        2_000.0,
        ConnectionLifecycleStateFact::Connected,
        "receiverWaitingKeyframe",
        0,
        |stats| {
            seed_pre_first_frame_acquisition_stats(stats, "receiverWaitingKeyframe", 2_000.0);
            stats.receive_picture_recovery_terminal_total = 63;
            stats.receive_keyframe_required = Some(true);
            stats.receive_keyframe_response_state = Some("no-packet".to_string());
            stats.receive_display_state = Some("none".to_string());
            stats.reference_chain_state = Some("need-keyframe".to_string());
            stats.receive_keyframe_sent_count_unresolved = 7;
        },
    );

    assert!(
        commands
            .iter()
            .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })),
        "pre-first-frame receiverWaitingKeyframe should stay in receive recovery, commands={commands:?}"
    );
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert!(
            matches!(
                ledger.action_selected.as_str(),
                "delegatedToReceive" | "cooldownSuppressed" | "coalesced:keyframeInFlight"
            ),
            "unexpected action_selected: {}",
            ledger.action_selected
        );
    });
}

#[test]
fn latched_remote_terminal_home_recovery_stages_reconnect_candidate() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Home));
    let commands = harness.apply(
        2_000.0,
        ConnectionLifecycleStateFact::Connected,
        "receiverWaitingKeyframe",
        1_016,
        |stats| {
            stats.session_phase = Some("recovering".to_string());
            stats.video_owner_state = Some("supply-starved".to_string());
            stats.video_owner_reason = Some("supplyStarved".to_string());
            stats.receive_picture_recovery_terminal_total = 63;
            stats.receive_keyframe_required = Some(true);
            stats.receive_keyframe_response_state = Some("no-packet".to_string());
            stats.receive_display_state = Some("none".to_string());
            stats.reference_chain_state = Some("need-keyframe".to_string());
            stats.receive_keyframe_sent_count_unresolved = 7;
            stats.recovery_displayed_idr_at_ms = Some(100.0);
            stats.recovery_fresh_anchor_recovered_at_ms = Some(100.0);
            stats.latest_video_host_present_time_ms = Some(270.0);
            stats.latest_video_decode_ok_time_ms = Some(250.0);
            stats.latest_video_twcc_observation = Some(stable_video_twcc_observation(63, 950.0));
            stats.first_video_packet_arrival_time_ms = Some(100.0);
            stats.inbound_primary_video_bytes_total = 54_232_076;
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 54_232_076,
                video_packet_count_total: 47_253,
                audio_bytes_total: 181_063,
                observed_at_ms: 2_000.0,
            });
        },
    );

    assert!(
        commands
            .iter()
            .any(|command| matches!(command, TransportCommand::RequestReconnectCandidate { .. })),
        "remote terminal should stage reconnect immediately, commands={commands:?}"
    );
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert!(
            ledger.input_signal.ends_with(":receiverWaitingKeyframe"),
            "unexpected input signal: {}",
            ledger.input_signal
        );
        assert_eq!(ledger.action_selected, "requestReconnectCandidate");
        assert_eq!(ledger.escalation_basis.as_deref(), Some("anchor_missing"));
    });
}

#[test]
fn pre_first_frame_bootstrap_missing_sps_records_local_keyframe_probe_in_ledger() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        seed_pre_first_frame_acquisition_stats(&mut stats, "bootstrapMissingSps", 100.0);
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
    assert!(
        commands.iter().all(|command| !matches!(
            command,
            TransportCommand::RequestPli { .. } | TransportCommand::RequestFir { .. }
        )),
        "PLI/FIR 由接收侧本地 picture recovery 执行，不经 session transport command: {commands:?}"
    );

    let stats = runtime_stats.lock().expect("runtime stats lock");
    let ledger = stats
        .latest_recovery_decision_ledger
        .as_ref()
        .expect("recovery decision ledger");
    assert_eq!(
        ledger.input_signal,
        "receiverWaitingKeyframe:bootstrapMissingSps"
    );
    assert!(
        matches!(
            ledger.action_selected.as_str(),
            "cooldownSuppressed" | "coalesced:keyframeInFlight"
        ),
        "unexpected action_selected: {}",
        ledger.action_selected
    );
}

#[test]
fn pre_first_frame_bootstrap_missing_sps_with_recent_episode_coalesces_probe() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        seed_pre_first_frame_acquisition_stats(&mut stats, "bootstrapMissingSps", 100.0);
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
                retired_at_ms: None,
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
        commands.iter().all(|command| !matches!(
            command,
            TransportCommand::RequestPli { .. } | TransportCommand::RequestFir { .. }
        )),
        "recent first-frame keyframe episode should stay coalesced locally: {commands:?}"
    );

    let stats = runtime_stats.lock().expect("runtime stats lock");
    let ledger = stats
        .latest_recovery_decision_ledger
        .as_ref()
        .expect("recovery decision ledger");
    assert_eq!(
        ledger.input_signal,
        "receiverWaitingKeyframe:bootstrapMissingSps"
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
            latest_diagnosis_label: Some("receiverWaitingKeyframe".to_string()),
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
fn connected_healthy_transport_await_reconnect_is_blocked_after_success_edge() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    policy.last_successful_media_edge_at_ms = Some(1_000.0);

    if let Ok(mut stats) = runtime_stats.lock() {
        stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
        stats.session_phase = Some("steady".to_string());
        stats.video_decoder_stalled = Some(false);
        stats.receive_display_state = Some("none".to_string());
        stats.reference_chain_state = Some("need-keyframe".to_string());
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 1,
            source_event: "frame-await-recovery-anchor".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "waiting-keyframe".to_string(),
                reason: Some("receiverWaitingKeyframe".to_string()),
                chain_break_evidence: None,
                observed_at_ms: 1_950.0,
            },
            observed_at_ms: 1_950.0,
        });
    }

    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
    connection.control_channel_open = true;
    connection.latest_transport_path = Some("Direct".to_string());
    connection.latest_rtt_ms = Some(12.0);
    connection.latest_loss_ratio_1s = Some(0.0);
    connection.ice_has_selected_or_nominated_pair = true;
    connection.last_observed_at_ms = Some(2_000.0);
    let snapshot = TransportSnapshot::new(
        1,
        2_000.0,
        connection,
        MediaProjection {
            frame_count: 128,
            ..MediaProjection::default()
        },
        RecoveryProjection {
            latest_diagnosis_label: Some("receiverWaitingKeyframe".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(2_000.0),
            ..Default::default()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );

    let block_reason = policy.media_reconnect_block_reason(
        &snapshot,
        VideoSchedulingOwnerState::RebuildingSupply,
        &reconnect_candidate_proposal(),
        &transport_await_owner_signal(2_000.0),
        2_000.0,
    );

    assert_eq!(
        block_reason,
        Some("mediaGate:connectedHealthyTransportAwait")
    );
}

#[test]
fn connected_healthy_lifecycle_liveness_reconnect_is_blocked() {
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));

    if let Ok(mut stats) = runtime_stats.lock() {
        stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
        stats.session_phase = Some("steady".to_string());
        stats.video_decoder_stalled = Some(false);
        stats.receive_display_state = Some("display-stable".to_string());
        stats.reference_chain_state = Some("continuous".to_string());
        stats.latest_video_decode_ok_time_ms = Some(1_980.0);
        stats.latest_video_host_present_time_ms = Some(1_980.0);
        stats.latest_video_twcc_observation = Some(stable_video_twcc_observation(71, 1_950.0));
    }

    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
    connection.last_observed_at_ms = Some(2_000.0);
    let snapshot = TransportSnapshot::new(
        1,
        2_000.0,
        connection,
        MediaProjection {
            frame_count: 128,
            ..MediaProjection::default()
        },
        RecoveryProjection {
            latest_diagnosis_label: Some("livenessNoProgressTimeout".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(2_000.0),
            ..Default::default()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let mut proposal = reconnect_candidate_proposal();

    let resolution = ExpensiveRecoveryGate::new(runtime_stats.as_ref(), false, None, None, 0)
        .apply_to_proposal(
            &snapshot,
            VideoSchedulingOwnerState::RebuildingSupply,
            &mut proposal,
            &lifecycle_liveness_owner_signal(2_000.0),
            2_000.0,
            TwccWarmupState::Inactive,
            false,
        );

    assert_eq!(proposal.decision.action, RecoveryAction::CooldownSuppressed);
    assert_eq!(
        resolution.detail.as_deref(),
        Some("reconnectBlocked:lifecycleGate:connectedHealthyNoProgress")
    );
    let policy_proposal = RecoveryPolicyProposal {
        decision: proposal.decision,
        reason: VideoEscalationReason::LifecycleRecovering,
        reason_label: "livenessNoProgressTimeout".to_string(),
        reason_domain: crate::XbxEngineRecoveryReasonDomain::ConnectivityTransport,
        reason_domain_before_runtime_resolution: None,
        reason_domain_after_runtime_resolution: None,
        remote_terminal_domain_promoted: false,
        remote_terminal_active: false,
        reconnect_gate_detail: resolution.detail,
        budget_before: proposal.budget_before,
        budget_after: proposal.budget_after,
        coalescing_mode: proposal.coalescing_mode,
        unlock_reason: proposal.unlock_reason,
        preempt_reason: proposal.preempt_reason,
    };
    assert_eq!(
        policy_proposal.ledger_gate_result(None, false),
        "suppressed:reconnectBlocked:lifecycleGate:connectedHealthyNoProgress"
    );
}

#[test]
fn lifecycle_liveness_awaits_success_edge_after_reconnect_grant() {
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Connecting;
    connection.last_observed_at_ms = Some(2_000.0);
    let snapshot = TransportSnapshot::new(
        1,
        2_000.0,
        connection,
        MediaProjection {
            frame_count: 128,
            ..MediaProjection::default()
        },
        RecoveryProjection {
            latest_diagnosis_label: Some("livenessNoProgressTimeout".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(2_000.0),
            ..Default::default()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let mut proposal = reconnect_candidate_proposal();

    let resolution = ExpensiveRecoveryGate::new(
        runtime_stats.as_ref(),
        false,
        Some(1_000.0),
        Some(1_000.0),
        1,
    )
    .apply_to_proposal(
        &snapshot,
        VideoSchedulingOwnerState::RebuildingSupply,
        &mut proposal,
        &lifecycle_liveness_owner_signal(2_000.0),
        2_000.0,
        TwccWarmupState::Inactive,
        false,
    );

    assert_eq!(proposal.decision.action, RecoveryAction::CooldownSuppressed);
    assert_eq!(
        resolution.detail.as_deref(),
        Some("reconnectBlocked:lifecycleGate:awaitSuccessEdge")
    );
}

#[test]
fn lifecycle_liveness_blocks_reconnect_while_transport_rebuild_is_in_flight() {
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Connecting;
    connection.last_observed_at_ms = Some(2_000.0);
    let snapshot = TransportSnapshot::new(
        1,
        2_000.0,
        connection,
        MediaProjection {
            frame_count: 128,
            ..MediaProjection::default()
        },
        RecoveryProjection {
            latest_diagnosis_label: Some("livenessNoProgressTimeout".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(2_000.0),
            ..Default::default()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let mut proposal = reconnect_candidate_proposal();

    let resolution =
        ExpensiveRecoveryGate::new(runtime_stats.as_ref(), false, None, Some(1_000.0), 1)
            .apply_to_proposal(
                &snapshot,
                VideoSchedulingOwnerState::RebuildingSupply,
                &mut proposal,
                &lifecycle_liveness_owner_signal(2_000.0),
                2_000.0,
                TwccWarmupState::Inactive,
                false,
            );

    assert_eq!(proposal.decision.action, RecoveryAction::CooldownSuppressed);
    assert_eq!(
        resolution.detail.as_deref(),
        Some("reconnectBlocked:lifecycleGate:transportRebuildInFlightNoProgress")
    );
}

#[test]
fn lifecycle_liveness_direct_ice_no_response_still_reconnects() {
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Connecting;
    connection.ice_direct_checks_without_response = true;
    connection.ice_has_selected_or_nominated_pair = false;
    connection.ice_max_requests_sent = 8;
    connection.ice_responses_received_total = 0;
    connection.ice_probe_observed_at_ms = Some(2_000.0);
    connection.last_observed_at_ms = Some(2_000.0);
    let snapshot = TransportSnapshot::new(
        1,
        2_000.0,
        connection,
        MediaProjection {
            frame_count: 0,
            ..MediaProjection::default()
        },
        RecoveryProjection {
            latest_diagnosis_label: Some("livenessNoProgressTimeout".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(2_000.0),
            ..Default::default()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let mut proposal = reconnect_candidate_proposal();

    let resolution = ExpensiveRecoveryGate::new(runtime_stats.as_ref(), false, None, None, 0)
        .apply_to_proposal(
            &snapshot,
            VideoSchedulingOwnerState::RebuildingSupply,
            &mut proposal,
            &lifecycle_liveness_owner_signal(2_000.0),
            2_000.0,
            TwccWarmupState::Inactive,
            false,
        );

    assert_eq!(
        proposal.decision.action,
        RecoveryAction::RequestReconnectCandidate
    );
    assert_eq!(
        resolution.detail.as_deref(),
        Some("reconnectGranted:connectivityEvidence")
    );
}

#[test]
fn connected_healthy_twcc_transport_await_reconnect_is_blocked_without_path_projection() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());

    if let Ok(mut stats) = runtime_stats.lock() {
        stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
        stats.session_phase = Some("steady".to_string());
        stats.video_decoder_stalled = Some(false);
        stats.receive_display_state = Some("display-stable".to_string());
        stats.reference_chain_state = Some("continuous".to_string());
        stats.latest_video_decode_ok_time_ms = Some(1_980.0);
        stats.latest_video_host_present_time_ms = Some(1_980.0);
        stats.latest_video_twcc_observation = Some(stable_video_twcc_observation(7, 1_950.0));
    }

    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
    connection.last_observed_at_ms = Some(2_000.0);
    let snapshot = TransportSnapshot::new(
        1,
        2_000.0,
        connection,
        MediaProjection {
            frame_count: 128,
            ..MediaProjection::default()
        },
        RecoveryProjection {
            latest_diagnosis_label: Some("receiverWaitingKeyframe".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(2_000.0),
            ..Default::default()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );

    let block_reason = policy.media_reconnect_block_reason(
        &snapshot,
        VideoSchedulingOwnerState::RebuildingSupply,
        &reconnect_candidate_proposal(),
        &transport_await_owner_signal(2_000.0),
        2_000.0,
    );

    assert_eq!(
        block_reason,
        Some("mediaGate:connectedHealthyTransportAwait")
    );
}

#[test]
fn connected_healthy_twcc_reconnect_block_reports_traceable_detail() {
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));

    if let Ok(mut stats) = runtime_stats.lock() {
        stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
        stats.session_phase = Some("steady".to_string());
        stats.video_decoder_stalled = Some(false);
        stats.receive_display_state = Some("display-stable".to_string());
        stats.reference_chain_state = Some("continuous".to_string());
        stats.latest_video_decode_ok_time_ms = Some(1_980.0);
        stats.latest_video_host_present_time_ms = Some(1_980.0);
        stats.latest_video_twcc_observation = Some(stable_video_twcc_observation(11, 1_950.0));
    }

    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
    connection.last_observed_at_ms = Some(2_000.0);
    let snapshot = TransportSnapshot::new(
        1,
        2_000.0,
        connection,
        MediaProjection {
            frame_count: 128,
            ..MediaProjection::default()
        },
        RecoveryProjection {
            latest_diagnosis_label: Some("receiverWaitingKeyframe".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(2_000.0),
            ..Default::default()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let mut proposal = reconnect_candidate_proposal();

    let resolution = ExpensiveRecoveryGate::new(runtime_stats.as_ref(), false, None, None, 0)
        .apply_to_proposal(
            &snapshot,
            VideoSchedulingOwnerState::RebuildingSupply,
            &mut proposal,
            &transport_await_owner_signal(2_000.0),
            2_000.0,
            TwccWarmupState::Inactive,
            false,
        );

    assert_eq!(proposal.decision.action, RecoveryAction::CooldownSuppressed);
    assert_eq!(
        resolution.detail.as_deref(),
        Some("reconnectBlocked:mediaGate:connectedHealthyTransportAwait")
    );
    let policy_proposal = RecoveryPolicyProposal {
        decision: proposal.decision,
        reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
        reason_label: "receiverWaitingKeyframe".to_string(),
        reason_domain: crate::XbxEngineRecoveryReasonDomain::ConnectivityTransport,
        reason_domain_before_runtime_resolution: None,
        reason_domain_after_runtime_resolution: None,
        remote_terminal_domain_promoted: false,
        remote_terminal_active: false,
        reconnect_gate_detail: resolution.detail,
        budget_before: proposal.budget_before,
        budget_after: proposal.budget_after,
        coalescing_mode: proposal.coalescing_mode,
        unlock_reason: proposal.unlock_reason,
        preempt_reason: proposal.preempt_reason,
    };
    assert_eq!(
        policy_proposal.ledger_gate_result(None, false),
        "suppressed:reconnectBlocked:mediaGate:connectedHealthyTransportAwait"
    );
}

#[test]
fn trace_like_repairing_reference_chain_without_fresh_output_does_not_claim_healthy_playback() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());

    if let Ok(mut stats) = runtime_stats.lock() {
        stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
        stats.session_phase = Some("steady".to_string());
        stats.video_decoder_stalled = Some(false);
        stats.receive_display_state = None;
        stats.reference_chain_state = Some("repairing".to_string());
        stats.latest_video_twcc_observation = Some(stable_video_twcc_observation(570, 1_592.0));
    }

    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
    connection.last_observed_at_ms = Some(2_324.0);
    let snapshot = TransportSnapshot::new(
        1,
        2_324.0,
        connection,
        MediaProjection {
            frame_count: 128,
            ..MediaProjection::default()
        },
        RecoveryProjection {
            latest_diagnosis_label: Some("receiverWaitingKeyframe".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(2_324.0),
            ..Default::default()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );

    let block_reason = policy.media_reconnect_block_reason(
        &snapshot,
        VideoSchedulingOwnerState::SupplyStarved,
        &reconnect_candidate_proposal(),
        &transport_await_owner_signal(2_324.0),
        2_324.0,
    );

    assert_eq!(block_reason, Some("mediaGate:missingHardEvidence"));
}

#[test]
fn stale_twcc_does_not_make_transport_await_connection_healthy() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());

    if let Ok(mut stats) = runtime_stats.lock() {
        stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
        stats.session_phase = Some("steady".to_string());
        stats.video_decoder_stalled = Some(false);
        stats.receive_display_state = Some("display-stable".to_string());
        stats.reference_chain_state = Some("continuous".to_string());
        stats.latest_video_twcc_observation = Some(stable_video_twcc_observation(9, 1_000.0));
        seed_transport_await_hard_evidence(&mut stats, 4_050.0);
    }

    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
    connection.last_observed_at_ms = Some(4_100.0);
    let snapshot = TransportSnapshot::new(
        1,
        4_100.0,
        connection,
        MediaProjection {
            frame_count: 128,
            ..MediaProjection::default()
        },
        RecoveryProjection {
            latest_diagnosis_label: Some("receiverWaitingKeyframe".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(4_100.0),
            ..Default::default()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );

    let block_reason = policy.media_reconnect_block_reason(
        &snapshot,
        VideoSchedulingOwnerState::RebuildingSupply,
        &reconnect_candidate_proposal(),
        &transport_await_owner_signal(4_100.0),
        4_100.0,
    );

    assert_eq!(block_reason, None);
}

#[test]
fn twcc_health_gate_requires_valid_twcc_and_only_fresh_ice_failure_blocks_it() {
    let mut cases = Vec::new();

    let mut invalid = stable_video_twcc_observation(20, 4_050.0);
    invalid.twcc_sample_valid = false;
    cases.push(("invalid-sample", invalid, None, None));

    let mut delayed = stable_video_twcc_observation(21, 4_050.0);
    delayed.quality = crate::XbxEngineTwccObservationQuality::Delayed;
    cases.push(("delayed-quality", delayed, None, None));

    let mut low_delivery = stable_video_twcc_observation(22, 4_050.0);
    low_delivery.delivery_ratio = 0.90;
    cases.push(("low-delivery", low_delivery, None, None));

    let mut high_loss = stable_video_twcc_observation(23, 4_050.0);
    high_loss.packet_loss_ratio = 0.08;
    cases.push(("high-loss", high_loss, None, None));

    let mut remote_source = stable_video_twcc_observation(24, 4_050.0);
    remote_source.source = "remote-observed".to_string();
    cases.push(("remote-source", remote_source, None, None));

    cases.push((
        "fresh-ice-direct-no-response",
        stable_video_twcc_observation(25, 4_050.0),
        Some(crate::XbxEngineIceConnectivityProbeObservation {
            candidate_pair_count: 1,
            nominated_pair_count: 0,
            succeeded_pair_count: 0,
            in_progress_pair_count: 1,
            failed_pair_count: 0,
            max_requests_sent: 12,
            max_responses_received: 0,
            responses_received_total: 0,
            has_selected_or_nominated_pair: false,
            direct_checks_without_response: true,
            local_candidate_type_summary: "unknown=1".to_string(),
            remote_candidate_type_summary: "unknown=1".to_string(),
            address_family_summary: "unknown=1".to_string(),
            observed_at_ms: 4_050.0,
        }),
        None,
    ));

    cases.push((
        "stale-ice-direct-no-response",
        stable_video_twcc_observation(26, 4_050.0),
        Some(crate::XbxEngineIceConnectivityProbeObservation {
            candidate_pair_count: 1,
            nominated_pair_count: 0,
            succeeded_pair_count: 0,
            in_progress_pair_count: 1,
            failed_pair_count: 0,
            max_requests_sent: 12,
            max_responses_received: 0,
            responses_received_total: 0,
            has_selected_or_nominated_pair: false,
            direct_checks_without_response: true,
            local_candidate_type_summary: "unknown=1".to_string(),
            remote_candidate_type_summary: "unknown=1".to_string(),
            address_family_summary: "unknown=1".to_string(),
            observed_at_ms: 500.0,
        }),
        Some("mediaGate:connectedHealthyTransportAwait"),
    ));

    for (name, twcc, ice_probe, expected_block_reason) in cases {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());

        if let Ok(mut stats) = runtime_stats.lock() {
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.session_phase = Some("steady".to_string());
            stats.video_decoder_stalled = Some(false);
            stats.receive_display_state = Some("display-stable".to_string());
            stats.reference_chain_state = Some("continuous".to_string());
            if expected_block_reason == Some("mediaGate:connectedHealthyTransportAwait") {
                stats.latest_video_decode_ok_time_ms = Some(4_080.0);
                stats.latest_video_host_present_time_ms = Some(4_080.0);
            }
            stats.latest_video_twcc_observation = Some(twcc);
            stats.latest_ice_connectivity_probe = ice_probe;
            seed_transport_await_hard_evidence(&mut stats, 4_050.0);
        }

        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
        connection.last_observed_at_ms = Some(4_100.0);
        let snapshot = TransportSnapshot::new(
            1,
            4_100.0,
            connection,
            MediaProjection {
                frame_count: 128,
                ..MediaProjection::default()
            },
            RecoveryProjection {
                latest_diagnosis_label: Some("receiverWaitingKeyframe".to_string()),
                pending_action: false,
                successful_action_count: 0,
                failed_action_count: 0,
                last_observed_at_ms: Some(4_100.0),
                ..Default::default()
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );

        let block_reason = policy.media_reconnect_block_reason(
            &snapshot,
            VideoSchedulingOwnerState::RebuildingSupply,
            &reconnect_candidate_proposal(),
            &transport_await_owner_signal(4_100.0),
            4_100.0,
        );

        assert_eq!(block_reason, expected_block_reason, "{name}");
    }
}

#[test]
fn connected_healthy_twcc_receiver_waiting_keyframe_does_not_emit_reconnect_command() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());

    if let Ok(mut stats) = runtime_stats.lock() {
        stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
        stats.session_phase = Some("steady".to_string());
        stats.recovery_active_escalation_reason = Some("receiverWaitingKeyframe".to_string());
        stats.video_owner_state = Some("supply-starved".to_string());
        stats.video_owner_reason = Some("receiverWaitingKeyframe".to_string());
        stats.video_decoder_stalled = Some(false);
        stats.receive_display_state = Some("display-stable".to_string());
        stats.reference_chain_state = Some("continuous".to_string());
        stats.receive_keyframe_response_state = Some("usable-idr".to_string());
        stats.receive_keyframe_required = Some(false);
        stats.latest_video_decode_ok_time_ms = Some(1_980.0);
        stats.latest_video_host_present_time_ms = Some(1_980.0);
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 9,
            source_event: "frame-await-recovery-anchor".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "waiting-keyframe".to_string(),
                reason: Some("receiverWaitingKeyframe".to_string()),
                chain_break_evidence: None,
                observed_at_ms: 1_950.0,
            },
            observed_at_ms: 1_950.0,
        });
        stats.latest_video_twcc_observation = Some(stable_video_twcc_observation(8, 1_950.0));
    }

    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
    connection.last_observed_at_ms = Some(2_000.0);
    let snapshot = TransportSnapshot::new(
        1,
        2_000.0,
        connection,
        MediaProjection {
            frame_count: 128,
            ..MediaProjection::default()
        },
        RecoveryProjection {
            latest_diagnosis_label: Some("receiverWaitingKeyframe".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(2_000.0),
            ..Default::default()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );

    let commands = transport_commands(policy.on_snapshot(&snapshot));
    assert!(
        commands
            .iter()
            .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })),
        "healthy TWCC transport-await should stay local, commands={commands:?}"
    );
    let stats = runtime_stats.lock().expect("runtime stats lock");
    let ledger = stats
        .latest_recovery_decision_ledger
        .as_ref()
        .expect("recovery decision ledger");
    assert_eq!(ledger.action_selected, "cooldownSuppressed");
    assert_ne!(ledger.gate_result, "pass:requestReconnect");
}

#[test]
fn disconnected_runtime_state_ignores_fresh_twcc_health_for_reconnect_block() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());

    if let Ok(mut stats) = runtime_stats.lock() {
        stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Disconnected;
        stats.session_phase = Some("steady".to_string());
        stats.video_decoder_stalled = Some(false);
        stats.receive_display_state = Some("display-stable".to_string());
        stats.reference_chain_state = Some("continuous".to_string());
        stats.latest_video_twcc_observation = Some(stable_video_twcc_observation(31, 1_950.0));
        seed_transport_await_hard_evidence(&mut stats, 1_950.0);
    }

    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
    connection.last_observed_at_ms = Some(2_000.0);
    let snapshot = TransportSnapshot::new(
        1,
        2_000.0,
        connection,
        MediaProjection {
            frame_count: 128,
            ..MediaProjection::default()
        },
        RecoveryProjection {
            latest_diagnosis_label: Some("receiverWaitingKeyframe".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(2_000.0),
            ..Default::default()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );

    let block_reason = policy.media_reconnect_block_reason(
        &snapshot,
        VideoSchedulingOwnerState::RebuildingSupply,
        &reconnect_candidate_proposal(),
        &transport_await_owner_signal(2_000.0),
        2_000.0,
    );

    assert_eq!(block_reason, None);
}

#[test]
fn remote_terminal_transport_await_can_reconnect_even_on_healthy_transport() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());

    if let Ok(mut stats) = runtime_stats.lock() {
        stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
        stats.receive_picture_recovery_terminal_total = 63;
        stats.receive_keyframe_required = Some(true);
        stats.receive_keyframe_response_state = Some("no-packet".to_string());
        stats.receive_display_state = Some("none".to_string());
        stats.reference_chain_state = Some("need-keyframe".to_string());
        stats.receive_keyframe_sent_count_unresolved = 7;
        stats.latest_video_twcc_observation = Some(stable_video_twcc_observation(64, 1_950.0));
    }

    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
    connection.control_channel_open = true;
    connection.latest_transport_path = Some("Direct".to_string());
    connection.latest_rtt_ms = Some(12.0);
    connection.latest_loss_ratio_1s = Some(0.0);
    connection.ice_has_selected_or_nominated_pair = true;
    connection.last_observed_at_ms = Some(2_000.0);
    let snapshot = TransportSnapshot::new(
        1,
        2_000.0,
        connection,
        MediaProjection {
            frame_count: 128,
            ..MediaProjection::default()
        },
        RecoveryProjection {
            latest_diagnosis_label: Some("receiverWaitingKeyframe".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(2_000.0),
            ..Default::default()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );

    let block_reason = policy.media_reconnect_block_reason(
        &snapshot,
        VideoSchedulingOwnerState::RebuildingSupply,
        &reconnect_candidate_proposal(),
        &transport_await_owner_signal(2_000.0),
        2_000.0,
    );

    assert_eq!(block_reason, None);
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
            source_event: "frame-await-recovery-anchor".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "waiting-keyframe".to_string(),
                reason: Some("receiverWaitingKeyframe".to_string()),
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
fn successful_media_edge_resets_liveness_no_progress_timer() {
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
        .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })));

    {
        let mut stats = runtime_stats.lock().expect("runtime stats lock");
        stats.latest_video_host_present_time_ms = Some(4_100.0);
    }
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
        second_commands
            .iter()
            .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })),
        "fresh host present edge should reset liveness no-progress timer, commands={second_commands:?}"
    );

    let third = TransportSnapshot::new(
        3,
        8_401.0,
        connection,
        media,
        RecoveryProjection {
            last_observed_at_ms: Some(8_401.0),
            ..recovery
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let third_commands = transport_commands(policy.on_snapshot(&third));
    assert!(
        third_commands
            .iter()
            .any(|command| matches!(command, TransportCommand::RequestReconnectCandidate { .. })),
        "without a later media edge, liveness should still reconnect after the bounded window"
    );
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
fn pre_first_frame_ingress_progress_resets_liveness_timer() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Home);
        stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
        stats.inbound_primary_video_bytes_total = 1_000;
        stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
            state: "remoteTrackAttached".to_string(),
            video_width: None,
            video_height: None,
            mime_type: Some("video/H264".to_string()),
            transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
            video_bytes_total: 1_000,
            video_packet_count_total: 8,
            audio_bytes_total: 0,
            observed_at_ms: 100.0,
        });
    }
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Recovering;
    connection.latest_transport_path = Some("relay/udp".to_string());
    connection.latest_rtt_ms = Some(18.0);
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

    if let Ok(mut stats) = runtime_stats.lock() {
        stats.inbound_primary_video_bytes_total = 2_000;
        if let Some(track) = stats.latest_video_track_status.as_mut() {
            track.video_bytes_total = 2_000;
            track.video_packet_count_total = 16;
            track.observed_at_ms = 10_000.0;
        }
    }
    let second = TransportSnapshot::new(
        2,
        10_000.0,
        connection.clone(),
        media.clone(),
        RecoveryProjection {
            last_observed_at_ms: Some(10_000.0),
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
        "首帧前 RTP/track 仍在推进时，liveness 计时应被重置"
    );

    let third = TransportSnapshot::new(
        3,
        24_000.0,
        connection.clone(),
        media.clone(),
        RecoveryProjection {
            last_observed_at_ms: Some(24_000.0),
            ..recovery.clone()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let third_commands = transport_commands(policy.on_snapshot(&third));
    assert!(
        third_commands.iter().all(|command| {
            !matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }),
        "距最近 ingress 进展未超过首帧前 fallback 窗口时，不应触发 reconnect"
    );

    let fourth = TransportSnapshot::new(
        4,
        25_100.0,
        connection,
        media,
        RecoveryProjection {
            last_observed_at_ms: Some(25_100.0),
            ..recovery
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let fourth_commands = transport_commands(policy.on_snapshot(&fourth));
    assert!(fourth_commands
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
                        state: "receiving".to_string(),
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
