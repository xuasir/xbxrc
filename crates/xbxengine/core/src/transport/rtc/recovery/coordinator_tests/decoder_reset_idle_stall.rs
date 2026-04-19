use super::super::{RecoveryCoordinator, RecoveryOwnerSignal, TransportAwaitRecoveryStage};
use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::recovery::escalation::{RecoveryAction, VideoEscalationReason};
use crate::transport::rtc::recovery::runtime_state::{
    test_support::runtime_state_for_diagnosis, unix_now_ms,
};
use crate::transport::rtc::recovery::startup::SessionPhase;
use crate::XbxEngineMediaRuntimeStats;
use crate::XbxEngineVideoTrackStatus;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use xbxengine_protocol::XbxEngineTransportStateDto;

use super::harness::test_escalation_controller;

#[test]
fn recent_idle_timeout_decoder_reset_suppresses_repeat_idle_timeout() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_recovery_epoch = 3;
    stats.transport_recovery_epoch_at_last_escalation = 3;
    stats.latest_video_escalation_observation = Some(crate::XbxEngineVideoEscalationObservation {
        observation_id: 8,
        reason: "adapterIdleTimeout".to_string(),
        action: "requestDecoderReset".to_string(),
        recovery_stage: "rebuilding-supply".to_string(),
        recovery_chain_value: "health".to_string(),
        recovery_failure_cost: "high".to_string(),
        recovery_window_source: "decoder-reset-window".to_string(),
        observed_at_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as f64,
    });
    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(250, 2, 2),
        Instant::now(),
        Duration::from_millis(800),
    );
    let decision = coordinator.on_reason_with_runtime_stats(
        VideoEscalationReason::AdapterIdleTimeout,
        &Mutex::new(stats),
    );
    assert_eq!(
        decision.action,
        RecoveryAction::CoalescedDecoderResetInFlight
    );
}

#[test]
fn trace_1775319678083_short_adapter_idle_timeout_burst_stays_in_decoder_reset_stage() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_recovery_epoch = 31;
    stats.transport_recovery_epoch_at_last_escalation = 31;
    stats.transport_state = XbxEngineTransportStateDto::Connected;
    stats.latest_video_host_present_time_ms = Some(now_ms - 1_900.0);
    stats.latest_video_decode_ok_time_ms = Some(now_ms - 1_880.0);
    stats.latest_video_packet_arrival_time_ms = Some(now_ms - 1_870.0);
    stats.host_no_pending_pressure_level = Some("critical".to_string());
    stats.host_no_pending_streak = 48;
    stats.latest_video_escalation_observation = Some(crate::XbxEngineVideoEscalationObservation {
        observation_id: 3101,
        reason: "adapterIdleTimeout".to_string(),
        action: "requestDecoderReset".to_string(),
        recovery_stage: "rebuilding-supply".to_string(),
        recovery_chain_value: "health".to_string(),
        recovery_failure_cost: "high".to_string(),
        recovery_window_source: "decoder-reset-window".to_string(),
        observed_at_ms: now_ms,
    });
    let shared_stats = Mutex::new(stats);
    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(250, 2, 2),
        Instant::now(),
        Duration::from_millis(800),
    );

    let first = coordinator
        .on_reason_with_runtime_stats(VideoEscalationReason::AdapterIdleTimeout, &shared_stats);
    assert_eq!(
        first.action,
        RecoveryAction::CoalescedDecoderResetInFlight,
        "trace 1775319678083 的短促 idle burst 首次重复不应直接跳重连"
    );

    let second = coordinator
        .on_reason_with_runtime_stats(VideoEscalationReason::AdapterIdleTimeout, &shared_stats);
    assert_eq!(
        second.action,
        RecoveryAction::CoalescedDecoderResetInFlight,
        "同一短窗内的第二次 idle timeout 仍应停留在 decoder reset 阶段"
    );
}

#[test]
fn hard_paused_stream_retries_decoder_reset_after_timeout() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_state = XbxEngineTransportStateDto::Connected;
    stats.inbound_video_bitrate_kbps = Some(0.0);
    stats.direct_gaming_bitrate_band = Some("paused".to_string());
    stats.video_present_fps = 0.0;
    stats.latest_video_host_present_time_ms = Some(now_ms - 1_600.0);
    stats.latest_video_packet_arrival_time_ms = Some(now_ms - 1_600.0);
    stats.latest_video_decoder_reset_time_ms = Some(now_ms - 1_400.0);
    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(250, 2, 2),
        Instant::now(),
        Duration::from_millis(800),
    );
    let decision = coordinator.on_reason_with_runtime_stats(
        VideoEscalationReason::AdapterIdleTimeout,
        &Mutex::new(stats),
    );
    assert_eq!(decision.action, RecoveryAction::RequestDecoderReset);
}

#[test]
fn hard_paused_stream_prefers_decoder_reset_over_reconnect_after_long_stall() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_state = XbxEngineTransportStateDto::Connected;
    stats.inbound_video_bitrate_kbps = Some(0.0);
    stats.direct_gaming_bitrate_band = Some("paused".to_string());
    stats.video_present_fps = 0.0;
    stats.latest_video_host_present_time_ms = Some(now_ms - 3_600.0);
    stats.latest_video_packet_arrival_time_ms = Some(now_ms - 3_600.0);
    stats.latest_video_decoder_reset_time_ms = Some(now_ms - 2_200.0);
    stats.latest_video_escalation_observation = Some(crate::XbxEngineVideoEscalationObservation {
        observation_id: 11,
        reason: "adapterIdleTimeout".to_string(),
        action: "cooldownSuppressed".to_string(),
        recovery_stage: "steady".to_string(),
        recovery_chain_value: "health".to_string(),
        recovery_failure_cost: "low".to_string(),
        recovery_window_source: "session-phase-window".to_string(),
        observed_at_ms: now_ms - 200.0,
    });
    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(250, 2, 2),
        Instant::now(),
        Duration::from_millis(800),
    );
    let decision = coordinator.on_reason_with_runtime_stats(
        VideoEscalationReason::AdapterIdleTimeout,
        &Mutex::new(stats),
    );
    assert_eq!(decision.action, RecoveryAction::RequestDecoderReset);
}

#[test]
fn hard_paused_stream_with_renderer_stall_prefers_decoder_reset_over_reconnect() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_state = XbxEngineTransportStateDto::Connected;
    stats.inbound_video_bitrate_kbps = Some(0.0);
    stats.direct_gaming_bitrate_band = Some("paused".to_string());
    stats.video_present_fps = 30.0;
    stats.video_renderer_stalled = Some(true);
    stats.latest_video_host_present_time_ms = Some(now_ms - 3_600.0);
    stats.latest_video_packet_arrival_time_ms = Some(now_ms - 3_600.0);
    stats.latest_video_decoder_reset_time_ms = Some(now_ms - 2_200.0);
    stats.latest_video_escalation_observation = Some(crate::XbxEngineVideoEscalationObservation {
        observation_id: 12,
        reason: "adapterIdleTimeout".to_string(),
        action: "cooldownSuppressed".to_string(),
        recovery_stage: "steady".to_string(),
        recovery_chain_value: "health".to_string(),
        recovery_failure_cost: "low".to_string(),
        recovery_window_source: "session-phase-window".to_string(),
        observed_at_ms: now_ms - 200.0,
    });
    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(250, 2, 2),
        Instant::now(),
        Duration::from_millis(800),
    );
    let decision = coordinator.on_reason_with_runtime_stats(
        VideoEscalationReason::AdapterIdleTimeout,
        &Mutex::new(stats),
    );
    assert_eq!(decision.action, RecoveryAction::RequestDecoderReset);
}

#[test]
fn transport_expired_deadline_hard_pause_does_not_bypass_to_decoder_reset() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_state = XbxEngineTransportStateDto::Connected;
    stats.inbound_video_bitrate_kbps = Some(0.0);
    stats.direct_gaming_bitrate_band = Some("paused".to_string());
    stats.video_present_fps = 0.0;
    stats.latest_video_host_present_time_ms = Some(now_ms - 3_600.0);
    stats.latest_video_packet_arrival_time_ms = Some(now_ms - 3_600.0);
    stats.latest_video_decoder_reset_time_ms = Some(now_ms - 2_200.0);
    stats.latest_video_escalation_observation = Some(crate::XbxEngineVideoEscalationObservation {
        observation_id: 13,
        reason: "transportExpiredDeadline".to_string(),
        action: "cooldownSuppressed".to_string(),
        recovery_stage: "steady".to_string(),
        recovery_chain_value: "health".to_string(),
        recovery_failure_cost: "low".to_string(),
        recovery_window_source: "hard-stall-window".to_string(),
        observed_at_ms: now_ms - 200.0,
    });
    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(250, 2, 2),
        Instant::now(),
        Duration::from_millis(800),
    );
    let decision = coordinator.on_reason_with_runtime_stats(
        VideoEscalationReason::TransportExpiredDeadline,
        &Mutex::new(stats),
    );
    // 连接域 deadline 不走 hard_stall 的本地 decoder reset 旁路，交给 escalation 连接域链。
    assert_eq!(decision.action, RecoveryAction::RequestKeyframe);
}

#[test]
fn startup_low_quality_falls_back_to_rebuilding_supply_primary_view() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.session_phase = Some("startup".to_string());
    stats.direct_gaming_bitrate_band = Some("startupLow".to_string());
    let state = runtime_state_for_diagnosis(
        &Mutex::new(stats),
        "healthy",
        Instant::now(),
        Duration::from_millis(800),
    );
    assert_eq!(state.primary_view.owner_state, "rebuilding-supply");
    assert_eq!(state.primary_view.owner_reason, "healthy");
}

#[test]
fn adapter_idle_timeout_falls_back_to_rebuilding_primary_view() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.recovery_diagnosis = Some("adapterIdleTimeout".to_string());
    let state = runtime_state_for_diagnosis(
        &Mutex::new(stats),
        "adapterIdleTimeout",
        Instant::now() - Duration::from_secs(3),
        Duration::from_millis(800),
    );
    assert_eq!(state.phase, SessionPhase::Recovering);
    assert_eq!(state.primary_view.owner_state, "rebuilding-supply");
    assert_eq!(state.primary_view.owner_reason, "adapterIdleTimeout");
}

#[test]
fn fresh_output_falls_back_to_stable_primary_view() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.recovery_diagnosis = Some("adapterIdleTimeout".to_string());
    stats.latest_video_host_present_time_ms = Some(now_ms - 40.0);
    stats.latest_video_decode_ok_time_ms = Some(now_ms - 40.0);
    stats.video_present_fps = 58.0;
    let state = runtime_state_for_diagnosis(
        &Mutex::new(stats),
        "adapterIdleTimeout",
        Instant::now() - Duration::from_secs(3),
        Duration::from_millis(800),
    );
    assert_eq!(state.phase, SessionPhase::Steady);
    assert_eq!(state.primary_view.owner_state, "stable-serving");
    assert_eq!(state.primary_view.owner_reason, "healthy");
}

#[test]
fn adapter_idle_timeout_is_downgraded_when_audio_only_and_recovery_recent() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.recovery_diagnosis = Some("adapterIdleTimeout".to_string());
    stats.latest_video_escalation_observation = Some(crate::XbxEngineVideoEscalationObservation {
        observation_id: 1,
        reason: "adapterIdleTimeout".to_string(),
        action: "requestDecoderReset".to_string(),
        recovery_stage: "priming".to_string(),
        recovery_chain_value: "health".to_string(),
        recovery_failure_cost: "high".to_string(),
        recovery_window_source: "startup-grace".to_string(),
        observed_at_ms: now_ms - 500.0,
    });
    stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
        state: "audioOnly".to_string(),
        video_width: None,
        video_height: None,
        mime_type: None,
        transport_state: XbxEngineTransportStateDto::Connected,
        video_bytes_total: 0,
        video_packet_count_total: 0,
        audio_bytes_total: 42,
        observed_at_ms: now_ms,
    });

    let state = runtime_state_for_diagnosis(
        &Mutex::new(stats),
        "adapterIdleTimeout",
        Instant::now(),
        Duration::from_millis(800),
    );
    assert_eq!(state.phase, SessionPhase::Startup);
    assert_eq!(state.input_profile.baseline, "homeLanGaming");
    assert_eq!(state.primary_view.owner_state, "rebuilding-supply");
    assert_eq!(state.primary_view.owner_reason, "healthy");
    assert_eq!(state.diagnosis_label, "healthy");
}

#[test]
fn owner_signal_is_preserved_through_coordinator_proposal() {
    let shared_stats = Mutex::new(XbxEngineMediaRuntimeStats::default());
    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(250, 2, 2),
        Instant::now(),
        Duration::from_millis(800),
    );
    let signal = RecoveryOwnerSignal {
        reason: VideoEscalationReason::TransportSampleLoss,
        reason_label: "displaySupplyNoPending".to_string(),
        observed_at_ms: unix_now_ms(),
    };
    let proposal = coordinator.propose_from_owner_signal(signal.clone(), &shared_stats);
    assert_eq!(
        signal.reason,
        VideoEscalationReason::TransportSampleLoss
    );
    assert_eq!(signal.reason_label, "displaySupplyNoPending");
}

#[test]
fn wait_keyframe_escalation_budget_is_released_after_new_epoch() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_recovery_epoch = 8;
    let shared_stats = Mutex::new(stats);
    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(120, 1, 1),
        Instant::now() - Duration::from_secs(3),
        Duration::from_millis(800),
    );

    let first = coordinator
        .on_reason_with_runtime_stats(VideoEscalationReason::WaitKeyframe, &shared_stats);
    assert_eq!(first.action, RecoveryAction::RequestKeyframe);

    let second = coordinator
        .on_reason_with_runtime_stats(VideoEscalationReason::WaitKeyframe, &shared_stats);
    assert_eq!(second.action, RecoveryAction::CoalescedKeyframeInFlight);

    RuntimeStatsSink::update_shared(&shared_stats, |stats| {
        stats.transport_recovery_epoch = 9;
        stats.transport_recovery_epoch_at_last_escalation = 8;
        stats.latest_video_escalation_observation =
            Some(crate::XbxEngineVideoEscalationObservation {
                observation_id: 200,
                reason: "waitKeyframe".to_string(),
                action: "requestKeyframe".to_string(),
                recovery_stage: "rebuilding-supply".to_string(),
                recovery_chain_value: "anchor".to_string(),
                recovery_failure_cost: "medium".to_string(),
                recovery_window_source: "transport-await-window".to_string(),
                observed_at_ms: now_ms - 50.0,
            });
    });
    std::thread::sleep(Duration::from_millis(130));

    let third = coordinator
        .on_reason_with_runtime_stats(VideoEscalationReason::WaitKeyframe, &shared_stats);
    assert_ne!(third.action, RecoveryAction::CooldownSuppressed);
}

#[test]
fn coordinator_staged_recovery_reissues_keyframe_before_transport_await_reset() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_recovery_epoch = 10;
    let shared_stats = Mutex::new(stats);
    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(120, 1, 1),
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
            observed_at_ms: now_ms + 80.0,
        },
        &shared_stats,
    );
    assert_eq!(
        second.decision.action,
        RecoveryAction::CoalescedKeyframeInFlight
    );

    std::thread::sleep(Duration::from_millis(420));
    let third = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryAnchor".to_string(),
            observed_at_ms: now_ms + 420.0,
        },
        &shared_stats,
    );
    assert_eq!(third.decision.action, RecoveryAction::RequestKeyframe);
}

#[test]
fn packet_seen_transport_await_episode_upgrades_to_decoder_reset_after_decode_grace_expires() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.session_phase = Some("recovering".to_string());
    stats.transport_recovery_epoch = 10;
    stats.transport_state = XbxEngineTransportStateDto::Connected;
    stats.host_no_pending_pressure_level = Some("critical".to_string());
    stats.host_no_pending_streak = 260;
    stats.latest_video_host_present_time_ms = Some(now_ms - 2_000.0);
    stats.latest_video_decode_ok_time_ms = Some(now_ms - 1_800.0);
    stats.latest_video_packet_arrival_time_ms = Some(now_ms - 10.0);
    stats.video_decoder_stalled = Some(true);
    stats.video_renderer_stalled = Some(true);
    stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
        observation_id: 501,
        source_event: "frame-await-recovery-anchor".to_string(),
        gap: None,
        frame: None,
        chain: crate::XbxEngineVideoTimelineChainSnapshot {
            state: "recovering".to_string(),
            reason: Some("awaitingRecoveryAnchor".to_string()),
            chain_break_evidence: None,

            observed_at_ms: now_ms,
        },
        observed_at_ms: now_ms,
    });
    let shared_stats = Mutex::new(stats);
    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(120, 1, 1),
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
                sent_at_ms: Some(now_ms + 10.0),
                deadline_at_ms: Some(now_ms + 960.0),
                transport_detail: None,
                first_video_packet_at_ms: None,
                first_video_packet_rtp_timestamp: None,
                first_video_packet_is_keyframe: None,
                first_keyframe_packet_at_ms: Some(now_ms + 20.0),
                first_keyframe_decoded_at_ms: None,
                response_rtp_timestamp: Some(7_777),
                response_frame_seq: None,
                response_verdict: Some("on-time".to_string()),
                lifecycle_phase: None,
                retired_at_ms: None,
            });
    });

    std::thread::sleep(Duration::from_millis(260));
    let second = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryAnchor".to_string(),
            observed_at_ms: now_ms + 260.0,
        },
        &shared_stats,
    );
    assert_eq!(second.decision.action, RecoveryAction::RequestDecoderReset);
}

#[test]
fn deferred_transport_await_episode_does_not_keep_keyframe_family_in_flight() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_recovery_epoch = 11;
    stats.transport_state = XbxEngineTransportStateDto::Connected;
    stats.video_anchor_clean_epoch = Some(11);
    stats.video_anchor_clean_observed_at_ms = Some(now_ms - 20.0);
    stats.video_anchor_clean_source_event = Some("chain-clean-anchor-submitted".to_string());
    stats.latest_video_decode_ok_time_ms = Some(now_ms - 10.0);
    stats.latest_video_track_status = Some(XbxEngineVideoTrackStatus {
        state: "remoteTrackAttached".to_string(),
        video_width: Some(1920),
        video_height: Some(1080),
        mime_type: Some("video/H264".to_string()),
        transport_state: XbxEngineTransportStateDto::Connected,
        video_bytes_total: 128_000,
        video_packet_count_total: 620,
        audio_bytes_total: 3_600,
        observed_at_ms: now_ms,
    });
    stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
        observation_id: 601,
        source_event: "frame-observed".to_string(),
        gap: None,
        frame: None,
        chain: crate::XbxEngineVideoTimelineChainSnapshot {
            state: "recovering".to_string(),
            reason: Some("awaitingRecoveryAnchor".to_string()),
            chain_break_evidence: None,

            observed_at_ms: now_ms,
        },
        observed_at_ms: now_ms,
    });
    let shared_stats = Mutex::new(stats);
    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(120, 1, 1),
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
                status: "deferred".to_string(),
                status_detail: None,
                requested_at_ms: now_ms,
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
                observation_id: 6_012,
                frame_rtp_timestamp: Some(91_001),
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
                observed_at_ms: now_ms + 1.0,

                ..Default::default()
            });
    });

    std::thread::sleep(Duration::from_millis(180));
    assert_eq!(
        RecoveryCoordinator::transport_await_recovery_stage_from_runtime(
            &shared_stats,
            now_ms + 180.0
        ),
        Some(TransportAwaitRecoveryStage::ProbeKeyframe)
    );
    let second = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryAnchor".to_string(),
            observed_at_ms: now_ms + 180.0,
        },
        &shared_stats,
    );
    assert_eq!(second.decision.action, RecoveryAction::RequestKeyframe);
    assert_ne!(second.decision.action, RecoveryAction::WaitForBurst);
    assert_ne!(
        second.decision.action,
        RecoveryAction::CoalescedKeyframeInFlight
    );
}

#[test]
fn stale_transport_await_decoder_reset_without_progress_can_reopen_decoder_reset() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.session_phase = Some("recovering".to_string());
    stats.transport_recovery_epoch = 12;
    stats.transport_recovery_epoch_at_last_escalation = 12;
    stats.transport_state = XbxEngineTransportStateDto::Connected;
    stats.host_no_pending_pressure_level = Some("critical".to_string());
    stats.host_no_pending_streak = 420;
    stats.latest_video_host_present_time_ms = Some(now_ms - 3_000.0);
    stats.latest_video_decode_ok_time_ms = Some(now_ms - 3_000.0);
    stats.latest_video_packet_arrival_time_ms = Some(now_ms - 10.0);
    stats.video_decoder_stalled = Some(true);
    stats.video_renderer_stalled = Some(true);
    stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
        observation_id: 701,
        source_event: "frame-await-recovery-anchor".to_string(),
        gap: None,
        frame: None,
        chain: crate::XbxEngineVideoTimelineChainSnapshot {
            state: "recovering".to_string(),
            reason: Some("awaitingRecoveryAnchor".to_string()),
            chain_break_evidence: None,

            observed_at_ms: now_ms,
        },
        observed_at_ms: now_ms,
    });
    stats.latest_keyframe_request_episode =
        Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
            episode_id: 9001,
            request_reason: Some("transportAwaitRecoveryAnchor".to_string()),
            request_kind: Some("pli".to_string()),
            status: "missed".to_string(),
            status_detail: None,
            requested_at_ms: now_ms - 1_400.0,
            sent_at_ms: Some(now_ms - 1_390.0),
            deadline_at_ms: Some(now_ms - 1_000.0),
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
    stats.latest_video_escalation_observation = Some(crate::XbxEngineVideoEscalationObservation {
        observation_id: 8001,
        reason: "transportAwaitRecoveryAnchor".to_string(),
        action: "requestDecoderReset".to_string(),
        recovery_stage: "rebuilding-supply".to_string(),
        recovery_chain_value: "anchor".to_string(),
        recovery_failure_cost: "high".to_string(),
        recovery_window_source: "transport-await-window".to_string(),
        observed_at_ms: now_ms - 1_200.0,
    });
    stats.latest_video_decoder_reset_time_ms = Some(now_ms - 1_100.0);
    let shared_stats = Mutex::new(stats);
    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(120, 1, 2),
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
    assert_eq!(proposal.budget_before.decoder_reset_budget_used, 1);
    assert_eq!(
        proposal.decision.action,
        RecoveryAction::RequestDecoderReset
    );
}

#[test]
fn invalid_transport_await_keyframe_response_releases_decoder_reset_inflight() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.session_phase = Some("recovering".to_string());
    stats.transport_recovery_epoch = 13;
    stats.transport_recovery_epoch_at_last_escalation = 13;
    stats.transport_state = XbxEngineTransportStateDto::Connected;
    stats.host_no_pending_pressure_level = Some("critical".to_string());
    stats.host_no_pending_streak = 360;
    stats.latest_video_host_present_time_ms = Some(now_ms - 3_000.0);
    stats.latest_video_decode_ok_time_ms = Some(now_ms - 3_000.0);
    stats.latest_video_packet_arrival_time_ms = Some(now_ms - 8.0);
    stats.video_decoder_stalled = Some(true);
    stats.video_renderer_stalled = Some(true);
    stats.inbound_primary_video_bytes_total = 6_000;
    stats.latest_video_track_status = Some(XbxEngineVideoTrackStatus {
        state: "remoteTrackAttached".to_string(),
        video_width: Some(1920),
        video_height: Some(1080),
        mime_type: Some("video/H264".to_string()),
        transport_state: XbxEngineTransportStateDto::Connected,
        video_bytes_total: 6_000,
        video_packet_count_total: 40,
        audio_bytes_total: 0,
        observed_at_ms: now_ms - 6.0,
    });
    stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
        observation_id: 901,
        source_event: "frame-await-recovery-anchor".to_string(),
        gap: None,
        frame: None,
        chain: crate::XbxEngineVideoTimelineChainSnapshot {
            state: "recovering".to_string(),
            reason: Some("awaitingRecoveryAnchor".to_string()),
            chain_break_evidence: None,

            observed_at_ms: now_ms - 6.0,
        },
        observed_at_ms: now_ms - 6.0,
    });
    stats.latest_keyframe_request_episode =
        Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
            episode_id: 9200,
            request_reason: Some("transportAwaitRecoveryAnchor".to_string()),
            request_kind: Some("pli".to_string()),
            status: "packet-seen".to_string(),
            status_detail: None,
            requested_at_ms: now_ms - 260.0,
            sent_at_ms: Some(now_ms - 250.0),
            deadline_at_ms: Some(now_ms + 500.0),
            transport_detail: None,
            first_video_packet_at_ms: None,
            first_video_packet_rtp_timestamp: None,
            first_video_packet_is_keyframe: None,
            first_keyframe_packet_at_ms: Some(now_ms - 230.0),
            first_keyframe_decoded_at_ms: None,
            response_rtp_timestamp: Some(8_888),
            response_frame_seq: None,
            response_verdict: Some("on-time".to_string()),
            lifecycle_phase: None,
            retired_at_ms: None,
        });
    stats.latest_h264_inspection_observation = Some(crate::XbxEngineH264InspectionObservation {
        observation_id: 9201,
        frame_rtp_timestamp: Some(8_888),
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
        observed_at_ms: now_ms - 225.0,

        ..Default::default()
    });
    stats.latest_video_escalation_observation = Some(crate::XbxEngineVideoEscalationObservation {
        observation_id: 9202,
        reason: "transportAwaitRecoveryAnchor".to_string(),
        action: "requestDecoderReset".to_string(),
        recovery_stage: "rebuilding-supply".to_string(),
        recovery_chain_value: "anchor".to_string(),
        recovery_failure_cost: "high".to_string(),
        recovery_window_source: "transport-await-window".to_string(),
        observed_at_ms: now_ms - 240.0,
    });
    stats.latest_video_decoder_reset_time_ms = Some(now_ms - 235.0);
    let shared_stats = Mutex::new(stats);
    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(120, 1, 2),
        Instant::now() - Duration::from_secs(3),
        Duration::from_millis(800),
    );

    assert!(
        RecoveryCoordinator::transport_await_recovery_stage_from_runtime(&shared_stats, now_ms)
            != Some(TransportAwaitRecoveryStage::AwaitDecoderResetProgress)
    );

    let proposal = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryAnchor".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    assert_eq!(
        proposal.decision.action,
        RecoveryAction::RequestDecoderReset
    );
}

#[test]
fn transport_await_lane_distinguishes_probe_decode_and_reset_progress() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_recovery_epoch = 13;
    stats.transport_state = XbxEngineTransportStateDto::Connected;
    stats.host_no_pending_pressure_level = Some("critical".to_string());
    stats.host_no_pending_streak = 200;
    stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
        observation_id: 801,
        source_event: "frame-await-recovery-anchor".to_string(),
        gap: None,
        frame: None,
        chain: crate::XbxEngineVideoTimelineChainSnapshot {
            state: "recovering".to_string(),
            reason: Some("awaitingRecoveryAnchor".to_string()),
            chain_break_evidence: None,

            observed_at_ms: now_ms,
        },
        observed_at_ms: now_ms,
    });
    stats.latest_keyframe_request_episode =
        Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
            episode_id: 9100,
            request_reason: Some("transportAwaitRecoveryAnchor".to_string()),
            request_kind: Some("pli".to_string()),
            status: "sent".to_string(),
            status_detail: None,
            requested_at_ms: now_ms - 40.0,
            sent_at_ms: Some(now_ms - 35.0),
            deadline_at_ms: Some(now_ms + 200.0),
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
    let shared_stats = Mutex::new(stats);

    assert_eq!(
        RecoveryCoordinator::transport_await_recovery_stage_from_runtime(&shared_stats, now_ms),
        Some(TransportAwaitRecoveryStage::ProbeKeyframe)
    );

    RuntimeStatsSink::update_shared(&shared_stats, |stats| {
        if let Some(episode) = stats.latest_keyframe_request_episode.as_mut() {
            episode.status = "packet-seen".to_string();
            episode.first_keyframe_packet_at_ms = Some(now_ms - 10.0);
            episode.response_verdict = Some("on-time".to_string());
        }
    });
    assert_eq!(
        RecoveryCoordinator::transport_await_recovery_stage_from_runtime(&shared_stats, now_ms),
        Some(TransportAwaitRecoveryStage::AwaitDecodeProgress)
    );

    RuntimeStatsSink::update_shared(&shared_stats, |stats| {
        stats.latest_video_escalation_observation =
            Some(crate::XbxEngineVideoEscalationObservation {
                observation_id: 8200,
                reason: "transportAwaitRecoveryAnchor".to_string(),
                action: "requestDecoderReset".to_string(),
                recovery_stage: "rebuilding-supply".to_string(),
                recovery_chain_value: "anchor".to_string(),
                recovery_failure_cost: "high".to_string(),
                recovery_window_source: "transport-await-window".to_string(),
                observed_at_ms: now_ms - 20.0,
            });
        stats.latest_video_decoder_reset_time_ms = Some(now_ms - 15.0);
    });
    assert_eq!(
        RecoveryCoordinator::transport_await_recovery_stage_from_runtime(&shared_stats, now_ms),
        Some(TransportAwaitRecoveryStage::AwaitDecoderResetProgress)
    );
}

#[test]
fn transport_await_invalid_nonidr_response_releases_reset_and_decode_wait_lanes() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_recovery_epoch = 14;
    stats.transport_state = XbxEngineTransportStateDto::Connected;
    stats.video_decoder_stalled = Some(true);
    stats.video_renderer_stalled = Some(true);
    stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
        observation_id: 851,
        source_event: "frame-await-recovery-anchor".to_string(),
        gap: None,
        frame: None,
        chain: crate::XbxEngineVideoTimelineChainSnapshot {
            state: "recovering".to_string(),
            reason: Some("awaitingRecoveryAnchor".to_string()),
            chain_break_evidence: None,

            observed_at_ms: now_ms,
        },
        observed_at_ms: now_ms,
    });
    stats.latest_video_track_status = Some(XbxEngineVideoTrackStatus {
        state: "remoteTrackAttached".to_string(),
        video_width: Some(1920),
        video_height: Some(1080),
        mime_type: Some("video/H264".to_string()),
        transport_state: XbxEngineTransportStateDto::Connected,
        video_bytes_total: 120_000,
        video_packet_count_total: 700,
        audio_bytes_total: 4_200,
        observed_at_ms: now_ms,
    });
    stats.latest_keyframe_request_episode =
        Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
            episode_id: 9_200,
            request_reason: Some("transportAwaitRecoveryAnchor".to_string()),
            request_kind: Some("pli".to_string()),
            status: "packet-seen".to_string(),
            status_detail: None,
            requested_at_ms: now_ms - 90.0,
            sent_at_ms: Some(now_ms - 80.0),
            deadline_at_ms: Some(now_ms + 200.0),
            transport_detail: None,
            first_video_packet_at_ms: None,
            first_video_packet_rtp_timestamp: None,
            first_video_packet_is_keyframe: None,
            first_keyframe_packet_at_ms: Some(now_ms - 15.0),
            first_keyframe_decoded_at_ms: None,
            response_rtp_timestamp: Some(55_123),
            response_frame_seq: None,
            response_verdict: Some("on-time".to_string()),
            lifecycle_phase: None,
            retired_at_ms: None,
        });
    stats.latest_h264_inspection_observation = Some(crate::XbxEngineH264InspectionObservation {
        observation_id: 8_520,
        frame_rtp_timestamp: Some(55_123),
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
        observed_at_ms: now_ms - 10.0,

        ..Default::default()
    });
    stats.latest_video_escalation_observation = Some(crate::XbxEngineVideoEscalationObservation {
        observation_id: 8_521,
        reason: "transportAwaitRecoveryAnchor".to_string(),
        action: "requestDecoderReset".to_string(),
        recovery_stage: "rebuilding-supply".to_string(),
        recovery_chain_value: "anchor".to_string(),
        recovery_failure_cost: "high".to_string(),
        recovery_window_source: "transport-await-window".to_string(),
        observed_at_ms: now_ms - 30.0,
    });
    stats.latest_video_decoder_reset_time_ms = Some(now_ms - 25.0);
    let shared_stats = Mutex::new(stats);

    assert_eq!(
        RecoveryCoordinator::transport_await_recovery_stage_from_runtime(&shared_stats, now_ms),
        Some(TransportAwaitRecoveryStage::ProbeKeyframe)
    );
}

#[test]
fn transport_await_invalid_nonidr_inspection_after_reset_does_not_coalesce_stale_decoder_reset() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_recovery_epoch = 15;
    stats.transport_state = XbxEngineTransportStateDto::Connected;
    stats.video_decoder_stalled = Some(true);
    stats.video_renderer_stalled = Some(true);
    stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
        observation_id: 861,
        source_event: "gap-repair-in-flight".to_string(),
        gap: None,
        frame: None,
        chain: crate::XbxEngineVideoTimelineChainSnapshot {
            state: "recovering".to_string(),
            reason: Some("awaitingRecoveryAnchor".to_string()),
            chain_break_evidence: None,

            observed_at_ms: now_ms,
        },
        observed_at_ms: now_ms,
    });
    stats.latest_video_track_status = Some(XbxEngineVideoTrackStatus {
        state: "remoteTrackAttached".to_string(),
        video_width: Some(1920),
        video_height: Some(1080),
        mime_type: Some("video/H264".to_string()),
        transport_state: XbxEngineTransportStateDto::Connected,
        video_bytes_total: 120_000,
        video_packet_count_total: 700,
        audio_bytes_total: 4_200,
        observed_at_ms: now_ms,
    });
    stats.latest_keyframe_request_episode =
        Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
            episode_id: 9_300,
            request_reason: Some("transportAwaitRecoveryAnchor".to_string()),
            request_kind: Some("pli".to_string()),
            status: "decoded".to_string(),
            status_detail: None,
            requested_at_ms: now_ms - 36_000.0,
            sent_at_ms: Some(now_ms - 35_900.0),
            deadline_at_ms: Some(now_ms + 200.0),
            transport_detail: None,
            first_video_packet_at_ms: Some(now_ms - 35_850.0),
            first_video_packet_rtp_timestamp: Some(66_123),
            first_video_packet_is_keyframe: Some(false),
            first_keyframe_packet_at_ms: Some(now_ms - 35_850.0),
            first_keyframe_decoded_at_ms: Some(now_ms - 35_840.0),
            response_rtp_timestamp: Some(66_123),
            response_frame_seq: None,
            response_verdict: Some("missed".to_string()),
            lifecycle_phase: None,
            retired_at_ms: None,
        });
    stats.latest_h264_inspection_observation = Some(crate::XbxEngineH264InspectionObservation {
        observation_id: 8_620,
        frame_rtp_timestamp: Some(77_123),
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
        observed_at_ms: now_ms - 10.0,

        ..Default::default()
    });
    stats.latest_video_escalation_observation = Some(crate::XbxEngineVideoEscalationObservation {
        observation_id: 8_621,
        reason: "transportAwaitRecoveryAnchor".to_string(),
        action: "requestDecoderReset".to_string(),
        recovery_stage: "rebuilding-supply".to_string(),
        recovery_chain_value: "anchor".to_string(),
        recovery_failure_cost: "high".to_string(),
        recovery_window_source: "transport-await-window".to_string(),
        observed_at_ms: now_ms - 30.0,
    });
    stats.latest_video_decoder_reset_time_ms = Some(now_ms - 25.0);
    let shared_stats = Mutex::new(stats);
    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(120, 1, 1),
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
    assert_ne!(
        proposal.decision.action,
        RecoveryAction::CoalescedDecoderResetInFlight
    );
}

#[test]
fn transport_await_with_connected_stall_evidence_escalates_on_second_post_cooldown_tick() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_recovery_epoch = 13;
    stats.transport_state = XbxEngineTransportStateDto::Connected;
    stats.video_renderer_stalled = Some(true);
    stats.latest_video_host_present_time_ms = Some(now_ms - 2_000.0);
    let shared_stats = Mutex::new(stats);
    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(120, 1, 1),
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

    std::thread::sleep(Duration::from_millis(220));
    let second = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryAnchor".to_string(),
            observed_at_ms: now_ms + 220.0,
        },
        &shared_stats,
    );
    assert_eq!(second.decision.action, RecoveryAction::RequestKeyframe);
}

#[test]
fn coordinator_staged_recovery_handles_sparse_transport_await_signals() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_recovery_epoch = 14;
    let shared_stats = Mutex::new(stats);
    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(120, 1, 1),
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
            observed_at_ms: now_ms + 1_600.0,
        },
        &shared_stats,
    );
    assert_ne!(second.decision.action, RecoveryAction::RequestDecoderReset);
}

#[test]
fn recent_clean_anchor_keeps_transport_await_recovery_keyframe_from_forcing_hard_escalation() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_recovery_epoch = 12;
    stats.video_anchor_clean_epoch = Some(12);
    stats.video_anchor_clean_observed_at_ms = Some(now_ms - 180.0);
    stats.video_anchor_clean_source_event = Some("chain-clean-anchor-submitted".to_string());
    stats.latest_video_host_present_time_ms = Some(now_ms);
    stats.latest_video_track_status = Some(XbxEngineVideoTrackStatus {
        state: "remoteTrackAttached".to_string(),
        video_width: Some(1280),
        video_height: Some(720),
        mime_type: Some("video/H264".to_string()),
        transport_state: XbxEngineTransportStateDto::Connected,
        video_bytes_total: 64_000,
        video_packet_count_total: 500,
        audio_bytes_total: 2_000,
        observed_at_ms: now_ms,
    });
    stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
        observation_id: 42,
        source_event: "frame-observed".to_string(),
        gap: None,
        frame: None,
        chain: crate::XbxEngineVideoTimelineChainSnapshot {
            state: "healthy".to_string(),
            reason: None,
            chain_break_evidence: None,

            observed_at_ms: now_ms - 30.0,
        },
        observed_at_ms: now_ms - 30.0,
    });
    let shared_stats = Mutex::new(stats);
    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(120, 1, 1),
        Instant::now() - Duration::from_secs(3),
        Duration::from_millis(800),
    );

    let _ = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryAnchor".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    let _ = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryAnchor".to_string(),
            observed_at_ms: now_ms + 60.0,
        },
        &shared_stats,
    );
    let third = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryAnchor".to_string(),
            observed_at_ms: now_ms + 120.0,
        },
        &shared_stats,
    );
    assert_eq!(
        third.decision.action,
        RecoveryAction::CoalescedKeyframeInFlight
    );
}

#[test]
fn clean_anchor_absorbs_stale_transport_await_ingress_waiting_stage() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_recovery_epoch = 12;
    stats.video_anchor_clean_epoch = Some(12);
    stats.video_anchor_clean_observed_at_ms = Some(now_ms - 20.0);
    stats.video_anchor_clean_source_event = Some("chain-clean-anchor-submitted".to_string());
    stats.latest_video_host_present_time_ms = Some(now_ms - 8.0);
    stats.latest_video_decode_ok_time_ms = Some(now_ms - 4.0);
    stats.latest_video_track_status = Some(XbxEngineVideoTrackStatus {
        state: "remoteTrackAttached".to_string(),
        video_width: Some(1280),
        video_height: Some(720),
        mime_type: Some("video/H264".to_string()),
        transport_state: XbxEngineTransportStateDto::Connected,
        video_bytes_total: 64_000,
        video_packet_count_total: 500,
        audio_bytes_total: 2_000,
        observed_at_ms: now_ms - 4.0,
    });
    stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
        observation_id: 42,
        source_event: "frame-await-recovery-anchor".to_string(),
        gap: None,
        frame: None,
        chain: crate::XbxEngineVideoTimelineChainSnapshot {
            state: "recovering".to_string(),
            reason: Some("transportAwaitRecoveryAnchor".to_string()),
            chain_break_evidence: None,

            observed_at_ms: now_ms - 120.0,
        },
        observed_at_ms: now_ms - 120.0,
    });

    assert_eq!(
        RecoveryCoordinator::transport_await_recovery_stage_from_runtime(
            &Mutex::new(stats),
            now_ms
        ),
        None
    );
}

#[test]
fn recent_clean_anchor_candidate_ledger_keeps_transport_await_recovery_keyframe_from_forcing_hard_escalation(
) {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_recovery_epoch = 12;
    stats.latest_video_host_present_time_ms = Some(now_ms);
    stats.latest_video_track_status = Some(XbxEngineVideoTrackStatus {
        state: "remoteTrackAttached".to_string(),
        video_width: Some(1280),
        video_height: Some(720),
        mime_type: Some("video/H264".to_string()),
        transport_state: XbxEngineTransportStateDto::Connected,
        video_bytes_total: 64_000,
        video_packet_count_total: 500,
        audio_bytes_total: 2_000,
        observed_at_ms: now_ms,
    });
    stats.latest_anchor_candidate_ledger = Some(crate::XbxEngineAnchorCandidateLedger {
        recovery_epoch: 12,
        frame_rtp_timestamp: Some(98_765),
        state: crate::XbxEngineAnchorCandidateState::SubmittedCleanAnchor,
        source_event: "chain-clean-anchor-submitted".to_string(),
        failure_reason: None,
        observed_at_ms: now_ms - 180.0,
    });
    stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
        observation_id: 43,
        source_event: "frame-observed".to_string(),
        gap: None,
        frame: None,
        chain: crate::XbxEngineVideoTimelineChainSnapshot {
            state: "healthy".to_string(),
            reason: None,
            chain_break_evidence: None,

            observed_at_ms: now_ms - 30.0,
        },
        observed_at_ms: now_ms - 30.0,
    });
    let shared_stats = Mutex::new(stats);
    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(120, 1, 1),
        Instant::now() - Duration::from_secs(3),
        Duration::from_millis(800),
    );

    let _ = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryAnchor".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    let _ = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryAnchor".to_string(),
            observed_at_ms: now_ms + 60.0,
        },
        &shared_stats,
    );
    let third = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryAnchor".to_string(),
            observed_at_ms: now_ms + 120.0,
        },
        &shared_stats,
    );
    assert_eq!(
        third.decision.action,
        RecoveryAction::CoalescedKeyframeInFlight
    );
}

#[test]
fn clean_anchor_acknowledgement_resets_transport_await_streak() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_recovery_epoch = 11;
    let shared_stats = Mutex::new(stats);
    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(120, 1, 1),
        Instant::now() - Duration::from_secs(3),
        Duration::from_millis(800),
    );

    let _ = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryAnchor".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    let _ = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryAnchor".to_string(),
            observed_at_ms: now_ms + 120.0,
        },
        &shared_stats,
    );

    coordinator.acknowledge_clean_anchor();

    let after_clean_anchor = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryAnchor".to_string(),
            observed_at_ms: now_ms + 240.0,
        },
        &shared_stats,
    );
    assert_ne!(
        after_clean_anchor.decision.action,
        RecoveryAction::RequestDecoderReset
    );
    assert_ne!(
        after_clean_anchor.decision.action,
        RecoveryAction::RequestReconnectCandidate
    );
}

#[test]
fn transport_await_hard_fallback_timeout_resets_across_recovery_epoch() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_recovery_epoch = 20;
    stats.video_renderer_stalled = Some(true);
    stats.latest_video_host_present_time_ms = Some(now_ms - 6_000.0);
    let shared_stats = Mutex::new(stats);
    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(120, 1, 1),
        Instant::now() - Duration::from_secs(3),
        Duration::from_millis(800),
    );

    let _ = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryAnchor".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    RuntimeStatsSink::update_shared(&shared_stats, |stats| {
        stats.transport_recovery_epoch = 21;
    });
    let _ = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryAnchor".to_string(),
            observed_at_ms: now_ms + 1_600.0,
        },
        &shared_stats,
    );
    RuntimeStatsSink::update_shared(&shared_stats, |stats| {
        stats.latest_video_escalation_observation =
            Some(crate::XbxEngineVideoEscalationObservation {
                observation_id: 901,
                reason: "transportAwaitRecoveryAnchor".to_string(),
                action: "requestDecoderReset".to_string(),
                recovery_stage: "rebuilding-supply".to_string(),
                recovery_chain_value: "anchor".to_string(),
                recovery_failure_cost: "high".to_string(),
                recovery_window_source: "transport-await-window".to_string(),
                observed_at_ms: now_ms + 1_650.0,
            });
        stats.latest_video_decoder_reset_time_ms = Some(now_ms + 1_700.0);
    });
    let timeout = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryAnchor".to_string(),
            observed_at_ms: now_ms + 2_700.0,
        },
        &shared_stats,
    );
    assert_ne!(
        timeout.decision.action,
        RecoveryAction::RequestReconnectCandidate
    );
    let fallback = RuntimeStatsSink::read_shared(&shared_stats, |stats| {
        (
            stats.recovery_hard_fallback_timer_ms,
            stats.recovery_hard_fallback_trigger_reason.clone(),
            stats.recovery_hard_fallback_timer_reset_reason.clone(),
        )
    })
    .unwrap_or((None, None, None));
    assert!(fallback.0.is_some_and(|timer_ms| timer_ms <= 1_200.0));
    assert!(fallback.1.is_none());
    assert!(fallback.2.is_none());
}

#[test]
fn transport_await_reconnecting_stage_stays_in_decoder_reset_after_hard_fallback_timeout() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_recovery_epoch = 31;
    stats.transport_state = XbxEngineTransportStateDto::Connecting;
    stats.transport_recovery_episode_active = true;
    stats.recovery_diagnosis = Some("transportAwaitRecoveryAnchor".to_string());
    stats.video_renderer_stalled = Some(true);
    stats.latest_video_host_present_time_ms = Some(now_ms - 10_000.0);
    stats.latest_video_packet_arrival_time_ms = Some(now_ms - 10_000.0);
    let shared_stats = Mutex::new(stats);
    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(120, 1, 1),
        Instant::now() - Duration::from_secs(3),
        Duration::from_millis(800),
    );

    let _ = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryAnchor".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    let promoted = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryAnchor".to_string(),
            observed_at_ms: now_ms + 10_000.0,
        },
        &shared_stats,
    );
    assert_eq!(
        promoted.decision.action,
        RecoveryAction::RequestDecoderReset
    );
}

#[test]
fn bootstrap_in_flight_signal_stays_in_local_probe_domain() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_recovery_epoch = 12;
    // `transport_await_bootstrap_in_flight_active_from_stats` 要求会话已连接，且存在 transport-await 的
    // keyframe episode 上下文；否则 sustaining 阶段不会成立，测试会误落到 ProbeKeyframe。
    stats.transport_state = XbxEngineTransportStateDto::Connected;
    stats.video_renderer_stalled = Some(false);
    stats.latest_keyframe_request_episode =
        Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
            episode_id: 1,
            request_reason: Some("transportAwaitRecoveryAnchor".to_string()),
            request_kind: None,
            status: "requested".to_string(),
            status_detail: None,
            requested_at_ms: now_ms - 200.0,
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
            response_verdict: None,
            lifecycle_phase: None,
            retired_at_ms: None,
        });
    stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
        observation_id: 777,
        source_event: "frame-observed".to_string(),
        gap: None,
        frame: None,
        chain: crate::XbxEngineVideoTimelineChainSnapshot {
            state: "recovering".to_string(),
            reason: Some("awaitingRecoveryAnchor".to_string()),
            chain_break_evidence: None,

            observed_at_ms: now_ms - 60.0,
        },
        observed_at_ms: now_ms - 60.0,
    });
    stats.latest_video_decode_ok_time_ms = Some(now_ms - 90.0);
    stats.latest_video_host_present_time_ms = Some(now_ms - 70.0);
    stats.latest_video_track_status = Some(XbxEngineVideoTrackStatus {
        state: "remoteTrackAttached".to_string(),
        video_width: Some(1280),
        video_height: Some(720),
        mime_type: Some("video/H264".to_string()),
        transport_state: XbxEngineTransportStateDto::Connected,
        video_bytes_total: 64_000,
        video_packet_count_total: 500,
        audio_bytes_total: 2_000,
        observed_at_ms: now_ms,
    });
    stats.latest_anchor_candidate_ledger = Some(crate::XbxEngineAnchorCandidateLedger {
        recovery_epoch: 12,
        frame_rtp_timestamp: Some(98_765),
        state: crate::XbxEngineAnchorCandidateState::SubmittedCleanAnchor,
        source_event: "chain-clean-anchor-submitted".to_string(),
        failure_reason: None,
        observed_at_ms: now_ms - 180.0,
    });
    let shared_stats = Mutex::new(stats);

    assert_eq!(
        RecoveryCoordinator::transport_await_recovery_stage(&shared_stats, 12, now_ms, true),
        Some(TransportAwaitRecoveryStage::BootstrapInFlight)
    );

    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(120, 1, 1),
        Instant::now() - Duration::from_secs(3),
        Duration::from_millis(800),
    );
    let proposal = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "recoverySustaining".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );

    assert_eq!(proposal.decision.action, RecoveryAction::WaitForBurst);
}

#[test]
fn transport_await_nonidr_breaks_sustaining_wait_burst_suppression() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_recovery_epoch = 44;
    stats.transport_state = XbxEngineTransportStateDto::Connected;
    stats.video_renderer_stalled = Some(false);
    stats.latest_keyframe_request_episode =
        Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
            episode_id: 1,
            request_reason: Some("transportAwaitRecoveryAnchor".to_string()),
            request_kind: None,
            status: "requested".to_string(),
            status_detail: None,
            requested_at_ms: now_ms - 200.0,
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
            response_verdict: None,
            lifecycle_phase: None,
            retired_at_ms: None,
        });
    stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
        observation_id: 888,
        source_event: "frame-await-recovery-anchor".to_string(),
        gap: None,
        frame: None,
        chain: crate::XbxEngineVideoTimelineChainSnapshot {
            state: "recovering".to_string(),
            reason: Some("awaitingRecoveryAnchor".to_string()),
            chain_break_evidence: None,

            observed_at_ms: now_ms - 40.0,
        },
        observed_at_ms: now_ms - 40.0,
    });
    stats.latest_video_decode_ok_time_ms = Some(now_ms - 90.0);
    stats.latest_video_host_present_time_ms = Some(now_ms - 70.0);
    stats.latest_video_track_status = Some(XbxEngineVideoTrackStatus {
        state: "remoteTrackAttached".to_string(),
        video_width: Some(1280),
        video_height: Some(720),
        mime_type: Some("video/H264".to_string()),
        transport_state: XbxEngineTransportStateDto::Connected,
        video_bytes_total: 64_000,
        video_packet_count_total: 500,
        audio_bytes_total: 2_000,
        observed_at_ms: now_ms,
    });
    stats.latest_anchor_candidate_ledger = Some(crate::XbxEngineAnchorCandidateLedger {
        recovery_epoch: 44,
        frame_rtp_timestamp: Some(12_345),
        state: crate::XbxEngineAnchorCandidateState::SubmittedCleanAnchor,
        source_event: "chain-clean-anchor-submitted".to_string(),
        failure_reason: None,
        observed_at_ms: now_ms - 180.0,
    });
    stats.latest_h264_inspection_observation = Some(crate::XbxEngineH264InspectionObservation {
        observation_id: 99,
        frame_rtp_timestamp: Some(12_400),
        nal_types: vec!["nonidr".to_string()],
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
        sample_width: Some(1280),
        sample_height: Some(720),
        bootstrap_ready: false,
        bootstrap_reject_reason: Some("NonIdrVcl".to_string()),
        admission_accepted: true,
        observed_at_ms: now_ms - 30.0,

        ..Default::default()
    });
    let shared_stats = Mutex::new(stats);
    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(120, 1, 1),
        Instant::now() - Duration::from_secs(3),
        Duration::from_millis(800),
    );
    let proposal = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "recoverySustaining".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    assert_ne!(
        proposal.decision.action,
        RecoveryAction::WaitForBurst,
        "fresh NonIdrVcl invalid bootstrap should break sustaining suppression"
    );
}

#[test]
fn transport_await_hard_fallback_timer_resets_on_healthy_clean_anchor() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_recovery_epoch = 30;
    stats.video_renderer_stalled = Some(true);
    stats.latest_video_host_present_time_ms = Some(now_ms - 6_000.0);
    let shared_stats = Mutex::new(stats);
    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(120, 1, 1),
        Instant::now() - Duration::from_secs(3),
        Duration::from_millis(800),
    );
    let _ = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryAnchor".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    RuntimeStatsSink::update_shared(&shared_stats, |stats| {
        stats.video_anchor_clean_epoch = Some(30);
        stats.video_anchor_clean_observed_at_ms = Some(now_ms + 100.0);
        stats.video_anchor_clean_source_event = Some("chain-clean-anchor-submitted".to_string());
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 77,
            source_event: "frame-observed".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "healthy".to_string(),
                reason: None,
                chain_break_evidence: None,

                observed_at_ms: now_ms + 100.0,
            },
            observed_at_ms: now_ms + 100.0,
        });
        stats.video_renderer_stalled = Some(false);
        stats.latest_video_host_present_time_ms = Some(now_ms + 100.0);
        stats.latest_video_track_status = Some(XbxEngineVideoTrackStatus {
            state: "remoteTrackAttached".to_string(),
            video_width: Some(1920),
            video_height: Some(1080),
            mime_type: Some("video/H264".to_string()),
            transport_state: XbxEngineTransportStateDto::Connected,
            video_bytes_total: 72_000,
            video_packet_count_total: 600,
            audio_bytes_total: 3_000,
            observed_at_ms: now_ms + 100.0,
        });
    });
    let _ = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryAnchor".to_string(),
            observed_at_ms: now_ms + 120.0,
        },
        &shared_stats,
    );
    let fallback = RuntimeStatsSink::read_shared(&shared_stats, |stats| {
        (
            stats.recovery_hard_fallback_timer_ms,
            stats.recovery_hard_fallback_trigger_reason.clone(),
            stats.recovery_hard_fallback_timer_reset_reason.clone(),
        )
    })
    .unwrap_or((None, None, None));
    assert!(fallback.0.is_none());
    assert!(fallback.1.is_none());
    assert_eq!(fallback.2.as_deref(), Some("explicitHealthyCleanAnchor"));

    RuntimeStatsSink::update_shared(&shared_stats, |stats| {
        stats.video_anchor_clean_epoch = None;
        stats.video_anchor_clean_observed_at_ms = None;
        stats.video_anchor_clean_source_event = None;
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 78,
            source_event: "frame-await-recovery-anchor".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "recovering".to_string(),
                reason: Some("streamThinStall".to_string()),
                chain_break_evidence: None,

                observed_at_ms: now_ms + 1_000.0,
            },
            observed_at_ms: now_ms + 1_000.0,
        });
        stats.video_renderer_stalled = Some(true);
        stats.latest_video_host_present_time_ms = Some(now_ms - 8_000.0);
    });
    let after_reentry = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryAnchor".to_string(),
            observed_at_ms: now_ms + 1_000.0,
        },
        &shared_stats,
    );
    assert_ne!(
        after_reentry.decision.action,
        RecoveryAction::RequestReconnectCandidate
    );
}
