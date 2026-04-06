use super::{RecoveryCoordinator, RecoveryOwnerSignal};
use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::recovery::escalation::{
    RecoveryAction, VideoEscalationConfig, VideoEscalationController, VideoEscalationReason,
};
use crate::transport::rtc::recovery::runtime_state::{resolve_recovery_profile, unix_now_ms};
use crate::transport::rtc::recovery::startup::SessionPhase;
use crate::XbxEngineMediaRuntimeStats;
use crate::{
    XbxEngineVideoNackObservation, XbxEngineVideoTrackStatus, XbxEngineVideoTwccObservation,
};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use xbxengine_protocol::{XbxEngineTargetTypeDto, XbxEngineTransportStateDto};

fn test_escalation_controller(
    cooldown_ms: u64,
    keyframe_burst_threshold: u8,
    decoder_reset_burst_threshold: u8,
) -> VideoEscalationController {
    VideoEscalationController::new(VideoEscalationConfig {
        cooldown_ms,
        keyframe_burst_threshold,
        decoder_reset_burst_threshold,
        keyframe_min_interval_ms: cooldown_ms,
        escalation_window_ms: cooldown_ms.saturating_mul(3),
        keyframe_upgrade_min_delay_ms: (cooldown_ms / 2).max(40),
    })
}

#[test]
fn home_lan_uses_aggressive_startup_recovery_profile() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.session_target_type = Some(XbxEngineTargetTypeDto::Home);
    stats.transport_path = Some("Direct (host->host)".to_string());
    let profile = resolve_recovery_profile(&Mutex::new(stats));
    assert!(profile.startup_fast_reset_enabled);
    assert_eq!(profile.startup_low_quality_retry_delay_ms, 320);
    assert_eq!(profile.startup_low_quality_floor_kbps, 8_000.0);
    assert_eq!(profile.startup_low_quality_recovered_kbps, 12_000.0);
    assert_eq!(profile.escalation_cooldown_ms, 260);
    assert_eq!(profile.escalation_keyframe_min_interval_ms, 260);
}

#[test]
fn relay_home_uses_conservative_startup_recovery_profile() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.session_target_type = Some(XbxEngineTargetTypeDto::Home);
    stats.transport_path = Some("Relay".to_string());
    let profile = resolve_recovery_profile(&Mutex::new(stats));
    assert!(!profile.startup_fast_reset_enabled);
    assert_eq!(profile.startup_low_quality_retry_delay_ms, 650);
    assert_eq!(profile.startup_low_quality_floor_kbps, 6_000.0);
    assert_eq!(profile.startup_low_quality_recovered_kbps, 10_000.0);
    assert_eq!(profile.escalation_cooldown_ms, 360);
    assert_eq!(profile.escalation_keyframe_min_interval_ms, 360);
}

#[test]
fn cloud_uses_less_throttled_recovery_profile() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.session_target_type = Some(XbxEngineTargetTypeDto::Cloud);
    stats.transport_path = Some("Direct (host->host)".to_string());
    let profile = resolve_recovery_profile(&Mutex::new(stats));
    assert!(!profile.startup_fast_reset_enabled);
    assert_eq!(profile.startup_low_quality_retry_delay_ms, 650);
    assert_eq!(profile.startup_low_quality_floor_kbps, 14_000.0);
    assert_eq!(profile.startup_low_quality_recovered_kbps, 20_000.0);
    assert_eq!(profile.escalation_cooldown_ms, 420);
    assert_eq!(profile.escalation_keyframe_min_interval_ms, 420);
    assert_eq!(profile.escalation_upgrade_window_ms, 1_800);
    assert_eq!(profile.escalation_keyframe_upgrade_min_delay_ms, 300);
    assert_eq!(profile.hard_fallback_transport_await_timeout_ms, 4_500);
    assert_eq!(
        profile.display_supply_thresholds.degraded_no_pending_streak,
        64
    );
    assert_eq!(
        profile.display_supply_thresholds.critical_no_pending_streak,
        128
    );
}

fn healthy_twcc_observation(now_ms: f64) -> XbxEngineVideoTwccObservation {
    XbxEngineVideoTwccObservation {
        observation_id: 1,
        source: "local-feedback".to_string(),
        feedback_packet_count: 20,
        covered_sequence_start: 10,
        covered_sequence_end: 29,
        covered_sequence_span: 20,
        observed_packet_count: 20,
        observed_byte_count: 32_000,
        coverage_ratio: None,
        ledger_hit_ratio: None,
        feedback_interval_ms: Some(100.0),
        arrival_span_ms: Some(95.0),
        receive_bitrate_kbps: Some(18_000.0),
        twcc_sample_valid: true,

        twcc_invalid_reason: None,

        quality: crate::XbxEngineTwccObservationQuality::Stable,
        delivery_ratio: 0.99,
        packet_loss_ratio: 0.01,
        observed_at_ms: now_ms,
    }
}

fn make_test_nack_observation(
    action: &str,
    frame_importance: &str,
    retry_count: u8,
    observed_at_ms: f64,
) -> XbxEngineVideoNackObservation {
    XbxEngineVideoNackObservation {
        observation_id: 1,
        action: action.to_string(),
        source: "sampleLoss".to_string(),
        first_sequence: 1,
        last_sequence: 2,
        packet_count: 2,
        retry_count,
        frame_rtp_timestamp: Some(1),
        frame_is_keyframe: Some(frame_importance == "keyframe"),
        frame_importance: Some(frame_importance.to_string()),
        deadline_at_ms: None,
        estimated_recovery_arrival_ms: None,
        nack_disposition: Some("attempted".to_string()),
        frame_playout_deadline_at_ms: None,
        frame_unrecoverable_reason: None,
        frame_budget: None,
        observed_at_ms,
    }
}

#[test]
fn recovered_nack_suppresses_transport_sample_loss_escalation() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.latest_video_nack_observation = Some(make_test_nack_observation(
        "recovered",
        "delta",
        0,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as f64,
    ));
    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(250, 2, 2),
        Instant::now(),
        Duration::from_millis(800),
    );
    let decision = coordinator.on_reason_with_runtime_stats(
        VideoEscalationReason::TransportSampleLoss,
        &Mutex::new(stats),
    );
    assert_eq!(decision.action, RecoveryAction::CooldownSuppressed);
}

#[test]
fn display_supply_critical_does_not_produce_reconnect_candidate() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_recovery_epoch = 9;
    stats.transport_state = XbxEngineTransportStateDto::Connected;
    stats.video_renderer_stalled = Some(true);
    stats.latest_video_host_present_time_ms = Some(now_ms - 2_400.0);
    stats.latest_video_decode_ok_time_ms = Some(now_ms - 1_400.0);
    let shared_stats = Mutex::new(stats);
    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(120, 1, 1),
        Instant::now() - Duration::from_secs(3),
        Duration::from_millis(800),
    );

    let first = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::DisplaySupplyCritical,
            reason_label: "displaySupplyCritical".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    assert_ne!(
        first.decision.action,
        RecoveryAction::RequestReconnectCandidate
    );

    let second = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::DisplaySupplyCritical,
            reason_label: "displaySupplyCritical".to_string(),
            observed_at_ms: now_ms + 320.0,
        },
        &shared_stats,
    );
    assert_ne!(
        second.decision.action,
        RecoveryAction::RequestReconnectCandidate
    );
}

#[test]
fn display_supply_critical_stays_local_when_important_nack_expires() {
    let observed_at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as f64;
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.latest_video_nack_observation = Some(make_test_nack_observation(
        "expiredDeadline",
        "keyframe",
        1,
        observed_at_ms,
    ));
    let shared_stats = Mutex::new(stats);
    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(250, 1, 1),
        Instant::now(),
        Duration::from_millis(800),
    );
    let proposal = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::DisplaySupplyCritical,
            reason_label: "displaySupplyCritical".to_string(),
            observed_at_ms,
        },
        &shared_stats,
    );
    assert_ne!(
        proposal.signal.reason,
        VideoEscalationReason::TransportAwaitRecoveryKeyframe
    );
    assert_ne!(
        proposal.decision.action,
        RecoveryAction::RequestReconnectCandidate
    );
}

#[test]
fn expired_delta_nack_stays_suppressed_without_stall_signal() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Home);
    stats.latest_video_nack_observation = Some(make_test_nack_observation(
        "expiredDeadline",
        "delta",
        2,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as f64,
    ));
    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(250, 2, 2),
        Instant::now(),
        Duration::from_millis(800),
    );
    let decision = coordinator.on_reason_with_runtime_stats(
        VideoEscalationReason::TransportSampleLoss,
        &Mutex::new(stats),
    );
    assert_eq!(decision.action, RecoveryAction::CooldownSuppressed);
}

#[test]
fn expired_delta_nack_in_cloud_requires_continuous_budget_before_keyframe() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud);
    let observed_at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as f64;
    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(250, 1, 1),
        Instant::now(),
        Duration::from_secs(2),
    );

    stats.latest_video_nack_observation = Some(make_test_nack_observation(
        "expiredDeadline",
        "delta",
        2,
        observed_at_ms,
    ));
    let shared_stats = Mutex::new(stats);
    let decision = coordinator.on_reason_with_runtime_stats(
        VideoEscalationReason::TransportExpiredDeadline,
        &shared_stats,
    );
    assert_eq!(decision.action, RecoveryAction::CooldownSuppressed);

    RuntimeStatsSink::update_shared(&shared_stats, |stats| {
        if let Some(observation) = stats.latest_video_nack_observation.as_mut() {
            observation.observation_id = 2;
            observation.observed_at_ms += 180.0;
        }
    });
    let decision = coordinator.on_reason_with_runtime_stats(
        VideoEscalationReason::TransportExpiredDeadline,
        &shared_stats,
    );
    assert_eq!(decision.action, RecoveryAction::CooldownSuppressed);

    RuntimeStatsSink::update_shared(&shared_stats, |stats| {
        if let Some(observation) = stats.latest_video_nack_observation.as_mut() {
            observation.observation_id = 3;
            observation.observed_at_ms += 180.0;
        }
    });
    let decision = coordinator.on_reason_with_runtime_stats(
        VideoEscalationReason::TransportExpiredDeadline,
        &shared_stats,
    );
    assert_eq!(decision.action, RecoveryAction::RequestKeyframe);
}

#[test]
fn expired_delta_nack_requests_keyframe_when_pipeline_is_stalled() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.video_renderer_stalled = Some(true);
    stats.latest_video_host_present_time_ms = Some(now_ms - 2_000.0);
    stats.latest_video_packet_arrival_time_ms = Some(now_ms - 40.0);
    stats.latest_video_nack_observation = Some(make_test_nack_observation(
        "expiredDeadline",
        "delta",
        2,
        now_ms,
    ));
    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(250, 1, 1),
        Instant::now() - Duration::from_secs(5),
        Duration::from_millis(800),
    );
    let decision = coordinator.on_reason_with_runtime_stats(
        VideoEscalationReason::TransportExpiredDeadline,
        &Mutex::new(stats),
    );
    assert_eq!(decision.action, RecoveryAction::RequestKeyframe);
}

#[test]
fn decoder_backend_failure_prioritizes_decoder_reset_over_transport_suppression() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_state = XbxEngineTransportStateDto::Connected;
    stats.latest_video_packet_arrival_time_ms = Some(now_ms - 30.0);
    stats.latest_video_decode_ok_time_ms = Some(now_ms - 1_800.0);
    stats.latest_video_host_present_time_ms = Some(now_ms - 1_800.0);
    stats.video_renderer_stalled = Some(true);
    stats.latest_video_twcc_observation = Some(healthy_twcc_observation(now_ms - 20.0));
    stats.video_decoder_hardware_failure_streak = 4;
    stats.latest_video_decoder_hardware_failure_time_ms = Some(now_ms - 25.0);
    stats.latest_video_decoder_reset_time_ms = Some(now_ms - 2_500.0);
    stats.latest_video_nack_observation = Some(make_test_nack_observation(
        "expiredDeadline",
        "delta",
        2,
        now_ms,
    ));

    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(250, 2, 2),
        Instant::now() - Duration::from_secs(5),
        Duration::from_millis(800),
    );
    let decision = coordinator.on_reason_with_runtime_stats(
        VideoEscalationReason::TransportExpiredDeadline,
        &Mutex::new(stats),
    );
    assert_eq!(decision.action, RecoveryAction::RequestDecoderReset);
}

#[test]
fn decoder_backend_failure_respects_reset_spacing_cooldown() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_state = XbxEngineTransportStateDto::Connected;
    stats.latest_video_packet_arrival_time_ms = Some(now_ms - 30.0);
    stats.latest_video_decode_ok_time_ms = Some(now_ms - 1_800.0);
    stats.latest_video_host_present_time_ms = Some(now_ms - 1_800.0);
    stats.video_renderer_stalled = Some(true);
    stats.latest_video_twcc_observation = Some(healthy_twcc_observation(now_ms - 20.0));
    stats.video_decoder_hardware_failure_streak = 5;
    stats.latest_video_decoder_hardware_failure_time_ms = Some(now_ms - 15.0);
    stats.latest_video_decoder_reset_time_ms = Some(now_ms - 100.0);

    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(250, 2, 2),
        Instant::now() - Duration::from_secs(5),
        Duration::from_millis(800),
    );
    let decision = coordinator.on_reason_with_runtime_stats(
        VideoEscalationReason::TransportExpiredDeadline,
        &Mutex::new(stats),
    );
    assert_eq!(decision.action, RecoveryAction::CooldownSuppressed);
}

#[test]
fn runtime_state_overrides_transport_diagnosis_to_decoder_backend_failure() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_state = XbxEngineTransportStateDto::Connected;
    stats.latest_video_packet_arrival_time_ms = Some(now_ms - 20.0);
    stats.latest_video_decode_ok_time_ms = Some(now_ms - 1_800.0);
    stats.latest_video_host_present_time_ms = Some(now_ms - 1_800.0);
    stats.video_renderer_stalled = Some(true);
    stats.latest_video_twcc_observation = Some(healthy_twcc_observation(now_ms - 20.0));
    stats.video_decoder_hardware_failure_streak = 3;
    stats.latest_video_decoder_hardware_failure_time_ms = Some(now_ms - 20.0);

    let state = RecoveryCoordinator::runtime_state_for_diagnosis(
        &Mutex::new(stats),
        "transportExpiredDeadline",
        Instant::now() - Duration::from_secs(3),
        Duration::from_millis(800),
    );
    assert_eq!(state.phase, SessionPhase::Recovering);
    assert_eq!(state.input_profile.baseline, "homeLanGaming");
    assert_eq!(
        state.input_profile.effective_label,
        "homeLanGaming+decoderConstrained"
    );
    assert_eq!(state.primary_view.owner_state, "rebuilding-supply");
    assert_eq!(state.primary_view.owner_reason, "decoderBackendFailure");
    assert_eq!(state.diagnosis_label, "decoderBackendFailure");
}

#[test]
fn runtime_state_keeps_transport_diagnosis_when_pipeline_is_still_advancing() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_state = XbxEngineTransportStateDto::Connected;
    stats.latest_video_packet_arrival_time_ms = Some(now_ms - 20.0);
    stats.latest_video_decode_ok_time_ms = Some(now_ms - 60.0);
    stats.latest_video_host_present_time_ms = Some(now_ms - 60.0);
    stats.video_renderer_stalled = Some(false);
    stats.latest_video_twcc_observation = Some(healthy_twcc_observation(now_ms - 20.0));
    stats.video_decoder_hardware_failure_streak = 4;
    stats.latest_video_decoder_hardware_failure_time_ms = Some(now_ms - 20.0);

    let state = RecoveryCoordinator::runtime_state_for_diagnosis(
        &Mutex::new(stats),
        "transportExpiredDeadline",
        Instant::now() - Duration::from_secs(3),
        Duration::from_millis(800),
    );
    assert_eq!(state.phase, SessionPhase::Steady);
    assert_eq!(state.input_profile.baseline, "homeLanGaming");
    assert_eq!(state.primary_view.owner_state, "stable-serving");
    assert_eq!(state.primary_view.owner_reason, "transportExpiredDeadline");
    assert_eq!(state.diagnosis_label, "transportExpiredDeadline");
}

#[test]
fn recovered_reference_nack_waits_for_burst_in_wait_keyframe_chain() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    let mut observation = make_test_nack_observation(
        "recovered",
        "reference",
        0,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as f64,
    );
    observation.frame_is_keyframe = Some(true);
    stats.latest_video_nack_observation = Some(observation);
    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(250, 2, 2),
        Instant::now(),
        Duration::from_millis(800),
    );
    let decision = coordinator
        .on_reason_with_runtime_stats(VideoEscalationReason::WaitKeyframe, &Mutex::new(stats));
    assert_eq!(decision.action, RecoveryAction::WaitForBurst);
}

#[test]
fn expired_reference_nack_pushes_idle_timeout_into_recovery_chain() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    let mut observation = make_test_nack_observation(
        "expiredDeadline",
        "reference",
        2,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as f64,
    );
    observation.frame_is_keyframe = Some(true);
    stats.latest_video_nack_observation = Some(observation);
    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(250, 1, 1),
        Instant::now() - Duration::from_secs(5),
        Duration::from_millis(800),
    );
    let decision = coordinator.on_reason_with_runtime_stats(
        VideoEscalationReason::AdapterIdleTimeout,
        &Mutex::new(stats),
    );
    assert_eq!(decision.action, RecoveryAction::RequestKeyframe);
}

#[test]
fn recent_wait_keyframe_recovery_suppresses_repeat_wait_keyframe() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_recovery_epoch = 4;
    stats.transport_recovery_epoch_at_last_escalation = 4;
    stats.latest_video_escalation_observation = Some(crate::XbxEngineVideoEscalationObservation {
        observation_id: 7,
        reason: "ingressWaitKeyframe".to_string(),
        action: "requestKeyframe".to_string(),
        recovery_stage: "rebuilding-supply".to_string(),
        recovery_chain_value: "anchor".to_string(),
        recovery_failure_cost: "medium".to_string(),
        recovery_window_source: "transport-await-window".to_string(),
        observed_at_ms: now_ms,
    });
    stats.latest_keyframe_request_episode =
        Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
            episode_id: 7,
            request_reason: Some("waitKeyframe".to_string()),
            request_kind: Some("pli".to_string()),
            status: "sent".to_string(),
            requested_at_ms: now_ms,
            sent_at_ms: Some(now_ms),
            deadline_at_ms: Some(now_ms + 240.0),
            first_keyframe_packet_at_ms: None,
            first_keyframe_decoded_at_ms: None,
            response_rtp_timestamp: None,
            response_frame_seq: None,
            response_verdict: Some("pending".to_string()),
        });
    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(250, 2, 2),
        Instant::now(),
        Duration::from_millis(800),
    );
    let decision = coordinator
        .on_reason_with_runtime_stats(VideoEscalationReason::WaitKeyframe, &Mutex::new(stats));
    assert_eq!(decision.action, RecoveryAction::CoalescedKeyframeInFlight);
}

#[test]
fn recent_wait_keyframe_decoder_reset_suppresses_repeat_wait_keyframe_reset() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_recovery_epoch = 4;
    stats.transport_recovery_epoch_at_last_escalation = 4;
    stats.latest_video_escalation_observation = Some(crate::XbxEngineVideoEscalationObservation {
        observation_id: 17,
        reason: "ingressWaitKeyframe".to_string(),
        action: "requestDecoderReset".to_string(),
        recovery_stage: "rebuilding-supply".to_string(),
        recovery_chain_value: "anchor".to_string(),
        recovery_failure_cost: "high".to_string(),
        recovery_window_source: "transport-await-window".to_string(),
        observed_at_ms: now_ms,
    });
    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(250, 1, 1),
        Instant::now(),
        Duration::from_millis(800),
    );

    let decision = coordinator
        .on_reason_with_runtime_stats(VideoEscalationReason::WaitKeyframe, &Mutex::new(stats));
    assert_eq!(
        decision.action,
        RecoveryAction::CoalescedDecoderResetInFlight
    );
}

#[test]
fn recent_wait_keyframe_without_sent_episode_does_not_coalesce_keyframe_inflight() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_recovery_epoch = 4;
    stats.transport_recovery_epoch_at_last_escalation = 4;
    stats.latest_video_escalation_observation = Some(crate::XbxEngineVideoEscalationObservation {
        observation_id: 70,
        reason: "transportAwaitRecoveryKeyframe".to_string(),
        action: "requestKeyframe".to_string(),
        recovery_stage: "rebuilding-supply".to_string(),
        recovery_chain_value: "anchor".to_string(),
        recovery_failure_cost: "medium".to_string(),
        recovery_window_source: "transport-await-window".to_string(),
        observed_at_ms: now_ms,
    });
    stats.latest_keyframe_request_episode =
        Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
            episode_id: 70,
            request_reason: Some("transportAwaitRecoveryKeyframe".to_string()),
            request_kind: None,
            status: "requested".to_string(),
            requested_at_ms: now_ms,
            sent_at_ms: None,
            deadline_at_ms: Some(now_ms + 240.0),
            first_keyframe_packet_at_ms: None,
            first_keyframe_decoded_at_ms: None,
            response_rtp_timestamp: None,
            response_frame_seq: None,
            response_verdict: Some("pending".to_string()),
        });
    stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
        observation_id: 71,
        source_event: "frame-await-recovery-keyframe".to_string(),
        gap: None,
        frame: None,
        chain: crate::XbxEngineVideoTimelineChainSnapshot {
            state: "recovering".to_string(),
            reason: Some("transportAwaitRecoveryKeyframe".to_string()),
            observed_at_ms: now_ms,
        },
        observed_at_ms: now_ms,
    });
    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(250, 2, 2),
        Instant::now(),
        Duration::from_millis(800),
    );

    let decision = coordinator.on_reason_with_runtime_stats(
        VideoEscalationReason::TransportAwaitRecoveryKeyframe,
        &Mutex::new(stats),
    );
    assert_ne!(decision.action, RecoveryAction::CoalescedKeyframeInFlight);
}

#[test]
fn wait_keyframe_without_failure_evidence_does_not_upgrade_to_decoder_reset() {
    let shared_stats = Mutex::new(XbxEngineMediaRuntimeStats {
        transport_recovery_epoch: 12,
        ..Default::default()
    });
    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(120, 1, 1),
        Instant::now() - Duration::from_secs(3),
        Duration::from_millis(800),
    );

    let first = coordinator
        .on_reason_with_runtime_stats(VideoEscalationReason::WaitKeyframe, &shared_stats);
    assert_eq!(first.action, RecoveryAction::RequestKeyframe);

    std::thread::sleep(Duration::from_millis(130));
    let second = coordinator
        .on_reason_with_runtime_stats(VideoEscalationReason::WaitKeyframe, &shared_stats);
    assert_eq!(second.action, RecoveryAction::CoalescedKeyframeInFlight);

    std::thread::sleep(Duration::from_millis(130));
    let third = coordinator
        .on_reason_with_runtime_stats(VideoEscalationReason::WaitKeyframe, &shared_stats);
    assert_ne!(third.action, RecoveryAction::RequestDecoderReset);
}

#[test]
fn rejected_wait_keyframe_anchor_candidate_upgrades_to_decoder_reset() {
    let now_ms = unix_now_ms();
    let shared_stats = Mutex::new(XbxEngineMediaRuntimeStats {
        transport_recovery_epoch: 13,
        latest_anchor_candidate_ledger: Some(crate::XbxEngineAnchorCandidateLedger {
            state: crate::XbxEngineAnchorCandidateState::Rejected,
            source_event: "frame-await-recovery-keyframe".to_string(),
            frame_rtp_timestamp: Some(2207340890),
            recovery_epoch: 13,
            failure_reason: Some(
                crate::XbxEngineAnchorCandidateFailureReason::AwaitingRecoveryKeyframe,
            ),
            observed_at_ms: now_ms,
        }),
        ..Default::default()
    });
    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(120, 1, 1),
        Instant::now() - Duration::from_secs(3),
        Duration::from_millis(800),
    );

    let first = coordinator
        .on_reason_with_runtime_stats(VideoEscalationReason::WaitKeyframe, &shared_stats);
    assert_eq!(first.action, RecoveryAction::RequestKeyframe);

    std::thread::sleep(Duration::from_millis(130));
    let second = coordinator
        .on_reason_with_runtime_stats(VideoEscalationReason::WaitKeyframe, &shared_stats);
    assert_eq!(second.action, RecoveryAction::CoalescedKeyframeInFlight);

    RuntimeStatsSink::update_shared(&shared_stats, |stats| {
        if let Some(candidate) = stats.latest_anchor_candidate_ledger.as_mut() {
            candidate.observed_at_ms = unix_now_ms();
        }
    });

    std::thread::sleep(Duration::from_millis(130));
    let third = coordinator
        .on_reason_with_runtime_stats(VideoEscalationReason::WaitKeyframe, &shared_stats);
    assert_eq!(third.action, RecoveryAction::RequestDecoderReset);
}

#[test]
fn new_transport_recovery_epoch_breaks_wait_keyframe_suppression() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_recovery_epoch = 5;
    stats.transport_recovery_epoch_at_last_escalation = 4;
    stats.latest_video_escalation_observation = Some(crate::XbxEngineVideoEscalationObservation {
        observation_id: 17,
        reason: "ingressWaitKeyframe".to_string(),
        action: "requestKeyframe".to_string(),
        recovery_stage: "rebuilding-supply".to_string(),
        recovery_chain_value: "anchor".to_string(),
        recovery_failure_cost: "medium".to_string(),
        recovery_window_source: "transport-await-window".to_string(),
        observed_at_ms: now_ms - 40.0,
    });
    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(250, 2, 2),
        Instant::now(),
        Duration::from_millis(800),
    );
    let decision = coordinator
        .on_reason_with_runtime_stats(VideoEscalationReason::WaitKeyframe, &Mutex::new(stats));
    assert_ne!(decision.action, RecoveryAction::CooldownSuppressed);
}

#[test]
fn cooldown_suppressed_observation_does_not_self_lock_wait_keyframe_chain() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.latest_video_escalation_observation = Some(crate::XbxEngineVideoEscalationObservation {
        observation_id: 9,
        reason: "ingressWaitKeyframe".to_string(),
        action: "cooldownSuppressed".to_string(),
        recovery_stage: "rebuilding-supply".to_string(),
        recovery_chain_value: "anchor".to_string(),
        recovery_failure_cost: "low".to_string(),
        recovery_window_source: "transport-await-window".to_string(),
        observed_at_ms: now_ms - 50.0,
    });
    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(250, 2, 2),
        Instant::now() - Duration::from_secs(3),
        Duration::from_millis(800),
    );
    let decision = coordinator
        .on_reason_with_runtime_stats(VideoEscalationReason::WaitKeyframe, &Mutex::new(stats));
    assert_ne!(decision.action, RecoveryAction::CooldownSuppressed);
}

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
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    assert_eq!(first.decision.action, RecoveryAction::RequestKeyframe);

    let second = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
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
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    assert_eq!(first.decision.action, RecoveryAction::RequestKeyframe);
    assert_eq!(first.budget_after.keyframe_budget_used, 1);

    RuntimeStatsSink::update_shared(&shared_stats, |stats| {
        stats.latest_keyframe_request_episode =
            Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                episode_id: first.decision.observation_id,
                request_reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                request_kind: None,
                status: "requested".to_string(),
                requested_at_ms: now_ms,
                sent_at_ms: None,
                deadline_at_ms: Some(now_ms + 960.0),
                first_keyframe_packet_at_ms: None,
                first_keyframe_decoded_at_ms: None,
                response_rtp_timestamp: None,
                response_frame_seq: None,
                response_verdict: Some("pending".to_string()),
            });
    });

    std::thread::sleep(Duration::from_millis(140));
    let second = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
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
fn thin_stall_timeout_alone_does_not_keep_transport_await_repeat_suppressed() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_recovery_epoch = 27;
    stats.transport_recovery_epoch_at_last_escalation = 27;
    stats.latest_video_escalation_observation = Some(crate::XbxEngineVideoEscalationObservation {
        observation_id: 901,
        reason: "transportAwaitRecoveryKeyframe".to_string(),
        action: "requestKeyframe".to_string(),
        recovery_stage: "rebuilding-supply".to_string(),
        recovery_chain_value: "transportAwaitRecoveryKeyframe".to_string(),
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
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    assert_eq!(first.decision.action, RecoveryAction::RequestKeyframe);

    RuntimeStatsSink::update_shared(&shared_stats, |stats| {
        stats.latest_keyframe_request_episode =
            Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                episode_id: first.decision.observation_id,
                request_reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                request_kind: None,
                status: "sent".to_string(),
                requested_at_ms: now_ms,
                sent_at_ms: Some(now_ms + 10.0),
                deadline_at_ms: Some(now_ms + 960.0),
                first_keyframe_packet_at_ms: None,
                first_keyframe_decoded_at_ms: None,
                response_rtp_timestamp: None,
                response_frame_seq: None,
                response_verdict: Some("pending".to_string()),
            });
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 903,
            source_event: "timeout-stream-thin-stall".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "broken".to_string(),
                reason: Some("streamThinStall".to_string()),
                observed_at_ms: now_ms + 140.0,
            },
            observed_at_ms: now_ms + 140.0,
        });
    });

    std::thread::sleep(Duration::from_millis(140));
    let second = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
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
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    assert_eq!(first.decision.action, RecoveryAction::RequestKeyframe);

    RuntimeStatsSink::update_shared(&shared_stats, |stats| {
        stats.latest_keyframe_request_episode =
            Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                episode_id: first.decision.observation_id,
                request_reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                request_kind: Some("pli".to_string()),
                status: "sent".to_string(),
                requested_at_ms: now_ms,
                sent_at_ms: Some(now_ms + 10.0),
                deadline_at_ms: Some(now_ms + 960.0),
                first_keyframe_packet_at_ms: None,
                first_keyframe_decoded_at_ms: None,
                response_rtp_timestamp: None,
                response_frame_seq: None,
                response_verdict: Some("pending".to_string()),
            });
        stats.latest_video_rtcp_send_failure_time_ms = Some(now_ms + 130.0);
        stats.latest_video_rtcp_send_failure_reason =
            Some("xbxEngineRtcVideoRtcpFeedbackTargetUnavailable".to_string());
    });

    std::thread::sleep(Duration::from_millis(140));
    let second = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
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
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    assert_eq!(first.decision.action, RecoveryAction::RequestKeyframe);

    RuntimeStatsSink::update_shared(&shared_stats, |stats| {
        stats.latest_keyframe_request_episode =
            Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                episode_id: first.decision.observation_id,
                request_reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                request_kind: Some("pli".to_string()),
                status: "decoded".to_string(),
                requested_at_ms: now_ms,
                sent_at_ms: Some(now_ms + 10.0),
                deadline_at_ms: Some(now_ms + 960.0),
                first_keyframe_packet_at_ms: Some(now_ms + 80.0),
                first_keyframe_decoded_at_ms: Some(now_ms + 90.0),
                response_rtp_timestamp: Some(1_234),
                response_frame_seq: Some(55),
                response_verdict: Some("on-time".to_string()),
            });
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 905,
            source_event: "frame-await-recovery-keyframe".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "recovering".to_string(),
                reason: Some("awaitingRecoveryKeyframe".to_string()),
                observed_at_ms: now_ms + 360.0,
            },
            observed_at_ms: now_ms + 360.0,
        });
    });

    std::thread::sleep(Duration::from_millis(360));
    let second = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
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
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    assert_eq!(first.decision.action, RecoveryAction::RequestKeyframe);

    RuntimeStatsSink::update_shared(&shared_stats, |stats| {
        stats.latest_keyframe_request_episode =
            Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                episode_id: first.decision.observation_id,
                request_reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                request_kind: Some("pli".to_string()),
                status: "missed".to_string(),
                requested_at_ms: now_ms,
                sent_at_ms: Some(now_ms + 10.0),
                deadline_at_ms: Some(now_ms + 120.0),
                first_keyframe_packet_at_ms: None,
                first_keyframe_decoded_at_ms: None,
                response_rtp_timestamp: None,
                response_frame_seq: None,
                response_verdict: Some("missed".to_string()),
            });
    });

    std::thread::sleep(Duration::from_millis(140));
    let second = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
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
                reason: "transportAwaitRecoveryKeyframe".to_string(),
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
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
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
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
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
            source_event: "frame-await-recovery-keyframe".to_string(),
            failure_reason: Some(
                crate::XbxEngineAnchorCandidateFailureReason::ChainBrokenReferenceUnrecoverable,
            ),
            observed_at_ms: now_ms + 140.0,
        });
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 906,
            source_event: "frame-await-recovery-keyframe".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "broken".to_string(),
                reason: Some("referenceChainUnrecoverable".to_string()),
                observed_at_ms: now_ms + 140.0,
            },
            observed_at_ms: now_ms + 140.0,
        });
    });

    std::thread::sleep(Duration::from_millis(140));
    let second = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms + 140.0,
        },
        &shared_stats,
    );
    assert_eq!(second.decision.action, RecoveryAction::RequestDecoderReset);
}

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
fn transport_expired_deadline_hard_pause_prefers_decoder_reset_over_reconnect() {
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
    assert_eq!(decision.action, RecoveryAction::RequestDecoderReset);
}

#[test]
fn startup_low_quality_falls_back_to_rebuilding_supply_primary_view() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.session_phase = Some("startup".to_string());
    stats.direct_gaming_bitrate_band = Some("startupLow".to_string());
    let state = RecoveryCoordinator::runtime_state_for_diagnosis(
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
    let state = RecoveryCoordinator::runtime_state_for_diagnosis(
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
    let state = RecoveryCoordinator::runtime_state_for_diagnosis(
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

    let state = RecoveryCoordinator::runtime_state_for_diagnosis(
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
    let proposal = coordinator.propose_from_owner_signal(signal, &shared_stats);
    assert_eq!(
        proposal.signal.reason,
        VideoEscalationReason::TransportSampleLoss
    );
    assert_eq!(proposal.signal.reason_label, "displaySupplyNoPending");
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
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    assert_eq!(first.decision.action, RecoveryAction::RequestKeyframe);

    let second = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
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
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms + 420.0,
        },
        &shared_stats,
    );
    assert_eq!(third.decision.action, RecoveryAction::RequestKeyframe);
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
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    assert_eq!(first.decision.action, RecoveryAction::RequestKeyframe);

    std::thread::sleep(Duration::from_millis(220));
    let second = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms + 220.0,
        },
        &shared_stats,
    );
    assert_eq!(
        second.decision.action,
        RecoveryAction::CoalescedKeyframeInFlight
    );
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
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    assert_eq!(first.decision.action, RecoveryAction::RequestKeyframe);

    let second = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
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
    stats.video_anchor_clean_source_event = Some("chain-clean-keyframe-submitted".to_string());
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
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    let _ = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms + 60.0,
        },
        &shared_stats,
    );
    let third = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
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
        source_event: "chain-clean-keyframe-submitted".to_string(),
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
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    let _ = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms + 60.0,
        },
        &shared_stats,
    );
    let third = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
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
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    let _ = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms + 120.0,
        },
        &shared_stats,
    );

    coordinator.acknowledge_clean_anchor();

    let after_clean_anchor = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
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
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
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
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms + 1_600.0,
        },
        &shared_stats,
    );
    RuntimeStatsSink::update_shared(&shared_stats, |stats| {
        stats.latest_video_escalation_observation =
            Some(crate::XbxEngineVideoEscalationObservation {
                observation_id: 901,
                reason: "transportAwaitRecoveryKeyframe".to_string(),
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
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
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
fn transport_await_reconnecting_stage_promotes_reconnect_after_hard_fallback_timeout() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_recovery_epoch = 31;
    stats.transport_state = XbxEngineTransportStateDto::Connecting;
    stats.transport_recovery_episode_active = true;
    stats.recovery_diagnosis = Some("transportAwaitRecoveryKeyframe".to_string());
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
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    let promoted = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms + 10_000.0,
        },
        &shared_stats,
    );
    assert_eq!(
        promoted.decision.action,
        RecoveryAction::RequestReconnectCandidate
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
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    RuntimeStatsSink::update_shared(&shared_stats, |stats| {
        stats.video_anchor_clean_epoch = Some(30);
        stats.video_anchor_clean_observed_at_ms = Some(now_ms + 100.0);
        stats.video_anchor_clean_source_event = Some("chain-clean-keyframe-submitted".to_string());
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 77,
            source_event: "frame-observed".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "healthy".to_string(),
                reason: None,
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
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
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
            source_event: "frame-await-recovery-keyframe".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "recovering".to_string(),
                reason: Some("streamThinStall".to_string()),
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
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms + 1_000.0,
        },
        &shared_stats,
    );
    assert_ne!(
        after_reentry.decision.action,
        RecoveryAction::RequestReconnectCandidate
    );
}

#[test]
fn transport_await_hard_fallback_does_not_inherit_timeout_window_after_decoder_reset_and_short_healthy_reentry(
) {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.session_target_type = Some(XbxEngineTargetTypeDto::Cloud);
    stats.transport_recovery_epoch = 33;
    stats.transport_state = XbxEngineTransportStateDto::Connected;
    stats.video_renderer_stalled = Some(true);
    stats.latest_video_host_present_time_ms = Some(now_ms - 8_000.0);
    stats.host_no_pending_pressure_level = Some("critical".to_string());
    stats.host_no_pending_streak = 320;
    stats.inbound_primary_video_bytes_total = 64_000;
    stats.latest_video_track_status = Some(XbxEngineVideoTrackStatus {
        state: "remoteTrackAttached".to_string(),
        video_width: Some(1920),
        video_height: Some(1080),
        mime_type: Some("video/H264".to_string()),
        transport_state: XbxEngineTransportStateDto::Connected,
        video_bytes_total: 96_000,
        video_packet_count_total: 400,
        audio_bytes_total: 4_000,
        observed_at_ms: now_ms,
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
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    RuntimeStatsSink::update_shared(&shared_stats, |stats| {
        stats.latest_video_decoder_reset_time_ms = Some(now_ms + 180.0);
    });
    let _ = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms + 5_000.0,
        },
        &shared_stats,
    );

    RuntimeStatsSink::update_shared(&shared_stats, |stats| {
        stats.video_anchor_clean_epoch = Some(33);
        stats.video_anchor_clean_observed_at_ms = Some(now_ms + 5_060.0);
        stats.video_anchor_clean_source_event = Some("chain-clean-keyframe-submitted".to_string());
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 3301,
            source_event: "frame-observed".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "healthy".to_string(),
                reason: None,
                observed_at_ms: now_ms + 5_060.0,
            },
            observed_at_ms: now_ms + 5_060.0,
        });
        stats.video_renderer_stalled = Some(false);
        stats.video_decoder_stalled = Some(false);
        stats.latest_video_host_present_time_ms = Some(now_ms + 5_060.0);
    });
    let _ = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms + 5_080.0,
        },
        &shared_stats,
    );

    RuntimeStatsSink::update_shared(&shared_stats, |stats| {
        stats.video_anchor_clean_epoch = None;
        stats.video_anchor_clean_observed_at_ms = None;
        stats.video_anchor_clean_source_event = None;
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 3302,
            source_event: "frame-await-recovery-keyframe".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "recovering".to_string(),
                reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                observed_at_ms: now_ms + 5_240.0,
            },
            observed_at_ms: now_ms + 5_240.0,
        });
        stats.video_renderer_stalled = Some(true);
        stats.video_decoder_stalled = Some(false);
        stats.latest_video_host_present_time_ms = Some(now_ms - 10_000.0);
    });
    let reentry = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms + 5_240.0,
        },
        &shared_stats,
    );
    assert_ne!(
        reentry.decision.action,
        RecoveryAction::RequestReconnectCandidate
    );
    let fallback = RuntimeStatsSink::read_shared(&shared_stats, |stats| {
        (
            stats.recovery_hard_fallback_timer_ms,
            stats.recovery_hard_fallback_trigger_reason.clone(),
        )
    })
    .unwrap_or((None, None));
    assert!(
        fallback.0.is_some_and(|timer_ms| timer_ms < 800.0),
        "hard fallback timer should restart after short healthy reentry, got {:?}",
        fallback.0
    );
    assert_ne!(
        fallback.1.as_deref(),
        Some("transportAwaitRecoveryKeyframeTimeout")
    );
}

#[test]
fn transport_await_hard_fallback_requires_decoder_reset_attempt_before_reconnect() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_recovery_epoch = 32;
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
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    let _ = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms + 1_600.0,
        },
        &shared_stats,
    );
    let timeout = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms + 2_700.0,
        },
        &shared_stats,
    );
    assert_ne!(
        timeout.decision.action,
        RecoveryAction::RequestReconnectCandidate
    );
    assert_ne!(
        timeout.decision.action,
        RecoveryAction::RequestReconnectCandidate
    );
    assert_ne!(timeout.decision.action, RecoveryAction::RequestDecoderReset);
}

#[test]
fn transport_await_hard_fallback_keeps_connected_ingress_local_when_decoder_reset_path_exhausted() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.session_target_type = Some(XbxEngineTargetTypeDto::Cloud);
    stats.transport_recovery_epoch = 41;
    stats.transport_state = XbxEngineTransportStateDto::Connected;
    stats.video_renderer_stalled = Some(true);
    stats.latest_video_host_present_time_ms = Some(now_ms - 9_000.0);
    stats.host_no_pending_pressure_level = Some("critical".to_string());
    stats.host_no_pending_streak = 320;
    stats.inbound_primary_video_bytes_total = 12_000;
    stats.latest_video_track_status = Some(XbxEngineVideoTrackStatus {
        state: "remoteTrackAttached".to_string(),
        video_width: Some(2560),
        video_height: Some(1440),
        mime_type: Some("video/H264".to_string()),
        transport_state: XbxEngineTransportStateDto::Connected,
        video_bytes_total: 42_000,
        video_packet_count_total: 120,
        audio_bytes_total: 2_100,
        observed_at_ms: now_ms,
    });
    let shared_stats = Mutex::new(stats);
    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(120, 1, 1),
        Instant::now() - Duration::from_secs(3),
        Duration::from_millis(800),
    );

    // 先把 decoder reset 预算耗尽。
    let _ = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    let _ = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms + 200.0,
        },
        &shared_stats,
    );

    // 用一次显式 healthy clean anchor 把 hard fallback 计时窗口重置，
    // 让后续 timeout 窗口内“不存在 decoder reset 尝试”。
    RuntimeStatsSink::update_shared(&shared_stats, |stats| {
        stats.video_anchor_clean_epoch = Some(41);
        stats.video_anchor_clean_observed_at_ms = Some(now_ms + 260.0);
        stats.video_anchor_clean_source_event = Some("chain-clean-keyframe-submitted".to_string());
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 1201,
            source_event: "frame-observed".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "healthy".to_string(),
                reason: None,
                observed_at_ms: now_ms + 260.0,
            },
            observed_at_ms: now_ms + 260.0,
        });
        stats.video_renderer_stalled = Some(false);
        stats.latest_video_host_present_time_ms = Some(now_ms + 260.0);
    });
    let _ = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms + 300.0,
        },
        &shared_stats,
    );

    // 回到 Connected + ingress 持续，且仍有新输出推进。
    // 该场景应继续留在本地恢复链（cooldown），不应立即升级到 reconnect。
    RuntimeStatsSink::update_shared(&shared_stats, |stats| {
        stats.video_anchor_clean_epoch = None;
        stats.video_anchor_clean_observed_at_ms = None;
        stats.video_anchor_clean_source_event = None;
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 1202,
            source_event: "frame-await-recovery-keyframe".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "recovering".to_string(),
                reason: Some("streamThinStall".to_string()),
                observed_at_ms: now_ms + 6_900.0,
            },
            observed_at_ms: now_ms + 6_900.0,
        });
        stats.video_renderer_stalled = Some(true);
        stats.latest_video_host_present_time_ms = Some(now_ms + 6_950.0);
        stats.latest_video_track_status = Some(XbxEngineVideoTrackStatus {
            state: "remoteTrackAttached".to_string(),
            video_width: Some(2560),
            video_height: Some(1440),
            mime_type: Some("video/H264".to_string()),
            transport_state: XbxEngineTransportStateDto::Connected,
            video_bytes_total: 88_000,
            video_packet_count_total: 300,
            audio_bytes_total: 4_200,
            observed_at_ms: now_ms + 6_900.0,
        });
        stats.inbound_primary_video_bytes_total = 48_000;
    });
    let timeout = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms + 7_000.0,
        },
        &shared_stats,
    );
    assert_ne!(
        timeout.decision.action,
        RecoveryAction::RequestReconnectCandidate
    );
    assert_eq!(
        timeout.decision.action,
        RecoveryAction::CoalescedKeyframeInFlight
    );
}

#[test]
fn transport_await_hard_fallback_does_not_treat_ingress_without_output_as_local_progress() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.session_target_type = Some(XbxEngineTargetTypeDto::Cloud);
    stats.transport_recovery_epoch = 51;
    stats.transport_state = XbxEngineTransportStateDto::Connected;
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
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    let _ = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms + 200.0,
        },
        &shared_stats,
    );

    RuntimeStatsSink::update_shared(&shared_stats, |stats| {
        stats.latest_video_escalation_observation =
            Some(crate::XbxEngineVideoEscalationObservation {
                observation_id: 9201,
                reason: "transportAwaitRecoveryKeyframe".to_string(),
                action: "cooldownSuppressed".to_string(),
                recovery_stage: "rebuilding-supply".to_string(),
                recovery_chain_value: "anchor".to_string(),
                recovery_failure_cost: "high".to_string(),
                recovery_window_source: "transport-await-window".to_string(),
                observed_at_ms: now_ms + 260.0,
            });
        stats.latest_video_decoder_reset_time_ms = Some(now_ms + 220.0);
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 1203,
            source_event: "frame-await-recovery-keyframe".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "recovering".to_string(),
                reason: Some("awaitingRecoveryKeyframe".to_string()),
                observed_at_ms: now_ms + 6_900.0,
            },
            observed_at_ms: now_ms + 6_900.0,
        });
        stats.latest_video_track_status = Some(XbxEngineVideoTrackStatus {
            state: "remoteTrackAttached".to_string(),
            video_width: Some(1920),
            video_height: Some(1080),
            mime_type: Some("video/H264".to_string()),
            transport_state: XbxEngineTransportStateDto::Connected,
            video_bytes_total: 96_000,
            video_packet_count_total: 340,
            audio_bytes_total: 4_600,
            observed_at_ms: now_ms + 6_900.0,
        });
        stats.inbound_primary_video_bytes_total = 64_000;
        // 明确保持“connected but unrecoverable”：仍在收包，但没有任何新输出推进。
        stats.latest_video_host_present_time_ms = Some(now_ms - 6_000.0);
        stats.latest_video_decode_ok_time_ms = Some(now_ms - 6_000.0);
    });

    let timeout = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms + 7_000.0,
        },
        &shared_stats,
    );
    assert_eq!(
        timeout.decision.action,
        RecoveryAction::RequestReconnectCandidate
    );
}

#[test]
fn transport_await_hard_fallback_keeps_local_when_decode_progress_is_fresh_without_present() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.session_target_type = Some(XbxEngineTargetTypeDto::Cloud);
    stats.transport_recovery_epoch = 52;
    stats.transport_state = XbxEngineTransportStateDto::Connected;
    stats.video_renderer_stalled = Some(true);
    stats.video_decoder_stalled = Some(false);
    stats.host_no_pending_pressure_level = Some("critical".to_string());
    stats.host_no_pending_streak = 360;
    stats.inbound_primary_video_bytes_total = 36_000;
    stats.latest_video_track_status = Some(XbxEngineVideoTrackStatus {
        state: "remoteTrackAttached".to_string(),
        video_width: Some(1920),
        video_height: Some(1080),
        mime_type: Some("video/H264".to_string()),
        transport_state: XbxEngineTransportStateDto::Connected,
        video_bytes_total: 160_000,
        video_packet_count_total: 720,
        audio_bytes_total: 5_200,
        observed_at_ms: now_ms,
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
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    let _ = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms + 200.0,
        },
        &shared_stats,
    );

    RuntimeStatsSink::update_shared(&shared_stats, |stats| {
        stats.latest_video_escalation_observation =
            Some(crate::XbxEngineVideoEscalationObservation {
                observation_id: 9301,
                reason: "transportAwaitRecoveryKeyframe".to_string(),
                action: "cooldownSuppressed".to_string(),
                recovery_stage: "rebuilding-supply".to_string(),
                recovery_chain_value: "anchor".to_string(),
                recovery_failure_cost: "high".to_string(),
                recovery_window_source: "transport-await-window".to_string(),
                observed_at_ms: now_ms + 260.0,
            });
        stats.latest_video_decoder_reset_time_ms = Some(now_ms + 220.0);
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 1204,
            source_event: "frame-await-recovery-keyframe".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "recovering".to_string(),
                reason: Some("awaitingRecoveryKeyframe".to_string()),
                observed_at_ms: now_ms + 6_900.0,
            },
            observed_at_ms: now_ms + 6_900.0,
        });
        if let Some(track) = stats.latest_video_track_status.as_mut() {
            track.observed_at_ms = now_ms + 6_900.0;
            track.video_bytes_total += 28_000;
            track.video_packet_count_total += 140;
        }
        stats.latest_video_host_present_time_ms = Some(now_ms - 6_000.0);
        stats.latest_video_decode_ok_time_ms = Some(now_ms + 6_950.0);
    });

    let timeout = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms + 7_000.0,
        },
        &shared_stats,
    );
    assert_eq!(timeout.decision.action, RecoveryAction::CooldownSuppressed);
}

#[test]
fn transport_await_hard_fallback_upgrades_to_reconnect_after_decode_progress_turns_stale() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.session_target_type = Some(XbxEngineTargetTypeDto::Cloud);
    stats.transport_recovery_epoch = 53;
    stats.transport_state = XbxEngineTransportStateDto::Connected;
    stats.video_renderer_stalled = Some(true);
    stats.video_decoder_stalled = Some(false);
    stats.host_no_pending_pressure_level = Some("critical".to_string());
    stats.host_no_pending_streak = 360;
    stats.inbound_primary_video_bytes_total = 44_000;
    stats.latest_video_track_status = Some(XbxEngineVideoTrackStatus {
        state: "remoteTrackAttached".to_string(),
        video_width: Some(1920),
        video_height: Some(1080),
        mime_type: Some("video/H264".to_string()),
        transport_state: XbxEngineTransportStateDto::Connected,
        video_bytes_total: 188_000,
        video_packet_count_total: 860,
        audio_bytes_total: 6_100,
        observed_at_ms: now_ms,
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
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    let _ = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms + 200.0,
        },
        &shared_stats,
    );

    RuntimeStatsSink::update_shared(&shared_stats, |stats| {
        stats.latest_video_escalation_observation =
            Some(crate::XbxEngineVideoEscalationObservation {
                observation_id: 9401,
                reason: "transportAwaitRecoveryKeyframe".to_string(),
                action: "cooldownSuppressed".to_string(),
                recovery_stage: "rebuilding-supply".to_string(),
                recovery_chain_value: "anchor".to_string(),
                recovery_failure_cost: "high".to_string(),
                recovery_window_source: "transport-await-window".to_string(),
                observed_at_ms: now_ms + 260.0,
            });
        stats.latest_video_decoder_reset_time_ms = Some(now_ms + 220.0);
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 1205,
            source_event: "frame-await-recovery-keyframe".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "recovering".to_string(),
                reason: Some("awaitingRecoveryKeyframe".to_string()),
                observed_at_ms: now_ms + 6_900.0,
            },
            observed_at_ms: now_ms + 6_900.0,
        });
        if let Some(track) = stats.latest_video_track_status.as_mut() {
            track.observed_at_ms = now_ms + 6_900.0;
            track.video_bytes_total += 24_000;
            track.video_packet_count_total += 120;
        }
        stats.latest_video_host_present_time_ms = Some(now_ms - 6_000.0);
        stats.latest_video_decode_ok_time_ms = Some(now_ms + 6_950.0);
    });

    let local = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms + 7_000.0,
        },
        &shared_stats,
    );
    assert_eq!(local.decision.action, RecoveryAction::CooldownSuppressed);

    RuntimeStatsSink::update_shared(&shared_stats, |stats| {
        if let Some(track) = stats.latest_video_track_status.as_mut() {
            track.observed_at_ms = now_ms + 7_180.0;
            track.video_bytes_total += 26_000;
            track.video_packet_count_total += 130;
        }
        stats.latest_video_host_present_time_ms = Some(now_ms - 6_000.0);
        stats.latest_video_decode_ok_time_ms = Some(now_ms - 6_000.0);
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 1206,
            source_event: "frame-await-recovery-keyframe".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "recovering".to_string(),
                reason: Some("awaitingRecoveryKeyframe".to_string()),
                observed_at_ms: now_ms + 7_180.0,
            },
            observed_at_ms: now_ms + 7_180.0,
        });
    });

    let escalated = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms + 7_200.0,
        },
        &shared_stats,
    );
    assert_eq!(
        escalated.decision.action,
        RecoveryAction::RequestReconnectCandidate
    );
}

#[test]
fn transport_await_hard_fallback_resets_on_non_await_reason() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_recovery_epoch = 31;
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
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    let _ = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::AdapterThinStream,
            reason_label: "adapterThinStream".to_string(),
            observed_at_ms: now_ms + 1_200.0,
        },
        &shared_stats,
    );
    let _ = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms + 2_800.0,
        },
        &shared_stats,
    );
    let fallback = RuntimeStatsSink::read_shared(&shared_stats, |stats| {
        (
            stats.recovery_hard_fallback_timer_ms,
            stats.recovery_hard_fallback_timer_reset_reason.clone(),
        )
    })
    .unwrap_or((None, None));
    assert!(fallback.0.is_some_and(|timer_ms| timer_ms <= 1_600.0));
    assert!(fallback.1.is_none());
}

#[test]
fn cooldown_suppressed_cannot_linger_when_connected_track_attached_but_no_present_decode_progress()
{
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.session_target_type = Some(XbxEngineTargetTypeDto::Cloud);
    stats.transport_recovery_epoch = 51;
    stats.transport_state = XbxEngineTransportStateDto::Connected;
    stats.video_renderer_stalled = Some(true);
    stats.video_decoder_stalled = Some(true);
    stats.host_no_pending_pressure_level = Some("critical".to_string());
    stats.host_no_pending_streak = 400;
    stats.inbound_primary_video_bytes_total = 24_000;
    stats.latest_video_track_status = Some(XbxEngineVideoTrackStatus {
        state: "remoteTrackAttached".to_string(),
        video_width: Some(1920),
        video_height: Some(1080),
        mime_type: Some("video/H264".to_string()),
        transport_state: XbxEngineTransportStateDto::Connected,
        video_bytes_total: 128_000,
        video_packet_count_total: 640,
        audio_bytes_total: 4_800,
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
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    assert_eq!(first.decision.action, RecoveryAction::RequestKeyframe);

    let second = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms + 200.0,
        },
        &shared_stats,
    );
    assert_eq!(
        second.decision.action,
        RecoveryAction::CoalescedKeyframeInFlight
    );

    RuntimeStatsSink::update_shared(&shared_stats, |stats| {
        stats.latest_video_escalation_observation =
            Some(crate::XbxEngineVideoEscalationObservation {
                observation_id: 9101,
                reason: "transportAwaitRecoveryKeyframe".to_string(),
                action: "cooldownSuppressed".to_string(),
                recovery_stage: "rebuilding-supply".to_string(),
                recovery_chain_value: "anchor".to_string(),
                recovery_failure_cost: "high".to_string(),
                recovery_window_source: "transport-await-window".to_string(),
                observed_at_ms: now_ms + 260.0,
            });
        stats.latest_video_decoder_reset_time_ms = Some(now_ms + 220.0);
    });

    let timeout = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms + 7_000.0,
        },
        &shared_stats,
    );
    assert_eq!(
        timeout.decision.action,
        RecoveryAction::RequestReconnectCandidate
    );
}

#[test]
fn new_transport_recovery_epoch_clears_severe_deadline_idle_timeout_shortcut() {
    let shared_stats = Mutex::new(XbxEngineMediaRuntimeStats {
        transport_recovery_epoch: 61,
        ..Default::default()
    });
    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(120, 1, 1),
        Instant::now() - Duration::from_secs(3),
        Duration::from_millis(800),
    );

    let first = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportSevereDeadline,
            reason_label: "transportSevereDeadline".to_string(),
            observed_at_ms: unix_now_ms(),
        },
        &shared_stats,
    );
    assert_eq!(first.decision.action, RecoveryAction::CooldownSuppressed);

    RuntimeStatsSink::update_shared(&shared_stats, |stats| {
        stats.transport_recovery_epoch = 62;
    });
    let second = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::AdapterIdleTimeout,
            reason_label: "adapterIdleTimeout".to_string(),
            observed_at_ms: unix_now_ms() + 20.0,
        },
        &shared_stats,
    );
    assert_ne!(
        second.decision.action,
        RecoveryAction::RequestReconnectCandidate
    );
}

#[test]
fn transport_severe_deadline_requires_fresh_second_hit_before_reconnect() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_recovery_epoch = 61;
    let shared_stats = Mutex::new(stats);
    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(120, 1, 1),
        Instant::now() - Duration::from_secs(3),
        Duration::from_millis(800),
    );

    let first = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportSevereDeadline,
            reason_label: "transportSevereDeadline".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    assert_eq!(first.decision.action, RecoveryAction::CooldownSuppressed);

    std::thread::sleep(Duration::from_millis(420));
    let delayed = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportSevereDeadline,
            reason_label: "transportSevereDeadline".to_string(),
            observed_at_ms: now_ms + 420.0,
        },
        &shared_stats,
    );
    assert_eq!(delayed.decision.action, RecoveryAction::CooldownSuppressed);

    let fresh_second = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportSevereDeadline,
            reason_label: "transportSevereDeadline".to_string(),
            observed_at_ms: now_ms + 460.0,
        },
        &shared_stats,
    );
    assert_eq!(
        fresh_second.decision.action,
        RecoveryAction::RequestReconnectCandidate
    );
}

#[test]
fn transport_expired_deadline_window_resets_after_large_gap() {
    let observed_at_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud);
    stats.latest_video_nack_observation = Some(make_test_nack_observation(
        "expiredDeadline",
        "delta",
        2,
        observed_at_ms,
    ));
    let shared_stats = Mutex::new(stats);
    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(120, 1, 1),
        Instant::now() - Duration::from_secs(3),
        Duration::from_millis(800),
    );

    let first = coordinator.on_reason_with_runtime_stats(
        VideoEscalationReason::TransportExpiredDeadline,
        &shared_stats,
    );
    assert_eq!(first.action, RecoveryAction::CooldownSuppressed);

    RuntimeStatsSink::update_shared(&shared_stats, |stats| {
        if let Some(observation) = stats.latest_video_nack_observation.as_mut() {
            observation.observation_id = 2;
            observation.observed_at_ms += 500.0;
        }
    });
    let second = coordinator.on_reason_with_runtime_stats(
        VideoEscalationReason::TransportExpiredDeadline,
        &shared_stats,
    );
    assert_eq!(second.action, RecoveryAction::CooldownSuppressed);

    RuntimeStatsSink::update_shared(&shared_stats, |stats| {
        if let Some(observation) = stats.latest_video_nack_observation.as_mut() {
            observation.observation_id = 3;
            observation.observed_at_ms += 40.0;
        }
    });
    let third = coordinator.on_reason_with_runtime_stats(
        VideoEscalationReason::TransportExpiredDeadline,
        &shared_stats,
    );
    assert_eq!(third.action, RecoveryAction::CooldownSuppressed);
}

#[test]
fn reconnect_budget_is_released_after_new_epoch_for_transport_severe_deadline() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_recovery_epoch = 71;
    let shared_stats = Mutex::new(stats);
    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(120, 1, 1),
        Instant::now() - Duration::from_secs(3),
        Duration::from_millis(800),
    );

    let _ = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportSevereDeadline,
            reason_label: "transportSevereDeadline".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    let first_reconnect = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportSevereDeadline,
            reason_label: "transportSevereDeadline".to_string(),
            observed_at_ms: now_ms + 20.0,
        },
        &shared_stats,
    );
    assert_eq!(
        first_reconnect.decision.action,
        RecoveryAction::RequestReconnectCandidate
    );

    let exhausted = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportSevereDeadline,
            reason_label: "transportSevereDeadline".to_string(),
            observed_at_ms: now_ms + 40.0,
        },
        &shared_stats,
    );
    assert_eq!(
        exhausted.decision.action,
        RecoveryAction::CooldownSuppressed
    );

    RuntimeStatsSink::update_shared(&shared_stats, |stats| {
        stats.transport_recovery_epoch = 72;
    });

    let epoch_reset_first = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportSevereDeadline,
            reason_label: "transportSevereDeadline".to_string(),
            observed_at_ms: now_ms + 80.0,
        },
        &shared_stats,
    );
    assert_eq!(
        epoch_reset_first.decision.action,
        RecoveryAction::CooldownSuppressed
    );

    let epoch_reset_second = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportSevereDeadline,
            reason_label: "transportSevereDeadline".to_string(),
            observed_at_ms: now_ms + 100.0,
        },
        &shared_stats,
    );
    assert_eq!(
        epoch_reset_second.decision.action,
        RecoveryAction::RequestReconnectCandidate
    );
}

#[test]
fn transport_recovered_late_does_not_inherit_severe_deadline_reconnect_counter() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_recovery_epoch = 81;
    let shared_stats = Mutex::new(stats);
    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(120, 1, 1),
        Instant::now() - Duration::from_secs(3),
        Duration::from_millis(800),
    );

    let severe = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportSevereDeadline,
            reason_label: "transportSevereDeadline".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    assert_eq!(severe.decision.action, RecoveryAction::CooldownSuppressed);

    let recovered_late = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportRecoveredLate,
            reason_label: "transportRecoveredLate".to_string(),
            observed_at_ms: now_ms + 20.0,
        },
        &shared_stats,
    );
    assert_ne!(
        recovered_late.decision.action,
        RecoveryAction::RequestReconnectCandidate
    );
    assert!(matches!(
        recovered_late.decision.action,
        RecoveryAction::RequestKeyframe | RecoveryAction::CooldownSuppressed
    ));
}
