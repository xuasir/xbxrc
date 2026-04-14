use super::{
    now_ms_f64, resolve_effective_idle_controls, resolve_inspection_admission,
    resolve_recovery_keyframe_action, should_absorb_idle_timeout_for_steady_gap,
    should_trigger_idle_timeout, RecoveryKeyframeAction, RtcVideoFrameSource,
};
use crate::media::video::h264::inspection::{H264AccessUnitInspection, H264AccessUnitInspector};
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
use crate::{XbxEngineRemoteAnswerObservation, XbxEngineVideoTrackStatus};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use xbxengine_protocol::{XbxEngineTargetTypeDto, XbxEngineTransportStateDto};

fn serviceable_runtime_stats(now_ms: f64) -> crate::XbxEngineMediaRuntimeStats {
    let mut stats = crate::XbxEngineMediaRuntimeStats::default();
    stats.transport_state = XbxEngineTransportStateDto::Connected;
    stats.transport_recovery_epoch = 7;
    stats.video_anchor_clean_epoch = Some(7);
    stats.video_anchor_clean_observed_at_ms = Some(now_ms - 20.0);
    stats.video_anchor_clean_source_event = Some("chain-clean-keyframe-submitted".to_string());
    stats.latest_video_decode_ok_time_ms = Some(now_ms - 24.0);
    stats.latest_video_host_present_time_ms = Some(now_ms - 28.0);
    stats.video_renderer_stalled = Some(false);
    stats.latest_video_track_status = Some(XbxEngineVideoTrackStatus {
        state: "remoteTrackAttached".to_string(),
        video_width: Some(1920),
        video_height: Some(1080),
        mime_type: Some("video/h264".to_string()),
        transport_state: XbxEngineTransportStateDto::Connected,
        video_bytes_total: 88_000,
        video_packet_count_total: 320,
        audio_bytes_total: 2_048,
        observed_at_ms: now_ms - 10.0,
    });
    stats
}

fn startup_h264_answer_without_sprop() -> XbxEngineRemoteAnswerObservation {
    XbxEngineRemoteAnswerObservation {
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
    }
}

fn bootstrap_non_idr_nalu() -> Vec<u8> {
    let mut nalu = bootstrap_idr_nalu().to_vec();
    nalu[0] = 0x41;
    nalu
}

#[test]
fn first_timeout_detection_only_starts_confirmation_window() {
    let now = Instant::now();
    let mut pending_since = None;

    assert!(
        !RtcVideoFrameSource::should_confirm_transient_timeout_signal(
            &mut pending_since,
            now,
            Duration::from_millis(120),
        )
    );
    assert!(pending_since.is_some());
}

#[test]
fn timeout_detection_after_confirmation_window_is_emitted() {
    let now = Instant::now();
    let mut pending_since = Some(now);

    assert!(
        RtcVideoFrameSource::should_confirm_transient_timeout_signal(
            &mut pending_since,
            now + Duration::from_millis(130),
            Duration::from_millis(120),
        )
    );
    assert!(pending_since.is_none());
}

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
fn recovery_wait_without_hard_risk_allows_healthy_delta_to_submit() {
    let (next_waiting_for_recovery_keyframe, recovery_action) =
        resolve_recovery_keyframe_action(true, true, false, false, 0, 0, false);

    assert!(!next_waiting_for_recovery_keyframe);
    assert_eq!(recovery_action, RecoveryKeyframeAction::Submit);
}

#[test]
fn clean_anchor_serviceable_output_allows_soft_recovery_keyframe_request_after_invalid_bootstrap() {
    let now_ms = 1_000.0;
    let mut stats = serviceable_runtime_stats(now_ms);
    stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
        observation_id: 1,
        source_event: "frame-observed".to_string(),
        gap: None,
        frame: None,
        chain: crate::XbxEngineVideoTimelineChainSnapshot {
            state: "healthy".to_string(),
            reason: None,
            chain_break_evidence: None,

            observed_at_ms: now_ms - 12.0,
        },
        observed_at_ms: now_ms - 12.0,
    });

    assert!(RtcVideoFrameSource::should_soft_request_recovery_keyframe(
        &stats,
        now_ms,
        Some("bootstrapMissingSps"),
        true,
        false,
        false,
    ));
}

#[test]
fn unresolved_current_transport_issue_blocks_soft_recovery_keyframe_request() {
    let now_ms = 1_000.0;
    let mut stats = serviceable_runtime_stats(now_ms);
    stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
        observation_id: 2,
        source_event: "frame-await-recovery-keyframe".to_string(),
        gap: Some(crate::XbxEngineVideoTimelineGapSnapshot {
            state: "pending".to_string(),
            sequence: Some(99),
            frame_rtp_timestamp: None,
            frame_importance: Some("reference".to_string()),
            budget_importance: None,

            evidence_importance: None,

            gap_dependency_confidence: None,

            observed_at_ms: now_ms - 5.0,
        }),
        frame: None,
        chain: crate::XbxEngineVideoTimelineChainSnapshot {
            state: "recovering".to_string(),
            reason: Some("transportAwaitRecoveryKeyframe".to_string()),
            chain_break_evidence: None,

            observed_at_ms: now_ms - 5.0,
        },
        observed_at_ms: now_ms - 5.0,
    });

    assert!(!RtcVideoFrameSource::should_soft_request_recovery_keyframe(
        &stats,
        now_ms,
        Some("bootstrapMissingSps"),
        true,
        true,
        true,
    ));
}

#[test]
fn recovery_wait_does_not_override_loss_semantics() {
    let (next_waiting_for_recovery_keyframe, recovery_action) =
        resolve_recovery_keyframe_action(true, true, false, false, 0, 1, false);

    assert!(!next_waiting_for_recovery_keyframe);
    assert_eq!(
        recovery_action,
        RecoveryKeyframeAction::DropAndRequestKeyframe
    );
}

#[test]
fn lossy_keyframe_defers_to_nack_recovery_admission() {
    let (next_waiting_for_recovery_keyframe, recovery_action) =
        resolve_recovery_keyframe_action(true, false, false, true, 0, 2, true);

    assert!(!next_waiting_for_recovery_keyframe);
    assert_eq!(
        recovery_action,
        RecoveryKeyframeAction::DropAndRequestKeyframe
    );
}

#[test]
fn short_sample_loss_burst_stays_in_drop_and_request_keyframe() {
    let (next_waiting_for_recovery_keyframe, recovery_action) =
        resolve_recovery_keyframe_action(true, false, false, false, 2, 1, false);

    assert!(!next_waiting_for_recovery_keyframe);
    assert_eq!(
        recovery_action,
        RecoveryKeyframeAction::DropAndRequestKeyframe
    );
}

#[test]
fn longer_sample_loss_burst_still_defers_to_nack_recovery_admission() {
    let (next_waiting_for_recovery_keyframe, recovery_action) =
        resolve_recovery_keyframe_action(true, false, false, false, 3, 1, false);

    assert!(!next_waiting_for_recovery_keyframe);
    assert_eq!(
        recovery_action,
        RecoveryKeyframeAction::DropAndRequestKeyframe
    );
}

#[test]
fn low_value_local_gap_wait_is_absorbed_without_transport_wait_upgrade() {
    let (next_waiting_for_recovery_keyframe, recovery_action) =
        resolve_recovery_keyframe_action(true, true, false, false, 0, 0, false);

    assert!(!next_waiting_for_recovery_keyframe);
    assert_eq!(recovery_action, RecoveryKeyframeAction::Submit);
}

#[test]
fn pre_first_frame_wait_does_not_absorb_non_keyframe_delta() {
    let (next_waiting_for_recovery_keyframe, recovery_action) =
        resolve_recovery_keyframe_action(false, true, false, false, 0, 0, false);

    assert!(next_waiting_for_recovery_keyframe);
    assert_eq!(recovery_action, RecoveryKeyframeAction::WaitKeyframe);
}

#[test]
fn sustaining_recovery_prefers_keepalive_over_reenter_wait_keyframe() {
    let (next_waiting_for_recovery_keyframe, recovery_action) =
        resolve_recovery_keyframe_action(true, true, true, true, 0, 0, false);

    assert!(!next_waiting_for_recovery_keyframe);
    assert_eq!(recovery_action, RecoveryKeyframeAction::Submit);
}

#[test]
fn hard_recovery_wait_without_building_phase_still_reenters_wait_keyframe() {
    let (next_waiting_for_recovery_keyframe, recovery_action) =
        resolve_recovery_keyframe_action(true, true, false, true, 0, 0, false);

    assert!(next_waiting_for_recovery_keyframe);
    assert_eq!(recovery_action, RecoveryKeyframeAction::WaitKeyframe);
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
        }, false, false, false),
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
        }, false, false, false),
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
        }, false, false, false),
        super::InspectionAdmission::AwaitRecoveryKeyframe
    );
}

#[test]
fn pre_first_frame_non_idr_continuation_is_rejected_until_first_frame_exists() {
    let inspector = H264AccessUnitInspector::new();
    inspector
        .seed_committed_parameter_sets_if_absent(&bootstrap_sps_nalu(), &bootstrap_pps_nalu())
        .expect("seed committed parameter sets");
    let mut payload = vec![0, 0, 0, 1];
    payload.extend_from_slice(&bootstrap_non_idr_nalu());
    let inspection = inspector
        .inspect_access_unit(&payload)
        .expect("inspect non-idr access unit");

    assert!(inspection.delta_continuation_ready());
    assert_eq!(
        resolve_inspection_admission(&inspection, false, false, false),
        super::InspectionAdmission::AwaitRecoveryKeyframe
    );
    assert_eq!(
        resolve_inspection_admission(&inspection, false, true, false),
        super::InspectionAdmission::Accept
    );
    assert_eq!(
        resolve_inspection_admission(&inspection, true, false, false),
        super::InspectionAdmission::Accept
    );
}

#[test]
fn sustaining_recovery_continuation_is_accepted_before_first_frame_output() {
    let inspector = H264AccessUnitInspector::new();
    inspector
        .seed_committed_parameter_sets_if_absent(&bootstrap_sps_nalu(), &bootstrap_pps_nalu())
        .expect("seed committed parameter sets");
    let mut payload = vec![0, 0, 0, 1];
    payload.extend_from_slice(&bootstrap_non_idr_nalu());
    let inspection = inspector
        .inspect_access_unit(&payload)
        .expect("inspect non-idr access unit");

    assert!(inspection.delta_continuation_ready());
    assert_eq!(
        resolve_inspection_admission(&inspection, false, false, true),
        super::InspectionAdmission::Accept
    );
}

#[tokio::test]
async fn sustaining_recovery_reject_restarts_recovery_keyframe_request() {
    let (tx, mut transport_observation_rx, mut source) = make_video_source_for_test();
    source
        .timeline_state
        .on_clean_keyframe_ingress(9_000, now_ms_f64());
    source.timeline_state.on_clean_keyframe_submitted();

    assert!(source.timeline_state.in_sustaining_recovery());
    assert!(!source.waiting_for_recovery_keyframe());

    let non_idr = bootstrap_non_idr_nalu();
    tx.send(make_video_rtp_packet(100, 9_001, true, &non_idr))
        .await
        .expect("non-idr packet should enqueue");
    tx.send(make_video_rtp_packet(101, 9_017, true, &non_idr))
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
            .expect("sustaining reject should request a new recovery keyframe")
            .expect("observation should exist");
    assert_eq!(
        observation,
        TransportObservation::Loss(TransportLossObservation::RecoveryKeyframeRequested)
    );
    assert!(transport_observation_rx.try_recv().is_err());
    assert!(source.waiting_for_recovery_keyframe());
    assert!(!source.timeline_state.in_sustaining_recovery());
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
    assert!(stats.transport_recovery_episode_active);
    assert_eq!(stats.transport_recovery_episode_closed_at_ms, None);
    assert_eq!(stats.transport_recovery_episode_close_reason, None);
}

#[test]
fn packet_loss_detected_does_not_reopen_episode_but_keyframe_request_does() {
    let (_tx, rx) = tokio::sync::mpsc::channel(1);
    let (transport_observation_tx, _transport_observation_rx) =
        tokio::sync::mpsc::unbounded_channel();
    let rtcp_port: Arc<dyn RtcRtcpSendPort> = Arc::new(NoopRtcpPort::default());
    let runtime_stats = Arc::new(Mutex::new(crate::XbxEngineMediaRuntimeStats::default()));
    let mut source = RtcVideoFrameSource::new(
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

    source.runtime_stats.begin_transport_recovery_episode(100.0);
    source.record_clean_keyframe_anchor(140.0);
    source
        .runtime_stats
        .complete_transport_recovery_after_stable_settle(180.0);

    source.queue_transport_observation(TransportObservation::Loss(
        TransportLossObservation::PacketLossDetected,
    ));

    {
        let stats = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(stats.transport_recovery_epoch, 1);
        assert!(!stats.transport_recovery_episode_active);
        assert_eq!(stats.video_anchor_clean_epoch, Some(1));
        assert_eq!(
            stats.video_anchor_clean_source_event.as_deref(),
            Some("chain-clean-keyframe-submitted")
        );
    }

    source.queue_transport_observation(TransportObservation::Loss(
        TransportLossObservation::RecoveryKeyframeRequested,
    ));

    let stats = runtime_stats.lock().expect("runtime stats lock");
    assert_eq!(stats.transport_recovery_epoch, 2);
    assert!(stats.transport_recovery_episode_active);
    assert!(stats
        .transport_recovery_episode_opened_at_ms
        .is_some_and(|opened_at_ms| opened_at_ms >= 180.0));
    assert_eq!(stats.video_anchor_clean_epoch, None);
    assert_eq!(stats.video_anchor_clean_observed_at_ms, None);
    assert_eq!(stats.video_anchor_clean_source_event, None);
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
async fn clean_keyframe_then_consecutive_non_idr_continuation_does_not_fall_back_to_wait_keyframe()
{
    let (tx, mut transport_observation_rx, mut source) = make_video_source_for_test();
    source
        .timeline_state
        .on_admission_await_recovery_keyframe(Some("awaitingRecoveryKeyframe"));
    source.timeline_state.mark_gap_reorder_pending(
        &[401],
        0.5,
        Some(8_900),
        "reference",
        "reference",
    );
    assert!(source.timeline_state.waiting_for_recovery_keyframe());
    assert!(source.timeline_state.has_hard_recovery_risk_for_test());

    send_bootstrap_access_unit(&tx, 100, 9_000).await;
    let non_idr = bootstrap_non_idr_nalu();
    tx.send(make_video_rtp_packet(103, 9_016, true, &non_idr))
        .await
        .expect("first continuation should flush bootstrap sample");
    tx.send(make_video_rtp_packet(104, 9_032, true, &non_idr))
        .await
        .expect("second continuation should flush first continuation");
    tx.send(make_video_rtp_packet(105, 9_048, true, &non_idr))
        .await
        .expect("third continuation should flush second continuation");
    tx.send(make_video_rtp_packet(106, 9_064, true, &non_idr))
        .await
        .expect("tail continuation should flush third continuation");
    drop(tx);

    let bootstrap_frame =
        tokio::time::timeout(Duration::from_millis(200), source.recv_frame_inner())
            .await
            .expect("bootstrap frame should assemble")
            .expect("bootstrap frame should be emitted");
    assert!(bootstrap_frame.is_keyframe);
    assert!(!source.timeline_state.waiting_for_recovery_keyframe());
    assert!(!source.timeline_state.has_hard_recovery_risk_for_test());

    for _ in 0..3 {
        let _ = tokio::time::timeout(Duration::from_millis(200), source.recv_frame_inner()).await;
    }

    while transport_observation_rx.try_recv().is_ok() {}
}

#[tokio::test]
async fn stale_wait_after_clean_anchor_still_submits_delta_continuation() {
    let (tx, mut transport_observation_rx, mut source) = make_video_source_for_test();

    send_bootstrap_access_unit(&tx, 100, 9_000).await;
    let non_idr = bootstrap_non_idr_nalu();
    tx.send(make_video_rtp_packet(103, 9_016, true, &non_idr))
        .await
        .expect("first continuation should flush bootstrap sample");

    let bootstrap_frame =
        tokio::time::timeout(Duration::from_millis(200), source.recv_frame_inner())
            .await
            .expect("bootstrap frame should assemble")
            .expect("bootstrap frame should be emitted");
    assert!(bootstrap_frame.is_keyframe);
    bootstrap_frame.h264.commit();
    source.runtime_stats.update(|stats| {
        stats.latest_video_decode_ok_time_ms = Some(1.0);
        stats.latest_video_host_present_time_ms = Some(1.0);
    });
    source
        .timeline_state
        .on_clean_keyframe_ingress(bootstrap_frame.rtp_timestamp, now_ms_f64());
    source.timeline_state.on_clean_keyframe_submitted();

    source.set_waiting_for_recovery_keyframe(true);
    assert!(source.waiting_for_recovery_keyframe());
    source.timeline_state.mark_gap_repair_in_flight(
        &[401],
        2.0,
        Some(9_000),
        "keyframe",
        "keyframe",
    );
    assert!(source.timeline_state.has_hard_recovery_risk_for_test());

    tx.send(make_video_rtp_packet(104, 9_032, true, &non_idr))
        .await
        .expect("non-idr packet should enqueue");
    tx.send(make_video_rtp_packet(105, 9_048, true, &non_idr))
        .await
        .expect("follow-up packet should flush previous sample");
    drop(tx);

    let frame = tokio::time::timeout(Duration::from_millis(200), source.recv_frame_inner())
        .await
        .expect("continuation frame should assemble")
        .expect("continuation frame should still be emitted");
    assert!(!frame.is_keyframe);
    assert!(!source.waiting_for_recovery_keyframe());
    assert!(transport_observation_rx.try_recv().is_err());
}

#[test]
fn waiting_recovery_keyframe_timeout_triggers_retry_request() {
    let (_tx, mut transport_observation_rx, mut source) = make_video_source_for_test();
    source.set_waiting_for_recovery_keyframe(true);
    // 强制 next_retry_at_ms 为过去时间，确保触发重试。
    source.next_recovery_keyframe_retry_at_ms = Some(0.0);

    let before_ms = now_ms_f64();
    source.maybe_retry_waiting_recovery_keyframe(before_ms);

    assert_eq!(source.recovery_keyframe_retry_count, 1);
    // next_retry_at_ms 应推进到 before_ms + retry_interval，用固定基准比较避免时间竞争。
    assert!(
        source
            .next_recovery_keyframe_retry_at_ms
            .is_some_and(|at| at > before_ms)
    );
    assert!(matches!(
        transport_observation_rx.try_recv(),
        Ok(TransportObservation::Loss(
            TransportLossObservation::RecoveryKeyframeRequested
        ))
    ));
}

#[test]
fn waiting_recovery_keyframe_stops_retrying_after_max_count() {
    let (_tx, mut transport_observation_rx, mut source) = make_video_source_for_test();
    source.set_waiting_for_recovery_keyframe(true);

    // 把 retry_count 推到上限前一次。
    use crate::transport::rtc::stream::video_source::nack_policy::RECOVERY_KEYFRAME_RETRY_MAX_COUNT;
    source.recovery_keyframe_retry_count = RECOVERY_KEYFRAME_RETRY_MAX_COUNT - 1;
    source.next_recovery_keyframe_retry_at_ms = Some(0.0);
    let now = now_ms_f64();

    // 最后一次合法重试。
    source.maybe_retry_waiting_recovery_keyframe(now);
    assert_eq!(
        source.recovery_keyframe_retry_count,
        RECOVERY_KEYFRAME_RETRY_MAX_COUNT
    );
    assert!(transport_observation_rx.try_recv().is_ok());

    // 再次触发：已达上限，不再发请求，next_retry_at_ms 被清空。
    source.next_recovery_keyframe_retry_at_ms = Some(0.0);
    source.maybe_retry_waiting_recovery_keyframe(now);
    assert_eq!(
        source.recovery_keyframe_retry_count,
        RECOVERY_KEYFRAME_RETRY_MAX_COUNT
    );
    assert!(source.next_recovery_keyframe_retry_at_ms.is_none());
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
        stats.latest_remote_answer_observation = Some(startup_h264_answer_without_sprop());
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
    assert!(transport_observation_rx.try_recv().is_err());
}

#[tokio::test]
async fn startup_h264_without_sprop_and_audio_only_emits_single_followup_bootstrap_request() {
    let (tx, mut transport_observation_rx, mut source) = make_video_source_for_test();
    source.runtime_stats.update(|stats| {
        stats.session_phase = Some("priming".to_string());
        stats.transport_state = XbxEngineTransportStateDto::Connected;
        stats.latest_remote_answer_observation = Some(startup_h264_answer_without_sprop());
        stats.latest_audio_playout_time_ms = Some(16.0);
        stats.latest_video_track_status = Some(XbxEngineVideoTrackStatus {
            state: "audioOnly".to_string(),
            video_width: None,
            video_height: None,
            mime_type: Some("video/h264".to_string()),
            transport_state: XbxEngineTransportStateDto::Connected,
            video_bytes_total: 0,
            video_packet_count_total: 0,
            audio_bytes_total: 2_048,
            observed_at_ms: 18.0,
        });
    });

    let non_idr = bootstrap_non_idr_nalu();
    tx.send(make_video_rtp_packet(100, 9_001, true, &non_idr))
        .await
        .expect("non-idr packet should enqueue");
    tx.send(make_video_rtp_packet(101, 9_017, true, &non_idr))
        .await
        .expect("follow-up packet should flush previous sample");
    drop(tx);

    let frame = tokio::time::timeout(Duration::from_millis(200), source.recv_frame_inner())
        .await
        .expect("reader should finish after rx closes");
    assert!(frame.is_none());

    let first = tokio::time::timeout(Duration::from_millis(50), transport_observation_rx.recv())
        .await
        .expect("initial bootstrap request observation should be emitted")
        .expect("observation should exist");
    assert_eq!(
        first,
        TransportObservation::Loss(TransportLossObservation::RecoveryKeyframeRequested)
    );

    let second = tokio::time::timeout(Duration::from_millis(50), transport_observation_rx.recv())
        .await
        .expect("follow-up bootstrap request observation should be emitted")
        .expect("observation should exist");
    assert_eq!(
        second,
        TransportObservation::Loss(TransportLossObservation::RecoveryKeyframeRequested)
    );

    let third = tokio::time::timeout(Duration::from_millis(50), transport_observation_rx.recv())
        .await
        .expect("await-recovery observation should be emitted")
        .expect("observation should exist");
    assert_eq!(
        third,
        TransportObservation::Admission(TransportAdmissionObservation::AwaitRecoveryKeyframe)
    );
    assert!(transport_observation_rx.try_recv().is_err());
}

#[tokio::test]
async fn first_frame_acquisition_priority_non_idr_with_committed_parameter_sets_emits_followup_and_await_recovery_before_first_frame_edge(
) {
    let (tx, mut transport_observation_rx, mut source) = make_video_source_for_test();
    source.runtime_stats.update(|stats| {
        stats.session_phase = Some("priming".to_string());
        stats.transport_state = XbxEngineTransportStateDto::Connected;
        stats.latest_remote_answer_observation = Some(startup_h264_answer_without_sprop());
        stats.latest_audio_playout_time_ms = Some(16.0);
        stats.latest_video_track_status = Some(XbxEngineVideoTrackStatus {
            state: "remoteTrackAttached".to_string(),
            video_width: Some(1920),
            video_height: Some(1080),
            mime_type: Some("video/h264".to_string()),
            transport_state: XbxEngineTransportStateDto::Connected,
            video_bytes_total: 4_096,
            video_packet_count_total: 32,
            audio_bytes_total: 2_048,
            observed_at_ms: 18.0,
        });
    });
    source
        .h264_inspector
        .seed_committed_parameter_sets_if_absent(&bootstrap_sps_nalu(), &bootstrap_pps_nalu())
        .expect("committed parameter sets should seed successfully");

    let non_idr = bootstrap_non_idr_nalu();
    tx.send(make_video_rtp_packet(100, 9_001, true, &non_idr))
        .await
        .expect("non-idr packet should enqueue");
    tx.send(make_video_rtp_packet(101, 9_017, true, &non_idr))
        .await
        .expect("follow-up packet should flush previous sample");
    drop(tx);

    let frame = tokio::time::timeout(Duration::from_millis(200), source.recv_frame_inner())
        .await
        .expect("reader should finish after rx closes");
    assert!(frame.is_none());

    let first = tokio::time::timeout(Duration::from_millis(50), transport_observation_rx.recv())
        .await
        .expect("initial bootstrap request observation should be emitted")
        .expect("observation should exist");
    assert_eq!(
        first,
        TransportObservation::Loss(TransportLossObservation::RecoveryKeyframeRequested)
    );

    let second = tokio::time::timeout(Duration::from_millis(50), transport_observation_rx.recv())
        .await
        .expect("follow-up bootstrap request observation should be emitted")
        .expect("observation should exist");
    assert_eq!(
        second,
        TransportObservation::Loss(TransportLossObservation::RecoveryKeyframeRequested)
    );

    let third = tokio::time::timeout(Duration::from_millis(50), transport_observation_rx.recv())
        .await
        .expect("await-recovery observation should be emitted")
        .expect("observation should exist");
    assert_eq!(
        third,
        TransportObservation::Admission(TransportAdmissionObservation::AwaitRecoveryKeyframe)
    );
    assert!(transport_observation_rx.try_recv().is_err());
}

#[tokio::test]
async fn first_frame_acquisition_followup_request_is_disabled_after_first_frame_feedback() {
    let (tx, mut transport_observation_rx, mut source) = make_video_source_for_test();
    source.runtime_stats.update(|stats| {
        stats.session_phase = Some("priming".to_string());
        stats.transport_state = XbxEngineTransportStateDto::Connected;
        stats.latest_remote_answer_observation = Some(startup_h264_answer_without_sprop());
        stats.latest_audio_playout_time_ms = Some(16.0);
        stats.latest_video_decode_ok_time_ms = Some(14.0);
        stats.latest_video_host_present_time_ms = Some(15.0);
        stats.video_present_epoch = 1;
        stats.latest_video_track_status = Some(XbxEngineVideoTrackStatus {
            state: "audioOnly".to_string(),
            video_width: None,
            video_height: None,
            mime_type: Some("video/h264".to_string()),
            transport_state: XbxEngineTransportStateDto::Connected,
            video_bytes_total: 512,
            video_packet_count_total: 2,
            audio_bytes_total: 2_048,
            observed_at_ms: 18.0,
        });
    });

    let non_idr = bootstrap_non_idr_nalu();
    tx.send(make_video_rtp_packet(100, 9_001, true, &non_idr))
        .await
        .expect("non-idr packet should enqueue");
    tx.send(make_video_rtp_packet(101, 9_017, true, &non_idr))
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
    assert!(transport_observation_rx.try_recv().is_err());
}

#[tokio::test]
async fn first_frame_acquisition_followup_stays_enabled_before_first_frame_even_with_committed_parameter_sets(
) {
    let (tx, mut transport_observation_rx, mut source) = make_video_source_for_test();
    source.runtime_stats.update(|stats| {
        stats.session_phase = Some("priming".to_string());
        stats.transport_state = XbxEngineTransportStateDto::Connected;
        stats.latest_remote_answer_observation = Some(startup_h264_answer_without_sprop());
        stats.latest_audio_playout_time_ms = Some(16.0);
        stats.latest_video_track_status = Some(XbxEngineVideoTrackStatus {
            state: "remoteTrackAttached".to_string(),
            video_width: None,
            video_height: None,
            mime_type: Some("video/h264".to_string()),
            transport_state: XbxEngineTransportStateDto::Connected,
            video_bytes_total: 18_106,
            video_packet_count_total: 16,
            audio_bytes_total: 2_048,
            observed_at_ms: 18.0,
        });
    });

    send_bootstrap_access_unit(&tx, 100, 9_000).await;
    tx.send(make_video_rtp_packet(
        103,
        9_016,
        true,
        bootstrap_idr_nalu(),
    ))
    .await
    .expect("boundary packet should flush bootstrap sample");

    let bootstrap_frame =
        tokio::time::timeout(Duration::from_millis(200), source.recv_frame_inner())
            .await
            .expect("bootstrap frame should assemble")
            .expect("bootstrap frame should be emitted");
    assert!(bootstrap_frame.is_keyframe);
    assert!(bootstrap_frame.h264.bootstrap_ready);

    let initial = tokio::time::timeout(Duration::from_millis(50), transport_observation_rx.recv())
        .await
        .expect("initial bootstrap request observation should be emitted")
        .expect("observation should exist");
    assert_eq!(
        initial,
        TransportObservation::Loss(TransportLossObservation::RecoveryKeyframeRequested)
    );

    let non_idr = bootstrap_non_idr_nalu();
    tx.send(make_video_rtp_packet(103, 9_016, true, &non_idr))
        .await
        .expect("non-idr packet should enqueue");
    tx.send(make_video_rtp_packet(104, 9_032, true, &non_idr))
        .await
        .expect("follow-up packet should flush previous sample");
    drop(tx);

    let frame = tokio::time::timeout(Duration::from_millis(200), source.recv_frame_inner())
        .await
        .expect("reader should finish after rx closes");
    assert!(frame.is_none());

    let followup = tokio::time::timeout(Duration::from_millis(50), transport_observation_rx.recv())
        .await
        .expect("follow-up bootstrap request observation should be emitted")
        .expect("observation should exist");
    assert_eq!(
        followup,
        TransportObservation::Loss(TransportLossObservation::RecoveryKeyframeRequested)
    );

    let await_recovery =
        tokio::time::timeout(Duration::from_millis(50), transport_observation_rx.recv())
            .await
            .expect("await-recovery observation should be emitted")
            .expect("observation should exist");
    assert_eq!(
        await_recovery,
        TransportObservation::Admission(TransportAdmissionObservation::AwaitRecoveryKeyframe)
    );
    assert!(transport_observation_rx.try_recv().is_err());
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
async fn bootstrap_packets_without_followup_boundary_can_emit_when_early_emit_enabled() {
    // 默认行为保持不变，这里仅验证实验开关打开时可提前出帧。
    let (tx, mut transport_observation_rx, mut source) = make_video_source_for_test();
    source.set_jitter_early_emit_enabled(true);
    send_bootstrap_access_unit(&tx, 100, 9000).await;
    let frame = tokio::time::timeout(Duration::from_millis(200), source.recv_frame_inner())
        .await
        .expect("reader should finish");
    drop(tx);

    let frame = frame.expect("early emit should materialize a frame");
    assert!(frame.is_keyframe);
    assert_eq!(frame.rtp_timestamp, 9000);
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
