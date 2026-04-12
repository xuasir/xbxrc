use super::super::{RecoveryCoordinator, RecoveryOwnerSignal};
use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::recovery::escalation::{RecoveryAction, VideoEscalationReason};
use crate::transport::rtc::recovery::runtime_state::{resolve_recovery_profile, unix_now_ms};
use crate::transport::rtc::recovery::startup::SessionPhase;
use crate::XbxEngineMediaRuntimeStats;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use xbxengine_protocol::{XbxEngineTargetTypeDto, XbxEngineTransportStateDto};

use super::harness::{
    healthy_twcc_observation, make_test_nack_observation, test_escalation_controller,
};

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

#[test]
fn recovery_profile_pre_first_frame_fallback_ms_matches_session_policy() {
    let mut home = XbxEngineMediaRuntimeStats::default();
    home.session_target_type = Some(XbxEngineTargetTypeDto::Home);
    home.transport_path = Some("Direct (host->host)".to_string());
    let home_profile = resolve_recovery_profile(&Mutex::new(home));
    assert_eq!(
        home_profile.pre_first_frame_reconnect_fallback_ms(),
        15_000.0
    );

    let mut cloud = XbxEngineMediaRuntimeStats::default();
    cloud.session_target_type = Some(XbxEngineTargetTypeDto::Cloud);
    cloud.transport_path = Some("Direct (host->host)".to_string());
    let cloud_profile = resolve_recovery_profile(&Mutex::new(cloud));
    assert_eq!(
        cloud_profile.pre_first_frame_reconnect_fallback_ms(),
        35_000.0
    );
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
            status_detail: None,
            requested_at_ms: now_ms,
            sent_at_ms: Some(now_ms),
            deadline_at_ms: Some(now_ms + 240.0),
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
            status_detail: None,
            requested_at_ms: now_ms,
            sent_at_ms: None,
            deadline_at_ms: Some(now_ms + 240.0),
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
        });
    stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
        observation_id: 71,
        source_event: "frame-await-recovery-keyframe".to_string(),
        gap: None,
        frame: None,
        chain: crate::XbxEngineVideoTimelineChainSnapshot {
            state: "recovering".to_string(),
            reason: Some("transportAwaitRecoveryKeyframe".to_string()),
            chain_break_evidence: None,

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
    assert_eq!(second.action, RecoveryAction::RequestKeyframe);

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
    assert_eq!(second.action, RecoveryAction::RequestKeyframe);

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
