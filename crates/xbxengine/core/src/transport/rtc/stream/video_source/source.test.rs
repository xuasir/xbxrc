use super::{
    resolve_effective_idle_controls, resolve_inspection_admission,
    resolve_recovery_keyframe_action, should_absorb_idle_timeout_for_steady_gap,
    should_trigger_idle_timeout, RecoveryKeyframeAction, RtcVideoFrameSource,
};
use crate::media::video::h264::inspection::H264AccessUnitInspection;
use crate::media::video::test_fixtures::{
    bootstrap_idr_nalu, bootstrap_pps_nalu, bootstrap_sps_nalu, make_video_rtp_packet,
    make_video_source_for_test, send_bootstrap_access_unit, NoopRtcpPort,
};
use crate::transport::rtc::stream::adapter_types::{
    TransportAdmissionObservation, TransportLossObservation, TransportObservation,
};
use crate::transport::rtc::stream::packet_types::{RtcVideoIngressKind, RtcVideoRepairMetadata};
use crate::transport::rtc::stream::sink::RtcRtcpSendPort;
use crate::transport::rtc::stream::video_source::NackSchedulerConfig;
use crate::XbxEngineRemoteAnswerObservation;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use xbxengine_protocol::{XbxEngineTargetTypeDto, XbxEngineTransportStateDto};

#[test]
fn idle_timeout_is_suppressed_before_first_packet() {
    let started_at = Instant::now();
    let later = started_at + Duration::from_millis(500);

    assert!(!should_trigger_idle_timeout(
        false,
        later,
        started_at,
        Duration::from_millis(150),
    ));
    assert!(should_trigger_idle_timeout(
        true,
        later,
        started_at,
        Duration::from_millis(150),
    ));
}

#[test]
fn cloud_profile_relaxes_idle_timeout_and_hint_cooldown() {
    let (idle_timeout, idle_hint_cooldown) = resolve_effective_idle_controls(
        Duration::from_millis(250),
        Duration::from_millis(400),
        Some(&XbxEngineTargetTypeDto::Cloud),
        Some(120.0),
    );

    assert_eq!(idle_timeout, Duration::from_millis(700));
    assert_eq!(idle_hint_cooldown, Duration::from_millis(700));
}

#[test]
fn slow_feedback_relaxes_idle_timeout_even_for_non_cloud() {
    let (idle_timeout, idle_hint_cooldown) = resolve_effective_idle_controls(
        Duration::from_millis(300),
        Duration::from_millis(450),
        Some(&XbxEngineTargetTypeDto::Home),
        Some(500.0),
    );

    assert_eq!(idle_timeout, Duration::from_millis(700));
    assert_eq!(idle_hint_cooldown, Duration::from_millis(700));
}

#[test]
fn steady_idle_timeout_is_absorbed_when_render_output_is_still_fresh() {
    let absorbed = should_absorb_idle_timeout_for_steady_gap(
        XbxEngineTransportStateDto::Connected,
        3,
        Some(3),
        Some("chain-clean-keyframe-submitted"),
        Some(1_000.0 - 90.0),
        Some(1_000.0 - 70.0),
        Some(false),
        Some(false),
        1_000.0,
        Duration::from_millis(150),
    );
    assert!(absorbed);
}

#[test]
fn no_render_slack_or_no_fresh_output_still_emits_idle_timeout_observation() {
    let stale_output_not_absorbed = should_absorb_idle_timeout_for_steady_gap(
        XbxEngineTransportStateDto::Connected,
        3,
        Some(3),
        Some("chain-clean-keyframe-submitted"),
        Some(1_000.0 - 520.0),
        Some(1_000.0 - 510.0),
        Some(false),
        Some(false),
        1_000.0,
        Duration::from_millis(150),
    );
    assert!(!stale_output_not_absorbed);

    let no_anchor_not_absorbed = should_absorb_idle_timeout_for_steady_gap(
        XbxEngineTransportStateDto::Connected,
        3,
        None,
        None,
        Some(1_000.0 - 60.0),
        Some(1_000.0 - 45.0),
        Some(false),
        Some(false),
        1_000.0,
        Duration::from_millis(150),
    );
    assert!(!no_anchor_not_absorbed);
}

#[test]
fn clean_anchor_soft_reentry_allows_healthy_delta_to_submit() {
    let (next_waiting_for_recovery_keyframe, recovery_action) =
        resolve_recovery_keyframe_action(true, false, 0, 0, false, true);

    assert!(!next_waiting_for_recovery_keyframe);
    assert_eq!(recovery_action, RecoveryKeyframeAction::Submit);
}

#[test]
fn clean_anchor_soft_reentry_does_not_override_loss_semantics() {
    let (next_waiting_for_recovery_keyframe, recovery_action) =
        resolve_recovery_keyframe_action(true, false, 0, 1, false, true);

    assert!(!next_waiting_for_recovery_keyframe);
    assert_eq!(
        recovery_action,
        RecoveryKeyframeAction::DropAndRequestKeyframe
    );
}

#[test]
fn recovery_wait_without_soft_reentry_remains_waiting() {
    let (next_waiting_for_recovery_keyframe, recovery_action) =
        resolve_recovery_keyframe_action(true, true, 0, 0, false, false);

    assert!(next_waiting_for_recovery_keyframe);
    assert_eq!(recovery_action, RecoveryKeyframeAction::WaitKeyframe);
}

#[test]
fn low_value_local_gap_wait_is_absorbed_without_transport_wait_upgrade() {
    let (next_waiting_for_recovery_keyframe, recovery_action) =
        resolve_recovery_keyframe_action(true, false, 0, 0, false, false);

    assert!(!next_waiting_for_recovery_keyframe);
    assert_eq!(recovery_action, RecoveryKeyframeAction::Submit);
}

#[test]
fn inspection_admission_rejects_frames_without_bootstrap_or_continuation() {
    assert_eq!(
        resolve_inspection_admission(&H264AccessUnitInspection {
            nals: Vec::new(),
            parameter_sets: None,
            width: None,
            height: None,
            is_idr: true,
            has_inband_sps: false,
            has_inband_pps: false,
            slice_headers_valid: true,
            parameter_sets_changed: false,
            config_changed: false,
            bootstrap_ready: true,
            bootstrap_reject_reason: None,
            commit_state:
                crate::media::video::h264::inspection::H264AccessUnitInspector::test_commit_state(),
        }),
        super::InspectionAdmission::Accept
    );

    assert_eq!(
        resolve_inspection_admission(&H264AccessUnitInspection {
            nals: Vec::new(),
            parameter_sets: None,
            width: None,
            height: None,
            is_idr: false,
            has_inband_sps: false,
            has_inband_pps: false,
            slice_headers_valid: true,
            parameter_sets_changed: false,
            config_changed: false,
            bootstrap_ready: false,
            bootstrap_reject_reason: None,
            commit_state:
                crate::media::video::h264::inspection::H264AccessUnitInspector::test_commit_state(),
        }),
        super::InspectionAdmission::AwaitRecoveryKeyframe
    );

    assert_eq!(
        resolve_inspection_admission(&H264AccessUnitInspection {
            nals: Vec::new(),
            parameter_sets: None,
            width: None,
            height: None,
            is_idr: false,
            has_inband_sps: false,
            has_inband_pps: false,
            slice_headers_valid: false,
            parameter_sets_changed: false,
            config_changed: false,
            bootstrap_ready: false,
            bootstrap_reject_reason: None,
            commit_state:
                crate::media::video::h264::inspection::H264AccessUnitInspector::test_commit_state(),
        }),
        super::InspectionAdmission::AwaitRecoveryKeyframe
    );
}

#[test]
fn clean_keyframe_anchor_records_current_transport_recovery_epoch() {
    let (tx, rx) = tokio::sync::mpsc::channel(1);
    let (transport_observation_tx, _transport_observation_rx) =
        tokio::sync::mpsc::unbounded_channel();
    let rtcp_port: Arc<dyn RtcRtcpSendPort> = Arc::new(NoopRtcpPort::default());
    let runtime_stats = Arc::new(Mutex::new(crate::XbxEngineMediaRuntimeStats::default()));
    let source = RtcVideoFrameSource::new(
        rx,
        transport_observation_tx,
        rtcp_port,
        runtime_stats.clone(),
        16,
        Duration::from_millis(10),
        Duration::from_millis(20),
        Duration::from_millis(200),
        NackSchedulerConfig {
            max_age_ms: 1_000,
            frame_deadline_ms: 120,
            burst_count: 2,
            retry_interval_ms: 20,
            max_retry_count: 3,
        },
    );
    drop(tx);

    source.runtime_stats.begin_transport_recovery_episode(100.0);
    source.record_clean_keyframe_anchor(180.0);

    let stats = runtime_stats.lock().expect("runtime stats lock");
    assert_eq!(stats.video_anchor_clean_epoch, Some(1));
    assert_eq!(stats.video_anchor_clean_observed_at_ms, Some(180.0));
    assert_eq!(
        stats.video_anchor_clean_source_event.as_deref(),
        Some("chain-clean-keyframe-submitted")
    );
    assert!(!stats.transport_recovery_episode_active);
    assert_eq!(stats.transport_recovery_episode_closed_at_ms, Some(180.0));
    assert_eq!(
        stats.transport_recovery_episode_close_reason.as_deref(),
        Some("cleanAnchor")
    );
}

#[tokio::test]
async fn bootstrap_keyframe_packets_are_assembled_into_frame() {
    let (tx, mut transport_observation_rx, mut source) = make_video_source_for_test();

    send_bootstrap_access_unit(&tx, 100, 9000).await;
    tx.send(make_video_rtp_packet(103, 9016, true, bootstrap_idr_nalu()))
        .await
        .expect("next frame packet should flush previous sample");
    drop(tx);

    let frame = tokio::time::timeout(Duration::from_millis(200), source.recv_frame_inner())
        .await
        .expect("frame assembly should finish")
        .expect("bootstrap frame should be emitted");
    assert!(frame.is_keyframe);
    assert!(frame.h264.bootstrap_ready);
    assert_eq!(frame.rtp_timestamp, 9000);
    assert!(frame.width > 0);
    assert!(frame.height > 0);
    assert!(transport_observation_rx.try_recv().is_err());
}

#[tokio::test]
async fn repair_packet_closes_bootstrap_gap_and_allows_frame_assembly() {
    let (tx, mut transport_observation_rx, mut source) = make_video_source_for_test();

    tx.send(make_video_rtp_packet(
        100,
        9000,
        false,
        bootstrap_sps_nalu(),
    ))
    .await
    .expect("sps packet should enqueue");
    tx.send(make_video_rtp_packet(102, 9000, true, bootstrap_idr_nalu()))
        .await
        .expect("idr packet should enqueue");
    let mut repair_packet = make_video_rtp_packet(101, 9000, false, bootstrap_pps_nalu());
    repair_packet.ingress_kind = RtcVideoIngressKind::RtxReinject {
        repair: RtcVideoRepairMetadata {
            native_ssrc: 88,
            native_payload_type: 97,
            native_sequence_number: 9_001,
        },
    };
    tx.send(repair_packet)
        .await
        .expect("repair packet should enqueue");
    tx.send(make_video_rtp_packet(103, 9016, true, bootstrap_idr_nalu()))
        .await
        .expect("next frame packet should flush previous sample");
    drop(tx);

    let frame = tokio::time::timeout(Duration::from_millis(200), source.recv_frame_inner())
        .await
        .expect("frame assembly should finish")
        .expect("repaired bootstrap frame should be emitted");
    assert!(frame.is_keyframe);
    assert!(frame.h264.bootstrap_ready);
    assert_eq!(frame.rtp_timestamp, 9000);
    assert!(transport_observation_rx.try_recv().is_err());

    let latest = source
        .runtime_stats
        .read(|stats| stats.latest_video_rtx_reinject_observation.clone())
        .flatten()
        .expect("repair observation should be recorded");
    assert_eq!(latest.sequence_number, 101);
    assert_eq!(latest.rtp_timestamp, 9000);
    assert_eq!(latest.native_sequence_number, Some(9_001));
    assert_eq!(latest.repair_ssrc, 88);
    assert!(matches!(
        latest.stage.as_str(),
        "adapterResolveMiss" | "adapterResolved"
    ));
}

#[tokio::test]
async fn repair_reorder_gap_closure_stays_local_and_records_resolved_gap_match() {
    let (tx, mut transport_observation_rx, mut source) = make_video_source_for_test();

    tx.send(make_video_rtp_packet(
        100,
        9_000,
        false,
        bootstrap_sps_nalu(),
    ))
    .await
    .expect("sps packet should enqueue");
    tx.send(make_video_rtp_packet(
        102,
        9_000,
        true,
        bootstrap_idr_nalu(),
    ))
    .await
    .expect("idr packet should enqueue");
    let mut repair_packet = make_video_rtp_packet(101, 9_000, false, bootstrap_pps_nalu());
    repair_packet.ingress_kind = RtcVideoIngressKind::RtxReinject {
        repair: RtcVideoRepairMetadata {
            native_ssrc: 88,
            native_payload_type: 97,
            native_sequence_number: 9_001,
        },
    };
    tx.send(repair_packet)
        .await
        .expect("repair packet should enqueue");
    tx.send(make_video_rtp_packet(
        103,
        9_016,
        true,
        bootstrap_idr_nalu(),
    ))
    .await
    .expect("next frame packet should flush previous sample");
    drop(tx);

    let frame = tokio::time::timeout(Duration::from_millis(200), source.recv_frame_inner())
        .await
        .expect("frame assembly should finish")
        .expect("repaired bootstrap frame should be emitted");
    assert!(frame.is_keyframe);
    assert!(frame.h264.bootstrap_ready);
    assert_eq!(frame.rtp_timestamp, 9_000);

    let latest = source
        .runtime_stats
        .read(|stats| stats.latest_video_rtx_reinject_observation.clone())
        .flatten()
        .expect("repair observation should be recorded");
    assert!(matches!(
        latest.stage.as_str(),
        "adapterResolveMiss" | "adapterResolved"
    ));
    assert_eq!(latest.sequence_number, 101);
    assert_eq!(latest.native_sequence_number, Some(9_001));
    assert_eq!(latest.repair_ssrc, 88);
    if latest.stage == "adapterResolved" {
        assert!(latest.matched_nack_range);
        assert!(latest.matched_pending_gap);
        assert_eq!(latest.matched_gap_sequence, Some(101));
    }

    assert!(
        tokio::time::timeout(Duration::from_millis(50), transport_observation_rx.recv())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn idr_without_parameter_sets_requests_recovery_keyframe_instead_of_emitting_frame() {
    let (tx, mut transport_observation_rx, mut source) = make_video_source_for_test();

    tx.send(make_video_rtp_packet(100, 9001, true, bootstrap_idr_nalu()))
        .await
        .expect("idr packet should enqueue");
    tx.send(make_video_rtp_packet(101, 9017, true, bootstrap_idr_nalu()))
        .await
        .expect("follow-up packet should flush previous sample");
    drop(tx);

    let frame = tokio::time::timeout(Duration::from_millis(200), source.recv_frame_inner())
        .await
        .expect("reader should finish after rx closes");
    assert!(frame.is_none());

    let observation =
        tokio::time::timeout(Duration::from_millis(50), transport_observation_rx.recv())
            .await
            .expect("await-recovery observation should be emitted")
            .expect("observation should exist");
    assert_eq!(
        observation,
        TransportObservation::Admission(TransportAdmissionObservation::AwaitRecoveryKeyframe)
    );
}

#[tokio::test]
async fn startup_h264_without_sprop_requests_keyframe_before_first_bad_frame_recovery() {
    let (tx, mut transport_observation_rx, mut source) = make_video_source_for_test();
    source.runtime_stats.update(|stats| {
        stats.session_phase = Some("priming".to_string());
        stats.transport_state = XbxEngineTransportStateDto::Connected;
        stats.latest_remote_answer_observation = Some(XbxEngineRemoteAnswerObservation {
            observation_id: 1,
            video_payload_order: vec![124],
            selected_video_payload_type: Some(124),
            selected_video_mime_type: Some("video/h264".to_string()),
            selected_video_profile_level_id: Some("4d002a".to_string()),
            selected_video_h264_sprop_parameter_sets: None,
            accepted_video_rtcp_feedback: vec!["nack:pli".to_string()],
            accepted_audio_rtcp_feedback: Vec::new(),
            accepted_video_header_extensions: Vec::new(),
            accepted_audio_header_extensions: Vec::new(),
            observed_at_ms: 10.0,
        });
    });

    tx.send(make_video_rtp_packet(100, 9001, true, bootstrap_idr_nalu()))
        .await
        .expect("idr packet should enqueue");
    tx.send(make_video_rtp_packet(101, 9017, true, bootstrap_idr_nalu()))
        .await
        .expect("follow-up packet should flush previous sample");
    drop(tx);

    let frame = tokio::time::timeout(Duration::from_millis(200), source.recv_frame_inner())
        .await
        .expect("reader should finish after rx closes");
    assert!(frame.is_none());

    let first = tokio::time::timeout(Duration::from_millis(50), transport_observation_rx.recv())
        .await
        .expect("startup keyframe request observation should be emitted")
        .expect("observation should exist");
    assert_eq!(
        first,
        TransportObservation::Loss(TransportLossObservation::RecoveryKeyframeRequested)
    );

    let second = tokio::time::timeout(Duration::from_millis(50), transport_observation_rx.recv())
        .await
        .expect("await-recovery observation should be emitted")
        .expect("observation should exist");
    assert_eq!(
        second,
        TransportObservation::Admission(TransportAdmissionObservation::AwaitRecoveryKeyframe)
    );
}

#[tokio::test]
async fn bootstrap_packets_without_followup_boundary_do_not_emit_partial_frame() {
    let (tx, mut transport_observation_rx, mut source) = make_video_source_for_test();

    send_bootstrap_access_unit(&tx, 100, 9000).await;
    drop(tx);

    let frame = tokio::time::timeout(Duration::from_millis(120), source.recv_frame_inner())
        .await
        .expect("reader should finish after rx closes");
    assert!(frame.is_none());
    assert!(transport_observation_rx.try_recv().is_err());
}

#[tokio::test]
async fn repair_rtx_packet_keeps_explicit_provenance_through_source_stage_updates() {
    let (tx, mut transport_observation_rx, mut source) = make_video_source_for_test();

    let mut packet = make_video_rtp_packet(100, 9_000, true, bootstrap_idr_nalu());
    packet.meta.ssrc = 777;
    packet.meta.payload_type = 124;
    packet.ingress_kind = RtcVideoIngressKind::RtxReinject {
        repair: RtcVideoRepairMetadata {
            native_ssrc: 99,
            native_payload_type: 97,
            native_sequence_number: 4_321,
        },
    };
    tx.send(packet).await.expect("repair packet should enqueue");
    drop(tx);

    let frame = tokio::time::timeout(Duration::from_millis(200), source.recv_frame_inner())
        .await
        .expect("reader should finish after rx closes");
    assert!(frame.is_none());

    let latest = source
        .runtime_stats
        .read(|stats| stats.latest_video_rtx_reinject_observation.clone())
        .flatten()
        .expect("repair provenance observation should be recorded");
    assert_eq!(latest.stage, "adapterResolveMiss");
    assert_eq!(latest.sequence_number, 100);
    assert_eq!(latest.repair_ssrc, 99);
    assert_eq!(latest.primary_ssrc, 777);
    assert_eq!(latest.native_sequence_number, Some(4_321));
    assert!(!latest.matched_nack_range);
    assert!(!latest.matched_pending_gap);
    assert!(latest.matched_gap_sequence.is_none());
    assert!(
        tokio::time::timeout(Duration::from_millis(50), transport_observation_rx.recv())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn unmatched_repair_rtx_burst_stays_local_and_does_not_emit_transport_observation() {
    let (tx, mut transport_observation_rx, mut source) = make_video_source_for_test();

    for offset in 0..5u16 {
        let mut packet = make_video_rtp_packet(300 + offset, 12_000, false, bootstrap_pps_nalu());
        packet.meta.ssrc = 777;
        packet.meta.payload_type = 124;
        packet.ingress_kind = RtcVideoIngressKind::RtxReinject {
            repair: RtcVideoRepairMetadata {
                native_ssrc: 99,
                native_payload_type: 97,
                native_sequence_number: 8_000 + offset,
            },
        };
        tx.send(packet)
            .await
            .expect("repair burst packet should enqueue");
    }
    drop(tx);

    let frame = tokio::time::timeout(Duration::from_millis(200), source.recv_frame_inner())
        .await
        .expect("reader should finish after rx closes");
    assert!(frame.is_none());

    let latest = source
        .runtime_stats
        .read(|stats| stats.latest_video_rtx_reinject_observation.clone())
        .flatten()
        .expect("repair burst observation should be recorded");
    assert_eq!(latest.stage, "adapterResolveMiss");
    assert_eq!(latest.sequence_number, 304);
    assert_eq!(latest.repair_ssrc, 99);
    assert_eq!(latest.primary_ssrc, 777);
    assert_eq!(latest.native_sequence_number, Some(8_004));
    assert!(!latest.matched_nack_range);
    assert!(!latest.matched_pending_gap);
    assert!(latest.matched_gap_sequence.is_none());

    assert!(
        tokio::time::timeout(Duration::from_millis(50), transport_observation_rx.recv())
            .await
            .is_err()
    );
}
