use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use crate::api::runtime::XbxEngineRuntimeConfig;
use crate::media::video::decode::actor::{DecodeActorHandle, DecodeMsg};
use crate::transport::rtc::connection::RtcConnectionService;
use crate::transport::rtc::facts::{CommandResultStatus, SessionCommand, TransportCommand};
use crate::transport::rtc::recovery::escalation::RecoveryAction;
use crate::transport::rtc::session::actor::SessionActor;
use crate::transport::rtc::session::clock::SystemSessionClock;
use crate::transport::rtc::session::policy::RtcSessionPolicy;
use crate::transport::rtc::stream::RtcMediaService;
use crate::{
    XbxEngineMediaRuntimeStats, XbxEnginePendingRuntimeRecoveryAction,
    XbxEngineVideoEscalationObservation,
};

use super::RtcTransportSessionBridge;

fn build_bridge(
    runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    pending_runtime_recovery_action: Arc<Mutex<Option<XbxEnginePendingRuntimeRecoveryAction>>>,
) -> RtcTransportSessionBridge<'static> {
    build_bridge_with_local_decoder_reset_handle(
        runtime_stats,
        pending_runtime_recovery_action,
        Arc::new(Mutex::new(None)),
    )
}

fn build_bridge_with_local_decoder_reset_handle(
    runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    pending_runtime_recovery_action: Arc<Mutex<Option<XbxEnginePendingRuntimeRecoveryAction>>>,
    local_decoder_reset_handle: Arc<Mutex<Option<Arc<DecodeActorHandle>>>>,
) -> RtcTransportSessionBridge<'static> {
    let runtime_stats = Box::leak(Box::new(runtime_stats));
    let runtime_config = Box::leak(Box::new(Arc::new(Mutex::new(
        XbxEngineRuntimeConfig::default(),
    ))));
    let pending_runtime_recovery_action = Box::leak(Box::new(pending_runtime_recovery_action));
    let connection = Box::leak(Box::new(Arc::new(Mutex::new(
        RtcConnectionService::default(),
    ))));
    let media = Box::leak(Box::new(Arc::new(Mutex::new(RtcMediaService::default()))));
    let local_decoder_reset_handle = Box::leak(Box::new(local_decoder_reset_handle));
    let transport_session = Box::leak(Box::new(Arc::new(Mutex::new(SessionActor::new(
        SystemSessionClock,
        RtcSessionPolicy::new(runtime_config.clone(), runtime_stats.clone()),
    )))));
    let transport_fact_sink = Box::leak(Box::new(Arc::new(Mutex::new(Vec::new()))));

    RtcTransportSessionBridge::new(
        runtime_stats,
        runtime_config,
        pending_runtime_recovery_action,
        connection,
        media,
        local_decoder_reset_handle,
        transport_session,
        transport_fact_sink,
    )
}

#[test]
fn queue_local_decoder_reset_marks_skipped_when_handle_missing() {
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
    let bridge = build_bridge(runtime_stats.clone(), pending_runtime_recovery_action);

    assert!(!bridge.queue_local_decoder_reset("recoveryCommand:test".to_string(), 321.0,));

    let snapshot = runtime_stats.lock().expect("runtime stats lock");
    assert_eq!(
        snapshot.latest_observation_label.as_deref(),
        Some("videoDecoderLocalResetSkipped")
    );
    assert!(snapshot
        .latest_observation_summary
        .as_deref()
        .is_some_and(|summary| summary.contains("source=recoveryCommand")));
}

#[test]
fn queue_local_decoder_reset_enqueues_message_when_handle_available() {
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
    let (tx, rx) = mpsc::sync_channel(1);
    let handle = Arc::new(DecodeActorHandle::from_test_sender(tx));
    let bridge = build_bridge_with_local_decoder_reset_handle(
        runtime_stats.clone(),
        pending_runtime_recovery_action,
        Arc::new(Mutex::new(Some(handle))),
    );

    assert!(bridge.queue_local_decoder_reset("recoveryCommand:test".to_string(), 654.0,));

    let snapshot = runtime_stats.lock().expect("runtime stats lock");
    assert_eq!(
        snapshot.latest_observation_label.as_deref(),
        Some("videoDecoderLocalResetQueued")
    );
    drop(snapshot);
    let msg = rx.recv().expect("local decoder reset message");
    match msg {
        DecodeMsg::LocalDecoderReset {
            reason,
            observed_at_ms,
        } => {
            assert_eq!(reason, "recoveryCommand:test");
            assert_eq!(observed_at_ms, 654.0);
        }
        _ => panic!("unexpected decode message"),
    }
}

#[test]
fn request_decoder_reset_command_enqueues_local_reset_without_connection_support() {
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
    let (tx, rx) = mpsc::sync_channel(1);
    let handle = Arc::new(DecodeActorHandle::from_test_sender(tx));
    let bridge = build_bridge_with_local_decoder_reset_handle(
        runtime_stats.clone(),
        pending_runtime_recovery_action,
        Arc::new(Mutex::new(Some(handle))),
    );

    bridge.apply_transport_session_command(SessionCommand::LocalDecoderReset {
        observation_id: 42,
        reason: "receiverWaitingKeyframe".to_string(),
    });

    let msg = rx.recv().expect("local decoder reset message");
    match msg {
        DecodeMsg::LocalDecoderReset { reason, .. } => {
            assert_eq!(reason, "recoveryCommand:receiverWaitingKeyframe");
        }
        _ => panic!("unexpected decode message"),
    }
    let snapshot = runtime_stats.lock().expect("runtime stats lock");
    assert_eq!(
        snapshot.latest_observation_label.as_deref(),
        Some("videoDecoderLocalResetQueued")
    );
}

#[test]
fn invalid_transport_await_response_releases_decoder_reset_family_gate() {
    let now_ms = crate::transport::rtc::stats::now_ms_f64();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_recovery_epoch = 5;
    stats.latest_video_decoder_reset_time_ms = Some(now_ms - 70.0);
    stats.latest_video_escalation_observation = Some(XbxEngineVideoEscalationObservation {
        observation_id: 41,
        reason: "receiverWaitingKeyframe".to_string(),
        action: "requestDecoderReset".to_string(),
        recovery_stage: "rebuilding-supply".to_string(),
        recovery_chain_value: "anchor".to_string(),
        recovery_failure_cost: "medium".to_string(),
        recovery_window_source: "hard-fallback-window".to_string(),
        observed_at_ms: now_ms - 60.0,
    });
    stats.latest_keyframe_request_episode =
        Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
            episode_id: 18,
            request_reason: Some("receiverWaitingKeyframe".to_string()),
            request_kind: Some("pli".to_string()),
            status: "decoded".to_string(),
            status_detail: None,
            requested_at_ms: now_ms - 260.0,
            sent_at_ms: Some(now_ms - 220.0),
            deadline_at_ms: Some(now_ms + 500.0),
            transport_detail: None,
            first_video_packet_at_ms: Some(now_ms - 55.0),
            first_video_packet_rtp_timestamp: Some(7_001),
            first_video_packet_is_keyframe: Some(false),
            first_keyframe_packet_at_ms: Some(now_ms - 55.0),
            first_keyframe_decoded_at_ms: Some(now_ms - 50.0),
            response_rtp_timestamp: Some(7_001),
            response_frame_seq: Some(88),
            response_verdict: Some("on-time".to_string()),
            lifecycle_phase: None,
            retired_at_ms: None,
        });
    stats.latest_h264_inspection_observation = Some(crate::XbxEngineH264InspectionObservation {
        observation_id: 77,
        frame_rtp_timestamp: Some(7_001),
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
        observed_at_ms: now_ms - 45.0,

        ..Default::default()
    });
    stats.latest_recovery_decision_ledger =
        Some(crate::XbxEngineRecoveryDecisionLedgerObservation {
            decision_id: 42,
            state_before: "recovering".to_string(),
            state_after: "recovering".to_string(),
            input_signal: "waitKeyframe:receiverWaitingKeyframe".to_string(),
            gate_result: "pass".to_string(),
            action_selected: "requestDecoderReset".to_string(),
            frame_value: None,
            gap_severity: None,
            repairability: None,
            recovery_episode_stage: None,
            recovery_episode_progress_at_ms: None,
            coalescing_mode: None,
            unlock_reason: None,
            preempt_reason: None,
            recovery_primary_action: None,
            owner_surface_state: None,
            anchor_evidence: None,
            keyframe_episode_health: None,
            escalation_basis: None,
            budget_before: None,
            budget_after: None,
            trigger_observation_label: None,
            trigger_observation_summary: None,
            command_result: None,
            command_detail: None,
            observed_at_ms: now_ms,
        });
    stats.recent_recovery_decision_ledgers =
        vec![stats.latest_recovery_decision_ledger.clone().unwrap()];

    let runtime_stats = Arc::new(Mutex::new(stats));
    let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
    let (tx, rx) = mpsc::sync_channel(1);
    let handle = Arc::new(DecodeActorHandle::from_test_sender(tx));
    let bridge = build_bridge_with_local_decoder_reset_handle(
        runtime_stats.clone(),
        pending_runtime_recovery_action,
        Arc::new(Mutex::new(Some(handle))),
    );

    bridge.apply_transport_session_command(SessionCommand::LocalDecoderReset {
        observation_id: 42,
        reason: "receiverWaitingKeyframe".to_string(),
    });

    let msg = rx.recv().expect("decoder reset should proceed");
    match msg {
        DecodeMsg::LocalDecoderReset { reason, .. } => {
            assert_eq!(reason, "recoveryCommand:receiverWaitingKeyframe");
        }
        _ => panic!("unexpected decode message"),
    }

    let snapshot = runtime_stats.lock().expect("runtime stats lock");
    let ledger = snapshot
        .latest_recovery_decision_ledger
        .as_ref()
        .expect("ledger");
    assert_eq!(
        ledger.unlock_reason.as_deref(),
        Some("bootstrapRejected:invalidBootstrap")
    );
    assert_eq!(ledger.command_result.as_deref(), Some("succeeded"));
    assert!(ledger
        .command_detail
        .as_deref()
        .is_some_and(|detail| { !detail.contains("sameFamilyCoalesced:decoderResetInFlight") }));
}

/// decoder reset 后新鲜 NonIdrVcl：episode 尚未进入 packet-seen/decoded 也应解除 decoderResetInFlight，
/// 避免同族合并无限压制后续 reset。
#[test]
fn invalid_transport_await_non_idr_vcl_unlocks_decoder_reset_gate_without_packet_seen_episode() {
    let now_ms = crate::transport::rtc::stats::now_ms_f64();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_recovery_epoch = 5;
    stats.latest_video_decoder_reset_time_ms = Some(now_ms - 70.0);
    stats.latest_video_escalation_observation = Some(XbxEngineVideoEscalationObservation {
        observation_id: 41,
        reason: "receiverWaitingKeyframe".to_string(),
        action: "requestDecoderReset".to_string(),
        recovery_stage: "rebuilding-supply".to_string(),
        recovery_chain_value: "anchor".to_string(),
        recovery_failure_cost: "medium".to_string(),
        recovery_window_source: "hard-fallback-window".to_string(),
        observed_at_ms: now_ms - 60.0,
    });
    stats.latest_keyframe_request_episode =
        Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
            episode_id: 19,
            request_reason: Some("receiverWaitingKeyframe".to_string()),
            request_kind: Some("pli".to_string()),
            status: "sent".to_string(),
            status_detail: None,
            requested_at_ms: now_ms - 260.0,
            sent_at_ms: Some(now_ms - 220.0),
            deadline_at_ms: Some(now_ms + 500.0),
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
    stats.latest_h264_inspection_observation = Some(crate::XbxEngineH264InspectionObservation {
        observation_id: 78,
        frame_rtp_timestamp: Some(7_002),
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
        observed_at_ms: now_ms - 45.0,

        ..Default::default()
    });
    stats.latest_recovery_decision_ledger =
        Some(crate::XbxEngineRecoveryDecisionLedgerObservation {
            decision_id: 43,
            state_before: "recovering".to_string(),
            state_after: "recovering".to_string(),
            input_signal: "waitKeyframe:receiverWaitingKeyframe".to_string(),
            gate_result: "pass".to_string(),
            action_selected: "requestDecoderReset".to_string(),
            frame_value: None,
            gap_severity: None,
            repairability: None,
            recovery_episode_stage: None,
            recovery_episode_progress_at_ms: None,
            coalescing_mode: None,
            unlock_reason: None,
            preempt_reason: None,
            recovery_primary_action: None,
            owner_surface_state: None,
            anchor_evidence: None,
            keyframe_episode_health: None,
            escalation_basis: None,
            budget_before: None,
            budget_after: None,
            trigger_observation_label: None,
            trigger_observation_summary: None,
            command_result: None,
            command_detail: None,
            observed_at_ms: now_ms,
        });
    stats.recent_recovery_decision_ledgers =
        vec![stats.latest_recovery_decision_ledger.clone().unwrap()];

    let runtime_stats = Arc::new(Mutex::new(stats));
    let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
    let (tx, rx) = mpsc::sync_channel(1);
    let handle = Arc::new(DecodeActorHandle::from_test_sender(tx));
    let bridge = build_bridge_with_local_decoder_reset_handle(
        runtime_stats.clone(),
        pending_runtime_recovery_action,
        Arc::new(Mutex::new(Some(handle))),
    );

    bridge.apply_transport_session_command(SessionCommand::LocalDecoderReset {
        observation_id: 43,
        reason: "receiverWaitingKeyframe".to_string(),
    });

    let msg = rx.recv().expect("decoder reset should proceed");
    match msg {
        DecodeMsg::LocalDecoderReset { reason, .. } => {
            assert_eq!(reason, "recoveryCommand:receiverWaitingKeyframe");
        }
        _ => panic!("unexpected decode message"),
    }

    let snapshot = runtime_stats.lock().expect("runtime stats lock");
    let ledger = snapshot
        .latest_recovery_decision_ledger
        .as_ref()
        .expect("ledger");
    assert!(
        ledger.unlock_reason.as_deref() == Some("decoderResetInvalidRecoveryResponse")
            || ledger.unlock_reason.as_deref() == Some("bootstrapRejected:invalidBootstrap"),
        "unexpected unlock_reason: {:?}",
        ledger.unlock_reason
    );
    assert_eq!(ledger.command_result.as_deref(), Some("succeeded"));
    assert!(ledger
        .command_detail
        .as_deref()
        .is_some_and(|detail| { !detail.contains("sameFamilyCoalesced:decoderResetInFlight") }));
}

#[test]
fn reconnect_candidate_records_escalation_observation_when_staged() {
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
    let bridge = build_bridge(runtime_stats.clone(), pending_runtime_recovery_action);

    bridge.apply_transport_session_command(SessionCommand::Transport(
        TransportCommand::RequestReconnectCandidate {
            observation_id: 42,
            reason: "recovering-stream".to_string(),
            reason_domain: crate::XbxEngineRecoveryReasonDomain::ConnectivityTransport,
        },
    ));

    let snapshot = runtime_stats.lock().expect("runtime stats lock");
    let escalation = snapshot
        .latest_video_escalation_observation
        .as_ref()
        .expect("escalation should be recorded");
    assert_eq!(escalation.observation_id, 42);
    assert_eq!(escalation.reason, "recovering-stream");
    assert_eq!(escalation.action, "requestReconnectCandidate");
    assert_eq!(
        snapshot.transport_recovery_epoch_at_last_escalation,
        snapshot.transport_recovery_epoch
    );
}

#[test]
fn reconnect_candidate_overwrites_pending_and_records_new_escalation() {
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let pending_runtime_recovery_action = Arc::new(Mutex::new(Some(
        XbxEnginePendingRuntimeRecoveryAction::RequestReconnectCandidate {
            observation_id: 1,
            reason: "existing".to_string(),
            reason_domain: crate::XbxEngineRecoveryReasonDomain::ConnectivityTransport,
        },
    )));
    let bridge = build_bridge(runtime_stats.clone(), pending_runtime_recovery_action);

    bridge.apply_transport_session_command(SessionCommand::Transport(
        TransportCommand::RequestReconnectCandidate {
            observation_id: 43,
            reason: "new-reason".to_string(),
            reason_domain: crate::XbxEngineRecoveryReasonDomain::ConnectivityTransport,
        },
    ));

    let snapshot = runtime_stats.lock().expect("runtime stats lock");
    let escalation = snapshot
        .latest_video_escalation_observation
        .as_ref()
        .expect("escalation should be recorded");
    assert_eq!(escalation.observation_id, 43);
    assert_eq!(escalation.reason, "new-reason");
    assert_eq!(
        snapshot.latest_observation_label.as_deref(),
        Some("rtcReconnectCandidateStaged")
    );
}

#[test]
fn reconnect_candidate_stage_preserves_reason_domain_in_pending_action() {
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
    let bridge = build_bridge(runtime_stats, pending_runtime_recovery_action.clone());

    bridge.apply_transport_session_command(SessionCommand::Transport(
        TransportCommand::RequestReconnectCandidate {
            observation_id: 44,
            reason: "displaySupplyCritical".to_string(),
            reason_domain: crate::XbxEngineRecoveryReasonDomain::Local,
        },
    ));

    let pending = pending_runtime_recovery_action
        .lock()
        .expect("pending reconnect action lock");
    assert!(matches!(
        pending.as_ref(),
        Some(XbxEnginePendingRuntimeRecoveryAction::RequestReconnectCandidate {
            observation_id: 44,
            reason,
            reason_domain: crate::XbxEngineRecoveryReasonDomain::Local,
        }) if reason == "displaySupplyCritical"
    ));
}

#[test]
fn reconnect_advances_recovery_epoch_by_contract() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_recovery_epoch = 2;
    let runtime_stats = Arc::new(Mutex::new(stats));
    let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
    let bridge = build_bridge(runtime_stats.clone(), pending_runtime_recovery_action);

    bridge.apply_transport_session_command(SessionCommand::Transport(
        TransportCommand::RequestReconnectCandidate {
            observation_id: 77,
            reason: "reconnect-needed".to_string(),
            reason_domain: crate::XbxEngineRecoveryReasonDomain::ConnectivityTransport,
        },
    ));

    let snapshot = runtime_stats.lock().expect("runtime stats lock");
    let escalation = snapshot
        .latest_video_escalation_observation
        .as_ref()
        .expect("escalation should be recorded");
    assert_eq!(escalation.action, "requestReconnectCandidate");
    assert_eq!(snapshot.transport_recovery_epoch, 3);
    assert_eq!(snapshot.transport_recovery_epoch_at_last_escalation, 3);
}

#[test]
fn transport_session_maps_local_decoder_reset_reason_to_non_advancing_epoch_policy() {
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
    let bridge = build_bridge(runtime_stats, pending_runtime_recovery_action);

    assert!(!bridge.should_advance_transport_recovery_epoch_on_success(
        RecoveryAction::RequestDecoderReset,
        "displaySupplyDegraded",
    ));
    assert!(!bridge.should_advance_transport_recovery_epoch_on_success(
        RecoveryAction::RequestDecoderReset,
        "receiverWaitingKeyframe",
    ));
}

#[test]
fn decoder_reset_is_deferred_when_control_reset_observation_is_recent() {
    let now_ms = crate::transport::rtc::stats::now_ms_f64();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_recovery_epoch = 7;
    stats.latest_video_escalation_observation = Some(XbxEngineVideoEscalationObservation {
        observation_id: 101,
        reason: "receiverWaitingKeyframe".to_string(),
        action: "requestDecoderReset".to_string(),
        recovery_stage: "rebuilding-supply".to_string(),
        recovery_chain_value: "anchor".to_string(),
        recovery_failure_cost: "high".to_string(),
        recovery_window_source: "hard-fallback-window".to_string(),
        observed_at_ms: now_ms,
    });
    let runtime_stats = Arc::new(Mutex::new(stats));
    let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
    let bridge = build_bridge(runtime_stats.clone(), pending_runtime_recovery_action);

    bridge.apply_transport_session_command(SessionCommand::LocalDecoderReset {
        observation_id: 202,
        reason: "receiverWaitingKeyframe".to_string(),
    });

    let snapshot = runtime_stats.lock().expect("runtime stats lock");
    assert_eq!(snapshot.transport_recovery_epoch, 7);
    assert_eq!(
        snapshot
            .latest_video_escalation_observation
            .as_ref()
            .map(|obs| obs.observation_id),
        Some(101)
    );
}

#[test]
fn receive_only_transport_defers_session_keyframe_commands() {
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
    crate::runtime_stats_sink::RuntimeStatsSink::new(runtime_stats.clone())
        .record_picture_recovery_episode_requested(
            1,
            Some("ingressWaitKeyframe".to_string()),
            100.0,
            None,
        );
    let bridge = build_bridge(runtime_stats.clone(), pending_runtime_recovery_action);

    bridge.apply_transport_session_command(SessionCommand::Transport(
        TransportCommand::RequestPli {
            observation_id: 1,
            reason: "ingressWaitKeyframe".to_string(),
        },
    ));

    let snapshot = runtime_stats.lock().expect("runtime stats lock");
    let episode = snapshot
        .latest_keyframe_request_episode
        .as_ref()
        .expect("episode");
    assert_eq!(episode.status, "deferred");
    assert_eq!(
        episode.response_verdict.as_deref(),
        Some("transportDeferred")
    );
}

#[test]
fn command_result_updates_matching_recovery_decision_ledger() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    let matching_ledger = crate::XbxEngineRecoveryDecisionLedgerObservation {
        decision_id: 88,
        state_before: "recovering".to_string(),
        state_after: "reconnecting".to_string(),
        input_signal: "liveness:livenessNoProgressTimeout".to_string(),
        gate_result: "pass".to_string(),
        action_selected: "requestReconnectCandidate".to_string(),
        frame_value: None,
        gap_severity: None,
        repairability: None,
        recovery_episode_stage: None,
        recovery_episode_progress_at_ms: None,
        coalescing_mode: None,
        unlock_reason: None,
        preempt_reason: None,
        recovery_primary_action: None,
        owner_surface_state: None,
        anchor_evidence: None,
        keyframe_episode_health: None,
        escalation_basis: None,
        budget_before: None,
        budget_after: None,
        trigger_observation_label: None,
        trigger_observation_summary: None,
        command_result: None,
        command_detail: None,
        observed_at_ms: 10.0,
    };
    stats.latest_recovery_decision_ledger = Some(matching_ledger.clone());
    stats.recent_recovery_decision_ledgers.push(matching_ledger);
    let runtime_stats = Arc::new(Mutex::new(stats));
    let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
    let bridge = build_bridge(runtime_stats.clone(), pending_runtime_recovery_action);

    bridge.record_transport_command_status(
        TransportCommand::RequestReconnectCandidate {
            observation_id: 88,
            reason: "recovering-stream".to_string(),
            reason_domain: crate::XbxEngineRecoveryReasonDomain::ConnectivityTransport,
        },
        CommandResultStatus::Deferred {
            reason: "pendingReason=existing".to_string(),
        },
    );

    let snapshot = runtime_stats.lock().expect("runtime stats lock");
    let ledger = snapshot
        .latest_recovery_decision_ledger
        .as_ref()
        .expect("decision ledger");
    assert_eq!(ledger.command_result.as_deref(), Some("deferred"));
    assert_eq!(
        ledger.command_detail.as_deref(),
        Some("command=requestReconnectCandidate detail=pendingReason=existing")
    );
    let historical_ledger = snapshot
        .recent_recovery_decision_ledgers
        .iter()
        .find(|ledger| ledger.decision_id == 88)
        .expect("historical decision ledger");
    assert_eq!(
        historical_ledger.command_result.as_deref(),
        Some("deferred")
    );
}

#[test]
fn command_result_updates_historical_ledger_when_latest_has_rotated() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.recent_recovery_decision_ledgers.push(
        crate::XbxEngineRecoveryDecisionLedgerObservation {
            decision_id: 88,
            state_before: "recovering".to_string(),
            state_after: "reconnecting".to_string(),
            input_signal: "liveness:livenessNoProgressTimeout".to_string(),
            gate_result: "pass".to_string(),
            action_selected: "requestReconnectCandidate".to_string(),
            frame_value: None,
            gap_severity: None,
            repairability: None,
            recovery_episode_stage: None,
            recovery_episode_progress_at_ms: None,
            coalescing_mode: None,
            unlock_reason: None,
            preempt_reason: None,
            recovery_primary_action: None,
            owner_surface_state: None,
            anchor_evidence: None,
            keyframe_episode_health: None,
            escalation_basis: None,
            budget_before: None,
            budget_after: None,
            trigger_observation_label: None,
            trigger_observation_summary: None,
            command_result: None,
            command_detail: None,
            observed_at_ms: 10.0,
        },
    );
    stats.latest_recovery_decision_ledger =
        Some(crate::XbxEngineRecoveryDecisionLedgerObservation {
            decision_id: 99,
            state_before: "recovering".to_string(),
            state_after: "recovering".to_string(),
            input_signal: "none".to_string(),
            gate_result: "no-signal".to_string(),
            action_selected: "none".to_string(),
            frame_value: None,
            gap_severity: None,
            repairability: None,
            recovery_episode_stage: None,
            recovery_episode_progress_at_ms: None,
            coalescing_mode: None,
            unlock_reason: None,
            preempt_reason: None,
            recovery_primary_action: None,
            owner_surface_state: None,
            anchor_evidence: None,
            keyframe_episode_health: None,
            escalation_basis: None,
            budget_before: None,
            budget_after: None,
            trigger_observation_label: None,
            trigger_observation_summary: None,
            command_result: None,
            command_detail: None,
            observed_at_ms: 11.0,
        });
    let runtime_stats = Arc::new(Mutex::new(stats));
    let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
    let bridge = build_bridge(runtime_stats.clone(), pending_runtime_recovery_action);

    bridge.record_transport_command_status(
        TransportCommand::RequestReconnectCandidate {
            observation_id: 88,
            reason: "recovering-stream".to_string(),
            reason_domain: crate::XbxEngineRecoveryReasonDomain::ConnectivityTransport,
        },
        CommandResultStatus::Succeeded,
    );

    let snapshot = runtime_stats.lock().expect("runtime stats lock");
    assert_eq!(
        snapshot
            .latest_recovery_decision_ledger
            .as_ref()
            .map(|ledger| ledger.decision_id),
        Some(99)
    );
    assert_eq!(
        snapshot
            .latest_recovery_decision_ledger
            .as_ref()
            .and_then(|ledger| ledger.command_result.as_deref()),
        None
    );
    let historical_ledger = snapshot
        .recent_recovery_decision_ledgers
        .iter()
        .find(|ledger| ledger.decision_id == 88)
        .expect("historical decision ledger");
    assert_eq!(
        historical_ledger.command_result.as_deref(),
        Some("succeeded")
    );
}

#[test]
fn trace_contract_feedback_target_pending_updates_ledger_with_family_deferred_reason() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    let decision = crate::XbxEngineRecoveryDecisionLedgerObservation {
        decision_id: 42,
        state_before: "recovering".to_string(),
        state_after: "active-recovery".to_string(),
        input_signal: "waitKeyframe:receiverWaitingKeyframe".to_string(),
        gate_result: "pass:localProbe".to_string(),
        action_selected: "requestPli".to_string(),
        frame_value: Some("RecoveryAnchor".to_string()),
        gap_severity: Some("AnchorGap".to_string()),
        repairability: None,
        recovery_episode_stage: Some("WaitingResponse".to_string()),
        recovery_episode_progress_at_ms: Some(120.0),
        coalescing_mode: None,
        unlock_reason: None,
        preempt_reason: None,
        recovery_primary_action: Some("requestPli".to_string()),
        owner_surface_state: None,
        anchor_evidence: None,
        keyframe_episode_health: None,
        escalation_basis: None,
        budget_before: None,
        budget_after: None,
        trigger_observation_label: None,
        trigger_observation_summary: None,
        command_result: None,
        command_detail: None,
        observed_at_ms: 10_140.0,
    };
    stats.latest_recovery_decision_ledger = Some(decision.clone());
    stats.recent_recovery_decision_ledgers.push(decision);

    let runtime_stats = Arc::new(Mutex::new(stats));
    let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
    let bridge = build_bridge(runtime_stats.clone(), pending_runtime_recovery_action);

    bridge.record_transport_command_status(
        TransportCommand::RequestPli {
            observation_id: 42,
            reason: "receiverWaitingKeyframe".to_string(),
        },
        CommandResultStatus::Deferred {
            reason: "capability:videoFeedbackWarming".to_string(),
        },
    );

    let snapshot = runtime_stats.lock().expect("runtime stats lock");
    let ledger = snapshot
        .latest_recovery_decision_ledger
        .as_ref()
        .expect("decision ledger");
    assert_eq!(ledger.command_result.as_deref(), Some("deferred"));
    assert_eq!(
        ledger.command_detail.as_deref(),
        Some("command=requestPli detail=capability:videoFeedbackWarming")
    );
}

#[test]
fn trace_contract_feedback_transport_not_ready_updates_ledger_with_transport_reason() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    let decision = crate::XbxEngineRecoveryDecisionLedgerObservation {
        decision_id: 43,
        state_before: "recovering".to_string(),
        state_after: "active-recovery".to_string(),
        input_signal: "waitKeyframe:receiverWaitingKeyframe".to_string(),
        gate_result: "pass:localProbe".to_string(),
        action_selected: "requestPli".to_string(),
        frame_value: Some("RecoveryAnchor".to_string()),
        gap_severity: Some("AnchorGap".to_string()),
        repairability: None,
        recovery_episode_stage: Some("WaitingResponse".to_string()),
        recovery_episode_progress_at_ms: Some(120.0),
        coalescing_mode: None,
        unlock_reason: None,
        preempt_reason: None,
        recovery_primary_action: Some("requestPli".to_string()),
        owner_surface_state: None,
        anchor_evidence: None,
        keyframe_episode_health: None,
        escalation_basis: None,
        budget_before: None,
        budget_after: None,
        trigger_observation_label: None,
        trigger_observation_summary: None,
        command_result: None,
        command_detail: None,
        observed_at_ms: 10_140.0,
    };
    stats.latest_recovery_decision_ledger = Some(decision.clone());
    stats.recent_recovery_decision_ledgers.push(decision);

    let runtime_stats = Arc::new(Mutex::new(stats));
    let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
    let bridge = build_bridge(runtime_stats.clone(), pending_runtime_recovery_action);

    bridge.record_transport_command_status(
        TransportCommand::RequestPli {
            observation_id: 43,
            reason: "receiverWaitingKeyframe".to_string(),
        },
        CommandResultStatus::Deferred {
            reason: "familyDeferred:videoRtcpFeedbackTransportNotReady".to_string(),
        },
    );

    let snapshot = runtime_stats.lock().expect("runtime stats lock");
    let ledger = snapshot
        .latest_recovery_decision_ledger
        .as_ref()
        .expect("decision ledger");
    assert_eq!(ledger.command_result.as_deref(), Some("deferred"));
    assert_eq!(
        ledger.command_detail.as_deref(),
        Some("command=requestPli detail=familyDeferred:videoRtcpFeedbackTransportNotReady")
    );
}
