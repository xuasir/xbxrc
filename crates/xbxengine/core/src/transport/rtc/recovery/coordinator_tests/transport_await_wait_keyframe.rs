use super::super::{RecoveryCoordinator, RecoveryOwnerSignal};
use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::recovery::escalation::{
    RecoveryAction, VideoEscalationConfig, VideoEscalationController, VideoEscalationReason,
};
use crate::transport::rtc::recovery::runtime_state::unix_now_ms;
use crate::XbxEngineMediaRuntimeStats;
use crate::XbxEngineVideoTrackStatus;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use xbxengine_protocol::XbxEngineTransportStateDto;

use super::harness::test_escalation_controller;

#[test]
fn transport_await_mild_lag_in_steady_stage_stays_suppressed_without_stall_evidence() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.session_phase = Some("steady".to_string());
    stats.video_owner_state = Some("stable-serving".to_string());
    stats.transport_state = XbxEngineTransportStateDto::Connected;
    stats.video_present_fps = 58.0;
    stats.latest_video_host_present_time_ms = Some(now_ms - 40.0);
    stats.latest_video_decode_ok_time_ms = Some(now_ms - 40.0);
    let shared_stats = Mutex::new(stats);
    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(250, 1, 1),
        Instant::now() - Duration::from_secs(3),
        Duration::from_millis(800),
    );

    let first = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryAnchor".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    assert_eq!(first.decision.action, RecoveryAction::RequestKeyframe);

    let second = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryAnchor".to_string(),
            observed_at_ms: now_ms + 130.0,
        },
        &shared_stats,
    );
    assert_eq!(
        second.decision.action,
        RecoveryAction::CoalescedKeyframeInFlight
    );
}

#[test]
fn unsent_requested_keyframe_is_rolled_back_before_transport_await_stage_upgrade() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.session_phase = Some("recovering".to_string());
    stats.transport_recovery_epoch = 19;
    stats.transport_state = XbxEngineTransportStateDto::Connected;
    stats.host_no_pending_pressure_level = Some("critical".to_string());
    stats.host_no_pending_streak = 220;
    stats.latest_video_host_present_time_ms = Some(now_ms - 1_900.0);
    stats.latest_video_decode_ok_time_ms = Some(now_ms - 600.0);
    stats.latest_video_packet_arrival_time_ms = Some(now_ms - 10.0);
    stats.video_decoder_stalled = Some(false);
    stats.video_renderer_stalled = Some(true);
    let shared_stats = Mutex::new(stats);
    let mut coordinator = RecoveryCoordinator::new(
        VideoEscalationController::new(VideoEscalationConfig {
            cooldown_ms: 120,
            keyframe_burst_threshold: 1,
            decoder_reset_burst_threshold: 1,
            keyframe_min_interval_ms: 240,
            escalation_window_ms: 480,
            keyframe_upgrade_min_delay_ms: 0,
        }),
        Instant::now() - Duration::from_secs(3),
        Duration::from_millis(800),
    );

    let first = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryAnchor".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    assert_eq!(first.decision.action, RecoveryAction::RequestKeyframe);
    assert_eq!(first.budget_after.keyframe_budget_used, 0);

    RuntimeStatsSink::update_shared(&shared_stats, |stats| {
        stats.latest_keyframe_request_episode =
            Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                episode_id: first.decision.observation_id,
                request_reason: Some("transportAwaitRecoveryAnchor".to_string()),
                request_kind: None,
                status: "requested".to_string(),
                status_detail: None,
                requested_at_ms: now_ms,
                sent_at_ms: None,
                deadline_at_ms: Some(now_ms + 960.0),
                transport_detail: None,
                first_video_packet_at_ms: None,
                first_video_packet_rtp_timestamp: None,
                first_video_packet_is_keyframe: None,
                first_keyframe_packet_at_ms: None,
                first_keyframe_decoded_at_ms: None,
                response_rtp_timestamp: None,
                response_frame_seq: None,
                response_verdict: Some("pending".to_string()),
                lifecycle_phase: None,
                retired_at_ms: None,
            });
    });

    std::thread::sleep(Duration::from_millis(140));
    let second = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryAnchor".to_string(),
            observed_at_ms: now_ms + 140.0,
        },
        &shared_stats,
    );
    assert_eq!(second.budget_before.keyframe_budget_used, 0);
    assert_eq!(
        second.decision.action,
        RecoveryAction::CoalescedKeyframeInFlight
    );
}

#[test]
fn reconfigure_without_failure_evidence_stays_in_local_wait_stage() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.session_phase = Some("recovery-eligible".to_string());
    stats.transport_recovery_epoch = 41;
    stats.transport_state = XbxEngineTransportStateDto::Connected;
    stats.latest_video_host_present_time_ms = Some(now_ms - 1_900.0);
    stats.latest_video_decode_ok_time_ms = Some(now_ms - 1_700.0);
    stats.video_renderer_stalled = Some(true);
    let shared_stats = Mutex::new(stats);
    let mut coordinator = RecoveryCoordinator::new(
        VideoEscalationController::new(VideoEscalationConfig {
            cooldown_ms: 120,
            keyframe_burst_threshold: 1,
            decoder_reset_burst_threshold: 1,
            keyframe_min_interval_ms: 120,
            escalation_window_ms: 240,
            keyframe_upgrade_min_delay_ms: 0,
        }),
        Instant::now() - Duration::from_secs(3),
        Duration::from_millis(800),
    );

    let proposal = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::Reconfigure,
            reason_label: "reconfigure".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    assert_eq!(
        proposal.decision.action,
        RecoveryAction::WaitForDecoderResetBurst
    );
    assert_eq!(proposal.budget_after.decoder_reset_budget_used, 0);
}

#[test]
fn soft_transport_await_signal_stays_in_local_recovery_chain() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.session_phase = Some("recovering".to_string());
    stats.transport_recovery_epoch = 51;
    stats.transport_state = XbxEngineTransportStateDto::Connected;
    stats.latest_video_host_present_time_ms = Some(now_ms - 18.0);
    stats.latest_video_decode_ok_time_ms = Some(now_ms - 12.0);
    stats.latest_video_packet_arrival_time_ms = Some(now_ms - 8.0);
    stats.video_decoder_stalled = Some(false);
    stats.video_renderer_stalled = Some(false);
    stats.latest_video_track_status = Some(XbxEngineVideoTrackStatus {
        state: "remoteTrackAttached".to_string(),
        video_width: Some(1920),
        video_height: Some(1080),
        mime_type: Some("video/H264".to_string()),
        transport_state: XbxEngineTransportStateDto::Connected,
        video_bytes_total: 16_328_400,
        video_packet_count_total: 14_094,
        audio_bytes_total: 237_766,
        observed_at_ms: now_ms - 20.0,
    });
    stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
        observation_id: 5101,
        source_event: "frame-await-recovery-anchor".to_string(),
        gap: None,
        frame: None,
        chain: crate::XbxEngineVideoTimelineChainSnapshot {
            state: "recovering".to_string(),
            reason: Some("awaitingRecoveryAnchor".to_string()),
            chain_break_evidence: None,

            observed_at_ms: now_ms - 10.0,
        },
        observed_at_ms: now_ms - 10.0,
    });
    stats.latest_h264_inspection_observation = Some(crate::XbxEngineH264InspectionObservation {
        observation_id: 5102,
        frame_rtp_timestamp: Some(1_024),
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
        sample_width: None,
        sample_height: None,
        bootstrap_ready: false,
        bootstrap_reject_reason: Some("NonIdrVcl".to_string()),
        admission_accepted: true,
        observed_at_ms: now_ms - 9.0,

        ..Default::default()
    });
    stats.latest_keyframe_request_episode =
        Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
            episode_id: 5100,
            request_reason: Some("transportAwaitRecoveryAnchor".to_string()),
            request_kind: Some("pli".to_string()),
            status: "decoded".to_string(),
            status_detail: None,
            requested_at_ms: now_ms - 180.0,
            sent_at_ms: Some(now_ms - 170.0),
            deadline_at_ms: Some(now_ms + 600.0),
            transport_detail: None,
            first_video_packet_at_ms: None,
            first_video_packet_rtp_timestamp: None,
            first_video_packet_is_keyframe: None,
            first_keyframe_packet_at_ms: Some(now_ms - 120.0),
            first_keyframe_decoded_at_ms: Some(now_ms - 110.0),
            response_rtp_timestamp: Some(1_000),
            response_frame_seq: Some(42),
            response_verdict: Some("on-time".to_string()),
            lifecycle_phase: None,
            retired_at_ms: None,
        });
    let shared_stats = Mutex::new(stats);
    let mut coordinator = RecoveryCoordinator::new(
        VideoEscalationController::new(VideoEscalationConfig {
            cooldown_ms: 120,
            keyframe_burst_threshold: 1,
            decoder_reset_burst_threshold: 1,
            keyframe_min_interval_ms: 120,
            escalation_window_ms: 240,
            keyframe_upgrade_min_delay_ms: 0,
        }),
        Instant::now() - Duration::from_secs(3),
        Duration::from_millis(800),
    );

    let proposal = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryAnchor".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    assert!(matches!(
        proposal.decision.action,
        RecoveryAction::WaitForBurst
            | RecoveryAction::CooldownSuppressed
            | RecoveryAction::RequestDecoderReset
            | RecoveryAction::CoalescedDecoderResetInFlight
    ));
    assert_eq!(proposal.budget_after.keyframe_budget_used, 0);
}

#[test]
fn first_frame_acquisition_transport_await_probe_stays_local() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.session_phase = Some("startup".to_string());
    stats.transport_recovery_epoch = 61;
    stats.transport_state = XbxEngineTransportStateDto::Connected;
    stats.host_display_tick_epoch = 120;
    stats.video_present_epoch = 0;
    stats.video_present_submit_count_total = 0;
    stats.latest_video_packet_arrival_time_ms = Some(now_ms - 8.0);
    stats.first_video_packet_arrival_time_ms = Some(now_ms - 100.0);
    stats.latest_video_track_status = Some(XbxEngineVideoTrackStatus {
        state: "remoteTrackAttached".to_string(),
        video_width: Some(1920),
        video_height: Some(1080),
        mime_type: Some("video/H264".to_string()),
        transport_state: XbxEngineTransportStateDto::Connected,
        video_bytes_total: 512_000,
        video_packet_count_total: 4_096,
        audio_bytes_total: 0,
        observed_at_ms: now_ms - 12.0,
    });
    stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
        observation_id: 6101,
        source_event: "frame-inspection-rejected-await-anchor".to_string(),
        gap: None,
        frame: None,
        chain: crate::XbxEngineVideoTimelineChainSnapshot {
            state: "recovering".to_string(),
            reason: Some("awaitingRecoveryAnchor".to_string()),
            chain_break_evidence: None,

            observed_at_ms: now_ms - 10.0,
        },
        observed_at_ms: now_ms - 10.0,
    });
    stats.latest_h264_inspection_observation = Some(crate::XbxEngineH264InspectionObservation {
        observation_id: 6102,
        frame_rtp_timestamp: Some(7_024),
        nal_types: vec!["SliceLayerWithoutPartitioningNonIdr".to_string()],
        nal_count: 1,
        vcl_nal_count: 1,
        has_inband_sps: false,
        has_inband_pps: false,
        committed_sps_present: false,
        committed_pps_present: false,
        slice_headers_valid: true,
        delta_continuation_ready: false,
        parameter_sets_changed: false,
        config_changed: false,
        is_idr: false,
        sample_width: None,
        sample_height: None,
        bootstrap_ready: false,
        bootstrap_reject_reason: Some("bootstrapMissingSps".to_string()),
        admission_accepted: true,
        observed_at_ms: now_ms - 9.0,

        ..Default::default()
    });
    let shared_stats = Mutex::new(stats);
    let mut coordinator = RecoveryCoordinator::new(
        VideoEscalationController::new(VideoEscalationConfig {
            cooldown_ms: 120,
            keyframe_burst_threshold: 1,
            decoder_reset_burst_threshold: 1,
            keyframe_min_interval_ms: 120,
            escalation_window_ms: 240,
            keyframe_upgrade_min_delay_ms: 0,
        }),
        Instant::now() - Duration::from_secs(3),
        Duration::from_millis(800),
    );

    let first = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryAnchor".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    assert_eq!(first.decision.action, RecoveryAction::RequestKeyframe);

    RuntimeStatsSink::update_shared(&shared_stats, |stats| {
        stats.latest_keyframe_request_episode =
            Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                episode_id: first.decision.observation_id,
                request_reason: Some("transportAwaitRecoveryAnchor".to_string()),
                request_kind: Some("pli".to_string()),
                status: "sent".to_string(),
                status_detail: None,
                requested_at_ms: now_ms,
                sent_at_ms: Some(now_ms + 2.0),
                deadline_at_ms: Some(now_ms + 600.0),
                transport_detail: None,
                first_video_packet_at_ms: None,
                first_video_packet_rtp_timestamp: None,
                first_video_packet_is_keyframe: None,
                first_keyframe_packet_at_ms: None,
                first_keyframe_decoded_at_ms: None,
                response_rtp_timestamp: None,
                response_frame_seq: None,
                response_verdict: Some("pending".to_string()),
                lifecycle_phase: None,
                retired_at_ms: None,
            });
    });

    let second = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryAnchor".to_string(),
            observed_at_ms: now_ms + 140.0,
        },
        &shared_stats,
    );
    assert!(matches!(
        second.decision.action,
        RecoveryAction::WaitForBurst
            | RecoveryAction::CooldownSuppressed
            | RecoveryAction::CoalescedKeyframeInFlight
    ));
}

#[test]
fn first_frame_acquisition_transport_await_stall_still_stays_in_keyframe_domain() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.session_phase = Some("steady".to_string());
    stats.transport_recovery_epoch = 63;
    stats.transport_state = XbxEngineTransportStateDto::Connected;
    stats.host_display_tick_epoch = 120;
    stats.video_present_epoch = 0;
    stats.video_present_submit_count_total = 0;
    stats.video_decoder_stalled = Some(true);
    stats.video_renderer_stalled = Some(true);
    stats.first_video_packet_arrival_time_ms = Some(now_ms - 4_000.0);
    stats.latest_video_track_status = Some(XbxEngineVideoTrackStatus {
        state: "remoteTrackAttached".to_string(),
        video_width: Some(1920),
        video_height: Some(1080),
        mime_type: Some("video/H264".to_string()),
        transport_state: XbxEngineTransportStateDto::Connected,
        video_bytes_total: 512_000,
        video_packet_count_total: 4_096,
        audio_bytes_total: 0,
        observed_at_ms: now_ms - 4_000.0,
    });
    stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
        observation_id: 6301,
        source_event: "frame-inspection-rejected-await-anchor".to_string(),
        gap: None,
        frame: None,
        chain: crate::XbxEngineVideoTimelineChainSnapshot {
            state: "recovering".to_string(),
            reason: Some("awaitingRecoveryAnchor".to_string()),
            chain_break_evidence: None,

            observed_at_ms: now_ms - 4_000.0,
        },
        observed_at_ms: now_ms - 4_000.0,
    });
    let shared_stats = Mutex::new(stats);
    let mut coordinator = RecoveryCoordinator::new(
        VideoEscalationController::new(VideoEscalationConfig {
            cooldown_ms: 120,
            keyframe_burst_threshold: 1,
            decoder_reset_burst_threshold: 1,
            keyframe_min_interval_ms: 120,
            escalation_window_ms: 240,
            keyframe_upgrade_min_delay_ms: 0,
        }),
        Instant::now() - Duration::from_secs(3),
        Duration::from_millis(800),
    );

    let first = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryAnchor".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    assert_eq!(first.decision.action, RecoveryAction::RequestKeyframe);

    RuntimeStatsSink::update_shared(&shared_stats, |stats| {
        stats.latest_keyframe_request_episode =
            Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                episode_id: first.decision.observation_id,
                request_reason: Some("transportAwaitRecoveryAnchor".to_string()),
                request_kind: Some("pli".to_string()),
                status: "sent".to_string(),
                status_detail: None,
                requested_at_ms: now_ms,
                sent_at_ms: Some(now_ms + 1.0),
                deadline_at_ms: Some(now_ms + 600.0),
                transport_detail: None,
                first_video_packet_at_ms: None,
                first_video_packet_rtp_timestamp: None,
                first_video_packet_is_keyframe: None,
                first_keyframe_packet_at_ms: None,
                first_keyframe_decoded_at_ms: None,
                response_rtp_timestamp: None,
                response_frame_seq: None,
                response_verdict: Some("pending".to_string()),
                lifecycle_phase: None,
                retired_at_ms: None,
            });
    });

    let second = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryAnchor".to_string(),
            observed_at_ms: now_ms + 500.0,
        },
        &shared_stats,
    );
    assert!(matches!(
        second.decision.action,
        RecoveryAction::WaitForBurst
            | RecoveryAction::CooldownSuppressed
            | RecoveryAction::CoalescedKeyframeInFlight
            | RecoveryAction::RequestKeyframe
    ));
    assert_ne!(second.decision.action, RecoveryAction::RequestDecoderReset);
    assert_ne!(
        second.decision.action,
        RecoveryAction::RequestReconnectCandidate
    );
}

#[test]
fn startup_non_idr_transport_await_probe_stays_local_before_first_frame() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.session_phase = Some("priming".to_string());
    stats.transport_recovery_epoch = 62;
    stats.transport_state = XbxEngineTransportStateDto::Connected;
    stats.host_display_tick_epoch = 120;
    stats.video_present_epoch = 0;
    stats.video_present_submit_count_total = 0;
    stats.latest_video_packet_arrival_time_ms = Some(now_ms - 8.0);
    stats.first_video_packet_arrival_time_ms = Some(now_ms - 100.0);
    stats.video_decoder_stalled = Some(false);
    stats.video_renderer_stalled = Some(false);
    stats.latest_video_track_status = Some(XbxEngineVideoTrackStatus {
        state: "remoteTrackAttached".to_string(),
        video_width: Some(1920),
        video_height: Some(1080),
        mime_type: Some("video/H264".to_string()),
        transport_state: XbxEngineTransportStateDto::Connected,
        video_bytes_total: 512_000,
        video_packet_count_total: 4_096,
        audio_bytes_total: 0,
        observed_at_ms: now_ms - 12.0,
    });
    stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
        observation_id: 6201,
        source_event: "frame-await-recovery-anchor".to_string(),
        gap: None,
        frame: None,
        chain: crate::XbxEngineVideoTimelineChainSnapshot {
            state: "recovering".to_string(),
            reason: Some("awaitingRecoveryAnchor".to_string()),
            chain_break_evidence: None,

            observed_at_ms: now_ms - 10.0,
        },
        observed_at_ms: now_ms - 10.0,
    });
    stats.latest_h264_inspection_observation = Some(crate::XbxEngineH264InspectionObservation {
        observation_id: 6202,
        frame_rtp_timestamp: Some(8_024),
        nal_types: vec!["SliceLayerWithoutPartitioningNonIdr".to_string()],
        nal_count: 1,
        vcl_nal_count: 1,
        has_inband_sps: false,
        has_inband_pps: false,
        committed_sps_present: true,
        committed_pps_present: true,
        slice_headers_valid: true,
        delta_continuation_ready: false,
        parameter_sets_changed: false,
        config_changed: false,
        is_idr: false,
        sample_width: None,
        sample_height: None,
        bootstrap_ready: false,
        bootstrap_reject_reason: Some("NonIdrVcl".to_string()),
        admission_accepted: true,
        observed_at_ms: now_ms - 9.0,

        ..Default::default()
    });
    let shared_stats = Mutex::new(stats);
    let mut coordinator = RecoveryCoordinator::new(
        VideoEscalationController::new(VideoEscalationConfig {
            cooldown_ms: 120,
            keyframe_burst_threshold: 1,
            decoder_reset_burst_threshold: 1,
            keyframe_min_interval_ms: 120,
            escalation_window_ms: 240,
            keyframe_upgrade_min_delay_ms: 0,
        }),
        Instant::now() - Duration::from_secs(3),
        Duration::from_millis(800),
    );

    let first = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryAnchor".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    assert_eq!(first.decision.action, RecoveryAction::RequestKeyframe);

    RuntimeStatsSink::update_shared(&shared_stats, |stats| {
        stats.latest_keyframe_request_episode =
            Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                episode_id: first.decision.observation_id,
                request_reason: Some("transportAwaitRecoveryAnchor".to_string()),
                request_kind: Some("pli".to_string()),
                status: "packet-seen".to_string(),
                status_detail: None,
                requested_at_ms: now_ms,
                sent_at_ms: Some(now_ms + 2.0),
                deadline_at_ms: Some(now_ms + 600.0),
                transport_detail: None,
                first_video_packet_at_ms: None,
                first_video_packet_rtp_timestamp: None,
                first_video_packet_is_keyframe: None,
                first_keyframe_packet_at_ms: Some(now_ms + 18.0),
                first_keyframe_decoded_at_ms: None,
                response_rtp_timestamp: Some(8_024),
                response_frame_seq: None,
                response_verdict: Some("on-time".to_string()),
                lifecycle_phase: None,
                retired_at_ms: None,
            });
    });

    let second = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryAnchor".to_string(),
            observed_at_ms: now_ms + 280.0,
        },
        &shared_stats,
    );
    assert!(
        matches!(
            second.decision.action,
            RecoveryAction::WaitForBurst
                | RecoveryAction::CooldownSuppressed
                | RecoveryAction::CoalescedKeyframeInFlight
                | RecoveryAction::RequestKeyframe
        ),
        "actual action: {:?}",
        second.decision.action
    );
}

#[test]
fn first_frame_acquisition_missing_pps_packet_seen_stays_local_before_first_frame() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.session_phase = Some("priming".to_string());
    stats.transport_recovery_epoch = 63;
    stats.transport_state = XbxEngineTransportStateDto::Connected;
    stats.host_display_tick_epoch = 120;
    stats.video_present_epoch = 0;
    stats.video_present_submit_count_total = 0;
    stats.latest_video_packet_arrival_time_ms = Some(now_ms - 8.0);
    stats.first_video_packet_arrival_time_ms = Some(now_ms - 100.0);
    stats.video_decoder_stalled = Some(false);
    stats.video_renderer_stalled = Some(false);
    stats.latest_video_track_status = Some(XbxEngineVideoTrackStatus {
        state: "remoteTrackAttached".to_string(),
        video_width: Some(1920),
        video_height: Some(1080),
        mime_type: Some("video/H264".to_string()),
        transport_state: XbxEngineTransportStateDto::Connected,
        video_bytes_total: 512_000,
        video_packet_count_total: 4_096,
        audio_bytes_total: 0,
        observed_at_ms: now_ms - 12.0,
    });
    stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
        observation_id: 6301,
        source_event: "frame-inspection-rejected-await-anchor".to_string(),
        gap: None,
        frame: None,
        chain: crate::XbxEngineVideoTimelineChainSnapshot {
            state: "recovering".to_string(),
            reason: Some("awaitingRecoveryAnchor".to_string()),
            chain_break_evidence: None,

            observed_at_ms: now_ms - 10.0,
        },
        observed_at_ms: now_ms - 10.0,
    });
    stats.latest_h264_inspection_observation = Some(crate::XbxEngineH264InspectionObservation {
        observation_id: 6302,
        frame_rtp_timestamp: Some(9_024),
        nal_types: vec!["SliceLayerWithoutPartitioningNonIdr".to_string()],
        nal_count: 1,
        vcl_nal_count: 1,
        has_inband_sps: false,
        has_inband_pps: false,
        committed_sps_present: true,
        committed_pps_present: false,
        slice_headers_valid: true,
        delta_continuation_ready: false,
        parameter_sets_changed: false,
        config_changed: false,
        is_idr: false,
        sample_width: None,
        sample_height: None,
        bootstrap_ready: false,
        bootstrap_reject_reason: Some("bootstrapMissingPps".to_string()),
        admission_accepted: true,
        observed_at_ms: now_ms - 9.0,

        ..Default::default()
    });
    stats.latest_keyframe_request_episode =
        Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
            episode_id: 6300,
            request_reason: Some("transportAwaitRecoveryAnchor".to_string()),
            request_kind: Some("pli".to_string()),
            status: "packet-seen".to_string(),
            status_detail: None,
            requested_at_ms: now_ms - 300.0,
            sent_at_ms: Some(now_ms - 290.0),
            deadline_at_ms: Some(now_ms + 600.0),
            transport_detail: None,
            first_video_packet_at_ms: None,
            first_video_packet_rtp_timestamp: None,
            first_video_packet_is_keyframe: None,
            first_keyframe_packet_at_ms: Some(now_ms - 260.0),
            first_keyframe_decoded_at_ms: None,
            response_rtp_timestamp: Some(9_024),
            response_frame_seq: None,
            response_verdict: Some("on-time".to_string()),
            lifecycle_phase: None,
            retired_at_ms: None,
        });
    stats.latest_anchor_candidate_ledger = Some(crate::XbxEngineAnchorCandidateLedger {
        state: crate::XbxEngineAnchorCandidateState::Rejected,
        source_event: "frame-inspection-rejected-await-anchor".to_string(),
        frame_rtp_timestamp: Some(9_024),
        recovery_epoch: 63,
        failure_reason: Some(
            crate::XbxEngineAnchorCandidateFailureReason::InspectionRejectedMissingPps,
        ),
        observed_at_ms: now_ms - 120.0,
    });

    let shared_stats = Mutex::new(stats);
    let mut coordinator = RecoveryCoordinator::new(
        VideoEscalationController::new(VideoEscalationConfig {
            cooldown_ms: 120,
            keyframe_burst_threshold: 1,
            decoder_reset_burst_threshold: 1,
            keyframe_min_interval_ms: 120,
            escalation_window_ms: 240,
            keyframe_upgrade_min_delay_ms: 0,
        }),
        Instant::now() - Duration::from_secs(3),
        Duration::from_millis(800),
    );

    let first = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryAnchor".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    assert_ne!(first.decision.action, RecoveryAction::RequestDecoderReset);

    let second = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryAnchor".to_string(),
            observed_at_ms: now_ms + 280.0,
        },
        &shared_stats,
    );
    assert_ne!(second.decision.action, RecoveryAction::RequestDecoderReset);
}

#[test]
fn post_first_frame_bootstrap_missing_pps_can_upgrade_to_decoder_reset() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.session_phase = Some("recovering".to_string());
    stats.transport_recovery_epoch = 64;
    stats.transport_state = XbxEngineTransportStateDto::Connected;
    stats.host_display_tick_epoch = 120;
    stats.video_present_epoch = 1;
    stats.video_present_submit_count_total = 1;
    stats.latest_video_host_present_time_ms = Some(now_ms - 20.0);
    stats.latest_video_decode_ok_time_ms = Some(now_ms - 20.0);
    stats.latest_video_packet_arrival_time_ms = Some(now_ms - 8.0);
    stats.video_decoder_stalled = Some(false);
    stats.video_renderer_stalled = Some(false);
    stats.latest_video_track_status = Some(XbxEngineVideoTrackStatus {
        state: "remoteTrackAttached".to_string(),
        video_width: Some(1920),
        video_height: Some(1080),
        mime_type: Some("video/H264".to_string()),
        transport_state: XbxEngineTransportStateDto::Connected,
        video_bytes_total: 512_000,
        video_packet_count_total: 4_096,
        audio_bytes_total: 0,
        observed_at_ms: now_ms - 12.0,
    });
    stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
        observation_id: 6401,
        source_event: "frame-inspection-rejected-await-anchor".to_string(),
        gap: None,
        frame: None,
        chain: crate::XbxEngineVideoTimelineChainSnapshot {
            state: "recovering".to_string(),
            reason: Some("awaitingRecoveryAnchor".to_string()),
            chain_break_evidence: None,

            observed_at_ms: now_ms - 10.0,
        },
        observed_at_ms: now_ms - 10.0,
    });
    stats.latest_h264_inspection_observation = Some(crate::XbxEngineH264InspectionObservation {
        observation_id: 6402,
        frame_rtp_timestamp: Some(9_124),
        nal_types: vec!["SliceLayerWithoutPartitioningNonIdr".to_string()],
        nal_count: 1,
        vcl_nal_count: 1,
        has_inband_sps: false,
        has_inband_pps: false,
        committed_sps_present: true,
        committed_pps_present: false,
        slice_headers_valid: true,
        delta_continuation_ready: false,
        parameter_sets_changed: false,
        config_changed: false,
        is_idr: false,
        sample_width: None,
        sample_height: None,
        bootstrap_ready: false,
        bootstrap_reject_reason: Some("bootstrapMissingPps".to_string()),
        admission_accepted: true,
        observed_at_ms: now_ms - 9.0,

        ..Default::default()
    });
    let shared_stats = Mutex::new(stats);
    let mut coordinator = RecoveryCoordinator::new(
        VideoEscalationController::new(VideoEscalationConfig {
            cooldown_ms: 120,
            keyframe_burst_threshold: 1,
            decoder_reset_burst_threshold: 1,
            keyframe_min_interval_ms: 120,
            escalation_window_ms: 240,
            keyframe_upgrade_min_delay_ms: 0,
        }),
        Instant::now() - Duration::from_secs(3),
        Duration::from_millis(800),
    );

    let first = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryAnchor".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    assert_eq!(first.decision.action, RecoveryAction::RequestKeyframe);

    RuntimeStatsSink::update_shared(&shared_stats, |stats| {
        stats.latest_keyframe_request_episode =
            Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                episode_id: first.decision.observation_id,
                request_reason: Some("transportAwaitRecoveryAnchor".to_string()),
                request_kind: Some("pli".to_string()),
                status: "packet-seen".to_string(),
                status_detail: None,
                requested_at_ms: now_ms,
                sent_at_ms: Some(now_ms + 2.0),
                deadline_at_ms: Some(now_ms + 600.0),
                transport_detail: None,
                first_video_packet_at_ms: None,
                first_video_packet_rtp_timestamp: None,
                first_video_packet_is_keyframe: None,
                first_keyframe_packet_at_ms: Some(now_ms + 18.0),
                first_keyframe_decoded_at_ms: None,
                response_rtp_timestamp: Some(9_124),
                response_frame_seq: None,
                response_verdict: Some("on-time".to_string()),
                lifecycle_phase: None,
                retired_at_ms: None,
            });
        stats.latest_anchor_candidate_ledger = Some(crate::XbxEngineAnchorCandidateLedger {
            state: crate::XbxEngineAnchorCandidateState::Rejected,
            source_event: "frame-inspection-rejected-await-anchor".to_string(),
            frame_rtp_timestamp: Some(9_124),
            recovery_epoch: 64,
            failure_reason: Some(
                crate::XbxEngineAnchorCandidateFailureReason::InspectionRejectedMissingPps,
            ),
            observed_at_ms: now_ms + 120.0,
        });
    });

    let second = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryAnchor".to_string(),
            observed_at_ms: now_ms + 280.0,
        },
        &shared_stats,
    );
    assert_eq!(second.decision.action, RecoveryAction::RequestDecoderReset);
}

#[test]
fn weak_transport_await_streak_does_not_preload_decoder_reset_upgrade() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.session_phase = Some("recovering".to_string());
    stats.transport_recovery_epoch = 62;
    stats.transport_state = XbxEngineTransportStateDto::Connected;
    stats.latest_video_host_present_time_ms = Some(now_ms - 20.0);
    stats.latest_video_decode_ok_time_ms = Some(now_ms - 14.0);
    stats.latest_video_packet_arrival_time_ms = Some(now_ms - 8.0);
    stats.video_decoder_stalled = Some(false);
    stats.video_renderer_stalled = Some(false);
    stats.latest_video_track_status = Some(XbxEngineVideoTrackStatus {
        state: "remoteTrackAttached".to_string(),
        video_width: Some(1920),
        video_height: Some(1080),
        mime_type: Some("video/H264".to_string()),
        transport_state: XbxEngineTransportStateDto::Connected,
        video_bytes_total: 16_328_400,
        video_packet_count_total: 14_094,
        audio_bytes_total: 237_766,
        observed_at_ms: now_ms - 20.0,
    });
    stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
        observation_id: 6201,
        source_event: "frame-await-recovery-anchor".to_string(),
        gap: None,
        frame: None,
        chain: crate::XbxEngineVideoTimelineChainSnapshot {
            state: "recovering".to_string(),
            reason: Some("awaitingRecoveryAnchor".to_string()),
            chain_break_evidence: None,

            observed_at_ms: now_ms - 10.0,
        },
        observed_at_ms: now_ms - 10.0,
    });
    let shared_stats = Mutex::new(stats);
    let mut coordinator = RecoveryCoordinator::new(
        VideoEscalationController::new(VideoEscalationConfig {
            cooldown_ms: 120,
            keyframe_burst_threshold: 1,
            decoder_reset_burst_threshold: 1,
            keyframe_min_interval_ms: 120,
            escalation_window_ms: 240,
            keyframe_upgrade_min_delay_ms: 0,
        }),
        Instant::now() - Duration::from_secs(3),
        Duration::from_millis(800),
    );

    let first = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryAnchor".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    assert_eq!(first.decision.action, RecoveryAction::RequestKeyframe);

    RuntimeStatsSink::update_shared(&shared_stats, |stats| {
        stats.latest_keyframe_request_episode =
            Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                episode_id: first.decision.observation_id,
                request_reason: Some("transportAwaitRecoveryAnchor".to_string()),
                request_kind: Some("pli".to_string()),
                status: "sent".to_string(),
                status_detail: None,
                requested_at_ms: now_ms,
                sent_at_ms: Some(now_ms + 2.0),
                deadline_at_ms: Some(now_ms + 600.0),
                transport_detail: None,
                first_video_packet_at_ms: None,
                first_video_packet_rtp_timestamp: None,
                first_video_packet_is_keyframe: None,
                first_keyframe_packet_at_ms: None,
                first_keyframe_decoded_at_ms: None,
                response_rtp_timestamp: None,
                response_frame_seq: None,
                response_verdict: Some("pending".to_string()),
                lifecycle_phase: None,
                retired_at_ms: None,
            });
    });

    let second = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryAnchor".to_string(),
            observed_at_ms: now_ms + 140.0,
        },
        &shared_stats,
    );
    assert!(matches!(
        second.decision.action,
        RecoveryAction::WaitForBurst
            | RecoveryAction::CooldownSuppressed
            | RecoveryAction::CoalescedKeyframeInFlight
    ));

    RuntimeStatsSink::update_shared(&shared_stats, |stats| {
        stats.latest_video_host_present_time_ms = Some(now_ms - 2_000.0);
        stats.latest_video_decode_ok_time_ms = Some(now_ms - 1_800.0);
        stats.video_renderer_stalled = Some(true);
        stats.latest_anchor_candidate_ledger = Some(crate::XbxEngineAnchorCandidateLedger {
            state: crate::XbxEngineAnchorCandidateState::Rejected,
            source_event: "frame-await-recovery-anchor".to_string(),
            frame_rtp_timestamp: Some(62_001),
            recovery_epoch: 62,
            failure_reason: Some(
                crate::XbxEngineAnchorCandidateFailureReason::AwaitingRecoveryKeyframe,
            ),
            observed_at_ms: now_ms + 280.0,
        });
    });

    let third = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryAnchor".to_string(),
            observed_at_ms: now_ms + 280.0,
        },
        &shared_stats,
    );
    assert!(
        matches!(
            third.decision.action,
            RecoveryAction::RequestKeyframe
                | RecoveryAction::CoalescedKeyframeInFlight
                | RecoveryAction::RequestDecoderReset
        ),
        "unexpected action: {:?}",
        third.decision.action
    );
    assert_ne!(
        third.decision.action,
        RecoveryAction::RequestReconnectCandidate
    );
}

#[test]
fn reconfigure_with_explicit_failure_evidence_can_request_decoder_reset() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.session_phase = Some("recovery-eligible".to_string());
    stats.transport_recovery_epoch = 42;
    stats.transport_recovery_epoch_at_last_escalation = 41;
    stats.transport_state = XbxEngineTransportStateDto::Connected;
    stats.latest_video_host_present_time_ms = Some(now_ms - 1_900.0);
    stats.latest_video_decode_ok_time_ms = Some(now_ms - 1_700.0);
    stats.video_renderer_stalled = Some(true);
    stats.latest_video_escalation_observation = Some(crate::XbxEngineVideoEscalationObservation {
        observation_id: 4201,
        reason: "reconfigure".to_string(),
        action: "requestDecoderReset".to_string(),
        recovery_stage: "recovery-eligible".to_string(),
        recovery_chain_value: "config".to_string(),
        recovery_failure_cost: "high".to_string(),
        recovery_window_source: "decoder-reset-window".to_string(),
        observed_at_ms: now_ms - 300.0,
    });
    let shared_stats = Mutex::new(stats);
    let mut coordinator = RecoveryCoordinator::new(
        VideoEscalationController::new(VideoEscalationConfig {
            cooldown_ms: 120,
            keyframe_burst_threshold: 1,
            decoder_reset_burst_threshold: 1,
            keyframe_min_interval_ms: 120,
            escalation_window_ms: 240,
            keyframe_upgrade_min_delay_ms: 0,
        }),
        Instant::now() - Duration::from_secs(3),
        Duration::from_millis(800),
    );

    let proposal = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::Reconfigure,
            reason_label: "reconfigure".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    assert_eq!(
        proposal.decision.action,
        RecoveryAction::CoalescedDecoderResetInFlight
    );
    assert_eq!(proposal.budget_after.decoder_reset_budget_used, 0);
}

#[test]
fn thin_stall_timeout_alone_does_not_keep_transport_await_repeat_suppressed() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_recovery_epoch = 27;
    stats.transport_recovery_epoch_at_last_escalation = 27;
    stats.latest_video_escalation_observation = Some(crate::XbxEngineVideoEscalationObservation {
        observation_id: 901,
        reason: "transportAwaitRecoveryAnchor".to_string(),
        action: "requestKeyframe".to_string(),
        recovery_stage: "rebuilding-supply".to_string(),
        recovery_chain_value: "transportAwaitRecoveryAnchor".to_string(),
        recovery_failure_cost: "medium".to_string(),
        recovery_window_source: "owner".to_string(),
        observed_at_ms: now_ms - 420.0,
    });
    stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
        observation_id: 902,
        source_event: "timeout-stream-thin-stall".to_string(),
        gap: None,
        frame: None,
        chain: crate::XbxEngineVideoTimelineChainSnapshot {
            state: "broken".to_string(),
            reason: Some("streamThinStall".to_string()),
            chain_break_evidence: None,

            observed_at_ms: now_ms - 20.0,
        },
        observed_at_ms: now_ms - 20.0,
    });
    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(120, 1, 1),
        Instant::now() - Duration::from_secs(3),
        Duration::from_millis(800),
    );

    let decision = coordinator.on_reason_with_runtime_stats(
        VideoEscalationReason::TransportAwaitRecoveryKeyframe,
        &Mutex::new(stats),
    );
    assert_ne!(decision.action, RecoveryAction::CoalescedKeyframeInFlight);
}

#[test]
fn sent_pending_keyframe_with_thin_stall_pressure_upgrades_to_decoder_reset() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.session_phase = Some("recovering".to_string());
    stats.transport_recovery_epoch = 28;
    stats.transport_state = XbxEngineTransportStateDto::Connected;
    stats.host_no_pending_pressure_level = Some("critical".to_string());
    stats.host_no_pending_streak = 220;
    stats.latest_video_host_present_time_ms = Some(now_ms - 2_100.0);
    stats.latest_video_decode_ok_time_ms = Some(now_ms - 1_700.0);
    stats.latest_video_packet_arrival_time_ms = Some(now_ms - 20.0);
    stats.video_decoder_stalled = Some(false);
    stats.video_renderer_stalled = Some(true);
    let shared_stats = Mutex::new(stats);
    let mut coordinator = RecoveryCoordinator::new(
        VideoEscalationController::new(VideoEscalationConfig {
            cooldown_ms: 120,
            keyframe_burst_threshold: 1,
            decoder_reset_burst_threshold: 1,
            keyframe_min_interval_ms: 240,
            escalation_window_ms: 480,
            keyframe_upgrade_min_delay_ms: 0,
        }),
        Instant::now() - Duration::from_secs(3),
        Duration::from_millis(800),
    );

    let first = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryAnchor".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    assert_eq!(first.decision.action, RecoveryAction::RequestKeyframe);

    RuntimeStatsSink::update_shared(&shared_stats, |stats| {
        stats.latest_keyframe_request_episode =
            Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                episode_id: first.decision.observation_id,
                request_reason: Some("transportAwaitRecoveryAnchor".to_string()),
                request_kind: None,
                status: "sent".to_string(),
                status_detail: None,
                requested_at_ms: now_ms,
                sent_at_ms: Some(now_ms + 10.0),
                deadline_at_ms: Some(now_ms + 960.0),
                transport_detail: None,
                first_video_packet_at_ms: None,
                first_video_packet_rtp_timestamp: None,
                first_video_packet_is_keyframe: None,
                first_keyframe_packet_at_ms: None,
                first_keyframe_decoded_at_ms: None,
                response_rtp_timestamp: None,
                response_frame_seq: None,
                response_verdict: Some("pending".to_string()),
                lifecycle_phase: None,
                retired_at_ms: None,
            });
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 903,
            source_event: "timeout-stream-thin-stall".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "broken".to_string(),
                reason: Some("streamThinStall".to_string()),
                chain_break_evidence: None,

                observed_at_ms: now_ms + 140.0,
            },
            observed_at_ms: now_ms + 140.0,
        });
    });

    std::thread::sleep(Duration::from_millis(140));
    let second = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryAnchor".to_string(),
            observed_at_ms: now_ms + 140.0,
        },
        &shared_stats,
    );
    assert_eq!(
        second.decision.action,
        RecoveryAction::CoalescedKeyframeInFlight
    );
}

#[test]
fn sent_pending_keyframe_with_recent_rtcp_unavailable_does_not_upgrade_to_decoder_reset() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.session_phase = Some("recovering".to_string());
    stats.transport_recovery_epoch = 29;
    stats.transport_state = XbxEngineTransportStateDto::Connected;
    stats.host_no_pending_pressure_level = Some("critical".to_string());
    stats.host_no_pending_streak = 220;
    stats.latest_video_host_present_time_ms = Some(now_ms - 2_100.0);
    stats.latest_video_decode_ok_time_ms = Some(now_ms - 1_700.0);
    stats.latest_video_packet_arrival_time_ms = Some(now_ms - 20.0);
    stats.video_decoder_stalled = Some(false);
    stats.video_renderer_stalled = Some(true);
    let shared_stats = Mutex::new(stats);
    let mut coordinator = RecoveryCoordinator::new(
        VideoEscalationController::new(VideoEscalationConfig {
            cooldown_ms: 120,
            keyframe_burst_threshold: 1,
            decoder_reset_burst_threshold: 1,
            keyframe_min_interval_ms: 240,
            escalation_window_ms: 480,
            keyframe_upgrade_min_delay_ms: 0,
        }),
        Instant::now() - Duration::from_secs(3),
        Duration::from_millis(800),
    );

    let first = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryAnchor".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    assert_eq!(first.decision.action, RecoveryAction::RequestKeyframe);

    RuntimeStatsSink::update_shared(&shared_stats, |stats| {
        stats.latest_keyframe_request_episode =
            Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                episode_id: first.decision.observation_id,
                request_reason: Some("transportAwaitRecoveryAnchor".to_string()),
                request_kind: Some("pli".to_string()),
                status: "sent".to_string(),
                status_detail: None,
                requested_at_ms: now_ms,
                sent_at_ms: Some(now_ms + 10.0),
                deadline_at_ms: Some(now_ms + 960.0),
                transport_detail: None,
                first_video_packet_at_ms: None,
                first_video_packet_rtp_timestamp: None,
                first_video_packet_is_keyframe: None,
                first_keyframe_packet_at_ms: None,
                first_keyframe_decoded_at_ms: None,
                response_rtp_timestamp: None,
                response_frame_seq: None,
                response_verdict: Some("pending".to_string()),
                lifecycle_phase: None,
                retired_at_ms: None,
            });
        stats.latest_video_rtcp_send_failure_time_ms = Some(now_ms + 130.0);
        stats.latest_video_rtcp_send_failure_reason =
            Some("xbxEngineRtcVideoRtcpFeedbackTargetUnavailable".to_string());
    });

    std::thread::sleep(Duration::from_millis(140));
    let second = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryAnchor".to_string(),
            observed_at_ms: now_ms + 140.0,
        },
        &shared_stats,
    );
    assert_ne!(second.decision.action, RecoveryAction::RequestDecoderReset);
}

#[test]
fn decoded_transport_await_keyframe_without_clean_anchor_upgrades_to_decoder_reset() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.session_phase = Some("recovering".to_string());
    stats.transport_recovery_epoch = 30;
    stats.transport_state = XbxEngineTransportStateDto::Connected;
    stats.host_no_pending_pressure_level = Some("critical".to_string());
    stats.host_no_pending_streak = 220;
    stats.latest_video_host_present_time_ms = Some(now_ms - 2_100.0);
    stats.latest_video_decode_ok_time_ms = Some(now_ms - 1_700.0);
    stats.latest_video_packet_arrival_time_ms = Some(now_ms - 20.0);
    stats.video_decoder_stalled = Some(false);
    stats.video_renderer_stalled = Some(true);
    let shared_stats = Mutex::new(stats);
    let mut coordinator = RecoveryCoordinator::new(
        VideoEscalationController::new(VideoEscalationConfig {
            cooldown_ms: 120,
            keyframe_burst_threshold: 1,
            decoder_reset_burst_threshold: 1,
            keyframe_min_interval_ms: 240,
            escalation_window_ms: 480,
            keyframe_upgrade_min_delay_ms: 0,
        }),
        Instant::now() - Duration::from_secs(3),
        Duration::from_millis(800),
    );

    let first = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryAnchor".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    assert_eq!(first.decision.action, RecoveryAction::RequestKeyframe);

    RuntimeStatsSink::update_shared(&shared_stats, |stats| {
        stats.latest_keyframe_request_episode =
            Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                episode_id: first.decision.observation_id,
                request_reason: Some("transportAwaitRecoveryAnchor".to_string()),
                request_kind: Some("pli".to_string()),
                status: "decoded".to_string(),
                status_detail: None,
                requested_at_ms: now_ms,
                sent_at_ms: Some(now_ms + 10.0),
                deadline_at_ms: Some(now_ms + 960.0),
                transport_detail: None,
                first_video_packet_at_ms: None,
                first_video_packet_rtp_timestamp: None,
                first_video_packet_is_keyframe: None,
                first_keyframe_packet_at_ms: Some(now_ms + 80.0),
                first_keyframe_decoded_at_ms: Some(now_ms + 90.0),
                response_rtp_timestamp: Some(1_234),
                response_frame_seq: Some(55),
                response_verdict: Some("on-time".to_string()),
                lifecycle_phase: None,
                retired_at_ms: None,
            });
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 905,
            source_event: "frame-await-recovery-anchor".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "recovering".to_string(),
                reason: Some("awaitingRecoveryAnchor".to_string()),
                chain_break_evidence: None,

                observed_at_ms: now_ms + 360.0,
            },
            observed_at_ms: now_ms + 360.0,
        });
    });

    std::thread::sleep(Duration::from_millis(360));
    let second = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryAnchor".to_string(),
            observed_at_ms: now_ms + 360.0,
        },
        &shared_stats,
    );
    assert_eq!(second.decision.action, RecoveryAction::RequestDecoderReset);
    assert_eq!(second.budget_after.decoder_reset_budget_used, 0);
}

#[test]
fn missed_transport_await_keyframe_episode_upgrades_to_decoder_reset() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.session_phase = Some("recovering".to_string());
    stats.transport_recovery_epoch = 31;
    stats.transport_state = XbxEngineTransportStateDto::Connected;
    stats.host_no_pending_pressure_level = Some("critical".to_string());
    stats.host_no_pending_streak = 220;
    stats.latest_video_host_present_time_ms = Some(now_ms - 2_100.0);
    stats.latest_video_decode_ok_time_ms = Some(now_ms - 1_700.0);
    stats.latest_video_packet_arrival_time_ms = Some(now_ms - 20.0);
    stats.video_decoder_stalled = Some(false);
    stats.video_renderer_stalled = Some(true);
    let shared_stats = Mutex::new(stats);
    let mut coordinator = RecoveryCoordinator::new(
        VideoEscalationController::new(VideoEscalationConfig {
            cooldown_ms: 120,
            keyframe_burst_threshold: 1,
            decoder_reset_burst_threshold: 1,
            keyframe_min_interval_ms: 240,
            escalation_window_ms: 480,
            keyframe_upgrade_min_delay_ms: 0,
        }),
        Instant::now() - Duration::from_secs(3),
        Duration::from_millis(800),
    );

    let first = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryAnchor".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    assert_eq!(first.decision.action, RecoveryAction::RequestKeyframe);

    RuntimeStatsSink::update_shared(&shared_stats, |stats| {
        stats.latest_keyframe_request_episode =
            Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                episode_id: first.decision.observation_id,
                request_reason: Some("transportAwaitRecoveryAnchor".to_string()),
                request_kind: Some("pli".to_string()),
                status: "missed".to_string(),
                status_detail: None,
                requested_at_ms: now_ms,
                sent_at_ms: Some(now_ms + 10.0),
                deadline_at_ms: Some(now_ms + 120.0),
                transport_detail: None,
                first_video_packet_at_ms: None,
                first_video_packet_rtp_timestamp: None,
                first_video_packet_is_keyframe: None,
                first_keyframe_packet_at_ms: None,
                first_keyframe_decoded_at_ms: None,
                response_rtp_timestamp: None,
                response_frame_seq: None,
                response_verdict: Some("missed".to_string()),
                lifecycle_phase: None,
                retired_at_ms: None,
            });
    });

    std::thread::sleep(Duration::from_millis(140));
    let second = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryAnchor".to_string(),
            observed_at_ms: now_ms + 140.0,
        },
        &shared_stats,
    );
    assert_eq!(second.decision.action, RecoveryAction::RequestDecoderReset);
    assert_eq!(second.budget_after.decoder_reset_budget_used, 0);

    RuntimeStatsSink::update_shared(&shared_stats, |stats| {
        stats.transport_recovery_epoch_at_last_escalation = 31;
        stats.latest_video_escalation_observation =
            Some(crate::XbxEngineVideoEscalationObservation {
                observation_id: second.decision.observation_id,
                reason: "transportAwaitRecoveryAnchor".to_string(),
                action: "requestDecoderReset".to_string(),
                recovery_stage: "rebuilding-supply".to_string(),
                recovery_chain_value: "anchor".to_string(),
                recovery_failure_cost: "high".to_string(),
                recovery_window_source: "transport-await-window".to_string(),
                observed_at_ms: now_ms + 150.0,
            });
    });
    let third = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryAnchor".to_string(),
            observed_at_ms: now_ms + 160.0,
        },
        &shared_stats,
    );
    assert_eq!(third.budget_before.decoder_reset_budget_used, 1);
    assert_eq!(
        third.decision.action,
        RecoveryAction::CoalescedDecoderResetInFlight
    );
}

#[test]
fn rejected_transport_await_anchor_candidate_upgrades_to_decoder_reset() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.session_phase = Some("recovering".to_string());
    stats.transport_recovery_epoch = 32;
    stats.transport_state = XbxEngineTransportStateDto::Connected;
    stats.host_no_pending_pressure_level = Some("critical".to_string());
    stats.host_no_pending_streak = 220;
    stats.latest_video_host_present_time_ms = Some(now_ms - 2_100.0);
    stats.latest_video_decode_ok_time_ms = Some(now_ms - 1_700.0);
    stats.latest_video_packet_arrival_time_ms = Some(now_ms - 20.0);
    stats.video_decoder_stalled = Some(false);
    stats.video_renderer_stalled = Some(true);
    let shared_stats = Mutex::new(stats);
    let mut coordinator = RecoveryCoordinator::new(
        VideoEscalationController::new(VideoEscalationConfig {
            cooldown_ms: 120,
            keyframe_burst_threshold: 1,
            decoder_reset_burst_threshold: 1,
            keyframe_min_interval_ms: 240,
            escalation_window_ms: 480,
            keyframe_upgrade_min_delay_ms: 0,
        }),
        Instant::now() - Duration::from_secs(3),
        Duration::from_millis(800),
    );

    let first = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryAnchor".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    assert_eq!(first.decision.action, RecoveryAction::RequestKeyframe);

    RuntimeStatsSink::update_shared(&shared_stats, |stats| {
        stats.latest_anchor_candidate_ledger = Some(crate::XbxEngineAnchorCandidateLedger {
            recovery_epoch: 32,
            frame_rtp_timestamp: Some(0x1020_3040),
            state: crate::XbxEngineAnchorCandidateState::Rejected,
            source_event: "frame-await-recovery-anchor".to_string(),
            failure_reason: Some(
                crate::XbxEngineAnchorCandidateFailureReason::ChainBrokenReferenceUnrecoverable,
            ),
            observed_at_ms: now_ms + 140.0,
        });
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 906,
            source_event: "frame-await-recovery-anchor".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "broken".to_string(),
                reason: Some("referenceChainUnrecoverable".to_string()),
                chain_break_evidence: None,

                observed_at_ms: now_ms + 140.0,
            },
            observed_at_ms: now_ms + 140.0,
        });
    });

    std::thread::sleep(Duration::from_millis(140));
    let second = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryAnchor".to_string(),
            observed_at_ms: now_ms + 140.0,
        },
        &shared_stats,
    );
    assert_eq!(second.decision.action, RecoveryAction::RequestDecoderReset);
}
