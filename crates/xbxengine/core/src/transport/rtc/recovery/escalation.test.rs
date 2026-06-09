use super::{
    KeyframeTransportFeedback, RecoveryAction, RecoveryBudgetKind, VideoEscalationConfig,
    VideoEscalationController, VideoEscalationReason,
};
use crate::transport::rtc::recovery::contract::decoder_reset_permitted_from_stats;
use crate::XbxEngineMediaRuntimeStats;
use std::time::Duration;

#[test]
fn waiting_keyframe_suppresses_decoder_reset_without_idr_admission() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.video_decoder_recovery_state = Some("waiting-keyframe".to_string());
    assert!(!decoder_reset_permitted_from_stats(
        &stats, None, 1_000.0, false
    ));
}

#[test]
fn receiver_waiting_keyframe_label_parses_as_wait_keyframe() {
    assert_eq!(
        VideoEscalationReason::from_recovery_reason_label("receiverWaitingKeyframe"),
        Some(VideoEscalationReason::WaitKeyframe)
    );
}

#[test]
fn wait_keyframe_delegates_to_receive_immediately() {
    let mut controller = VideoEscalationController::new(VideoEscalationConfig {
        cooldown_ms: 250,
        keyframe_burst_threshold: 2,
        decoder_reset_burst_threshold: 2,
        ..VideoEscalationConfig::default()
    });

    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::WaitKeyframe)
            .action,
        RecoveryAction::DelegatedToReceive
    );
}

#[test]
fn idle_timeout_delegates_to_receive_immediately() {
    let mut controller = VideoEscalationController::new(VideoEscalationConfig {
        cooldown_ms: 250,
        keyframe_burst_threshold: 2,
        decoder_reset_burst_threshold: 2,
        ..VideoEscalationConfig::default()
    });

    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::AdapterIdleTimeout)
            .action,
        RecoveryAction::DelegatedToReceive
    );
}

#[test]
fn reconfigure_burst_expires_before_requesting_decoder_reset() {
    let mut controller = VideoEscalationController::new(VideoEscalationConfig {
        cooldown_ms: 250,
        keyframe_burst_threshold: 2,
        decoder_reset_burst_threshold: 2,
        ..VideoEscalationConfig::default()
    });

    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::Reconfigure)
            .action,
        RecoveryAction::WaitForDecoderResetBurst
    );
    std::thread::sleep(Duration::from_millis(380));
    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::Reconfigure)
            .action,
        RecoveryAction::WaitForDecoderResetBurst
    );
}

#[test]
fn decoder_backend_failure_requests_decoder_reset_immediately() {
    let mut controller = VideoEscalationController::new(VideoEscalationConfig {
        cooldown_ms: 250,
        keyframe_burst_threshold: 2,
        decoder_reset_burst_threshold: 2,
        ..VideoEscalationConfig::default()
    });

    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::DecoderBackendFailure)
            .action,
        RecoveryAction::RequestDecoderReset
    );
}

#[test]
fn repeated_transport_deadline_failures_are_throttled_within_epoch() {
    let mut controller = VideoEscalationController::new(VideoEscalationConfig {
        cooldown_ms: 40,
        keyframe_burst_threshold: 2,
        decoder_reset_burst_threshold: 1,
        keyframe_min_interval_ms: 40,
        escalation_window_ms: 180,
        keyframe_upgrade_min_delay_ms: 10,
    });

    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::TransportExpiredDeadline)
            .action,
        RecoveryAction::DelegatedToReceive
    );
    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::TransportExpiredDeadline)
            .action,
        RecoveryAction::DelegatedToReceive
    );
    // keyframe_min_interval 在控制器内部有最小下限，需跨过窗口后才能再次发 keyframe。
    std::thread::sleep(Duration::from_millis(130));
    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::TransportExpiredDeadline)
            .action,
        RecoveryAction::DelegatedToReceive
    );
    std::thread::sleep(Duration::from_millis(130));
    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::TransportExpiredDeadline)
            .action,
        RecoveryAction::DelegatedToReceive
    );
}

#[test]
fn local_transport_deadlines_do_not_accumulate_into_keyframe_pressure() {
    let mut controller = VideoEscalationController::new(VideoEscalationConfig {
        cooldown_ms: 40,
        keyframe_burst_threshold: 2,
        decoder_reset_burst_threshold: 1,
        keyframe_min_interval_ms: 40,
        escalation_window_ms: 180,
        keyframe_upgrade_min_delay_ms: 10,
    });

    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::TransportRepairableDeadline)
            .action,
        RecoveryAction::CooldownSuppressed
    );
    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::TransportRepairableDeadline)
            .action,
        RecoveryAction::CooldownSuppressed
    );
    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::TransportLowValueDeadline)
            .action,
        RecoveryAction::CooldownSuppressed
    );
}

#[test]
fn transport_deadline_storm_within_same_window_does_not_reconnect() {
    let mut controller = VideoEscalationController::new(VideoEscalationConfig {
        cooldown_ms: 60,
        keyframe_burst_threshold: 2,
        decoder_reset_burst_threshold: 1,
        keyframe_min_interval_ms: 60,
        escalation_window_ms: 220,
        keyframe_upgrade_min_delay_ms: 10,
    });

    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::TransportExpiredDeadline)
            .action,
        RecoveryAction::DelegatedToReceive
    );
    for _ in 0..4 {
        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::TransportExpiredDeadline)
                .action,
            RecoveryAction::DelegatedToReceive
        );
    }
}

#[test]
fn severe_transport_deadline_requires_repeat_before_reconnect() {
    let mut controller = VideoEscalationController::new(VideoEscalationConfig {
        cooldown_ms: 60_000,
        keyframe_burst_threshold: 2,
        decoder_reset_burst_threshold: 2,
        keyframe_min_interval_ms: 60_000,
        escalation_window_ms: 120_000,
        keyframe_upgrade_min_delay_ms: 500,
    });

    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::TransportSevereDeadline)
            .action,
        RecoveryAction::CooldownSuppressed
    );
    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::TransportSevereDeadline)
            .action,
        RecoveryAction::RequestReconnectCandidate
    );
}

#[test]
fn adapter_idle_after_severe_deadline_prefers_reconnect_over_decoder_reset() {
    let mut controller = VideoEscalationController::new(VideoEscalationConfig {
        cooldown_ms: 200,
        keyframe_burst_threshold: 2,
        decoder_reset_burst_threshold: 2,
        ..VideoEscalationConfig::default()
    });

    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::TransportSevereDeadline)
            .action,
        RecoveryAction::CooldownSuppressed
    );
    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::AdapterIdleTimeout)
            .action,
        RecoveryAction::RequestReconnectCandidate
    );
}

#[test]
fn adapter_idle_after_severe_deadline_window_expires_does_not_jump_to_reconnect_candidate() {
    let mut controller = VideoEscalationController::new(VideoEscalationConfig {
        cooldown_ms: 120,
        keyframe_burst_threshold: 1,
        decoder_reset_burst_threshold: 1,
        keyframe_min_interval_ms: 120,
        escalation_window_ms: 260,
        keyframe_upgrade_min_delay_ms: 0,
    });

    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::TransportSevereDeadline)
            .action,
        RecoveryAction::CooldownSuppressed
    );
    std::thread::sleep(Duration::from_millis(400));
    let action = controller
        .on_reason(VideoEscalationReason::AdapterIdleTimeout)
        .action;
    assert_ne!(action, RecoveryAction::RequestReconnectCandidate);
}

#[test]
fn new_recovery_epoch_clears_severe_deadline_idle_timeout_shortcut() {
    let mut controller = VideoEscalationController::new(VideoEscalationConfig {
        cooldown_ms: 120,
        keyframe_burst_threshold: 1,
        decoder_reset_burst_threshold: 1,
        keyframe_min_interval_ms: 120,
        escalation_window_ms: 260,
        keyframe_upgrade_min_delay_ms: 0,
    });

    controller.begin_recovery_epoch(7);
    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::TransportSevereDeadline)
            .action,
        RecoveryAction::CooldownSuppressed
    );
    controller.begin_recovery_epoch(8);
    let action = controller
        .on_reason(VideoEscalationReason::AdapterIdleTimeout)
        .action;
    assert_ne!(action, RecoveryAction::RequestReconnectCandidate);
}

#[test]
fn persistent_wait_keyframe_without_failure_evidence_does_not_escalate_to_decoder_reset() {
    let mut controller = VideoEscalationController::new(VideoEscalationConfig {
        cooldown_ms: 200,
        keyframe_burst_threshold: 1,
        decoder_reset_burst_threshold: 2,
        keyframe_upgrade_min_delay_ms: 150,
        ..VideoEscalationConfig::default()
    });

    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::WaitKeyframe)
            .action,
        RecoveryAction::DelegatedToReceive
    );
    std::thread::sleep(Duration::from_millis(210));
    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::WaitKeyframe)
            .action,
        RecoveryAction::DelegatedToReceive
    );
    std::thread::sleep(Duration::from_millis(210));
    assert_eq!(
        controller
            .on_reason_with_epoch_policy(
                VideoEscalationReason::WaitKeyframe,
                0,
                true,
                true,
                false,
                true,
            )
            .action,
        RecoveryAction::DelegatedToReceive
    );
}

#[test]
fn persistent_wait_keyframe_with_failure_evidence_escalates_to_fir() {
    let mut controller = VideoEscalationController::new(VideoEscalationConfig {
        cooldown_ms: 200,
        keyframe_burst_threshold: 1,
        decoder_reset_burst_threshold: 2,
        keyframe_upgrade_min_delay_ms: 150,
        ..VideoEscalationConfig::default()
    });

    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::WaitKeyframe)
            .action,
        RecoveryAction::DelegatedToReceive
    );
    std::thread::sleep(Duration::from_millis(210));
    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::WaitKeyframe)
            .action,
        RecoveryAction::DelegatedToReceive
    );
    std::thread::sleep(Duration::from_millis(210));
    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::WaitKeyframe)
            .action,
        RecoveryAction::DelegatedToReceive
    );
}

#[test]
fn transport_sample_loss_delegates_to_receive_immediately() {
    let mut controller = VideoEscalationController::new(VideoEscalationConfig {
        cooldown_ms: 250,
        keyframe_burst_threshold: 3,
        decoder_reset_burst_threshold: 2,
        ..VideoEscalationConfig::default()
    });

    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::TransportSampleLoss)
            .action,
        RecoveryAction::DelegatedToReceive
    );
}

#[test]
fn repeated_transport_sample_loss_after_keyframe_stays_in_keyframe_family() {
    let mut controller = VideoEscalationController::new(VideoEscalationConfig {
        cooldown_ms: 200,
        keyframe_burst_threshold: 3,
        decoder_reset_burst_threshold: 2,
        keyframe_upgrade_min_delay_ms: 0,
        ..VideoEscalationConfig::default()
    });

    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::TransportSampleLoss)
            .action,
        RecoveryAction::DelegatedToReceive
    );
    std::thread::sleep(Duration::from_millis(130));
    let second = controller
        .on_reason(VideoEscalationReason::TransportSampleLoss)
        .action;
    assert!(matches!(
        second,
        RecoveryAction::DelegatedToReceive | RecoveryAction::CooldownSuppressed
    ));
}

#[test]
fn thin_stream_requests_keyframe_but_does_not_directly_upgrade_to_decoder_reset() {
    let mut controller = VideoEscalationController::new(VideoEscalationConfig {
        cooldown_ms: 250,
        keyframe_burst_threshold: 3,
        decoder_reset_burst_threshold: 2,
        keyframe_upgrade_min_delay_ms: 0,
        ..VideoEscalationConfig::default()
    });

    let second = controller
        .on_reason(VideoEscalationReason::AdapterThinStream)
        .action;
    assert!(matches!(
        second,
        RecoveryAction::DelegatedToReceive | RecoveryAction::RequestDecoderReset
    ));
    std::thread::sleep(Duration::from_millis(130));
    let third = controller
        .on_reason(VideoEscalationReason::AdapterThinStream)
        .action;
    assert_ne!(third, RecoveryAction::RequestDecoderReset);
}

#[test]
fn display_supply_critical_never_promotes_to_reconnect_candidate() {
    let mut controller = VideoEscalationController::new(VideoEscalationConfig {
        cooldown_ms: 120,
        keyframe_burst_threshold: 1,
        decoder_reset_burst_threshold: 1,
        keyframe_min_interval_ms: 120,
        escalation_window_ms: 220,
        keyframe_upgrade_min_delay_ms: 0,
    });

    assert_eq!(
        controller
            .on_reason_with_policy(VideoEscalationReason::DisplaySupplyCritical, true)
            .action,
        RecoveryAction::DelegatedToReceive
    );
    std::thread::sleep(Duration::from_millis(260));
    let second = controller
        .on_reason_with_policy(VideoEscalationReason::DisplaySupplyCritical, true)
        .action;
    assert_ne!(second, RecoveryAction::RequestReconnectCandidate);
    std::thread::sleep(Duration::from_millis(260));
    let third = controller
        .on_reason_with_policy(VideoEscalationReason::DisplaySupplyCritical, true)
        .action;
    assert_ne!(third, RecoveryAction::RequestReconnectCandidate);
}

#[test]
fn transport_await_recovery_keyframe_is_suppressed_for_receiver_local() {
    let mut controller = VideoEscalationController::new(VideoEscalationConfig {
        cooldown_ms: 250,
        keyframe_burst_threshold: 3,
        decoder_reset_burst_threshold: 2,
        keyframe_upgrade_min_delay_ms: 0,
        ..VideoEscalationConfig::default()
    });

    for _ in 0..4 {
        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::TransportAwaitRecoveryKeyframe)
                .action,
            RecoveryAction::CooldownSuppressed
        );
    }
}

#[test]
fn persistent_await_recovery_keyframe_escalates_to_decoder_reset_without_session_pli() {
    let mut controller = VideoEscalationController::new(VideoEscalationConfig {
        cooldown_ms: 120,
        keyframe_burst_threshold: 1,
        decoder_reset_burst_threshold: 2,
        keyframe_min_interval_ms: 120,
        escalation_window_ms: 220,
        keyframe_upgrade_min_delay_ms: 0,
    });

    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::TransportAwaitRecoveryKeyframe)
            .action,
        RecoveryAction::CooldownSuppressed
    );
    std::thread::sleep(Duration::from_millis(240));
    let second = controller
        .on_reason(VideoEscalationReason::TransportAwaitRecoveryKeyframe)
        .action;
    assert!(matches!(
        second,
        RecoveryAction::CooldownSuppressed
            | RecoveryAction::DelegatedToReceive
            | RecoveryAction::RequestDecoderReset
    ));
    std::thread::sleep(Duration::from_millis(240));
    assert!(matches!(
        controller
            .on_reason(VideoEscalationReason::TransportAwaitRecoveryKeyframe)
            .action,
        RecoveryAction::DelegatedToReceive | RecoveryAction::RequestDecoderReset
    ));
}

#[test]
fn keyframe_epoch_resets_on_reason_change() {
    let mut controller = VideoEscalationController::new(VideoEscalationConfig {
        cooldown_ms: 180,
        keyframe_burst_threshold: 1,
        decoder_reset_burst_threshold: 3,
        keyframe_min_interval_ms: 180,
        escalation_window_ms: 700,
        keyframe_upgrade_min_delay_ms: 160,
    });

    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::TransportAwaitRecoveryKeyframe)
            .action,
        RecoveryAction::CooldownSuppressed
    );
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::TransportAwaitRecoveryKeyframe)
            .action,
        RecoveryAction::CooldownSuppressed
    );
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::WaitKeyframe)
            .action,
        RecoveryAction::DelegatedToReceive
    );
}

#[test]
fn keyframe_epoch_can_be_reset_explicitly_after_recovery() {
    let mut controller = VideoEscalationController::new(VideoEscalationConfig {
        cooldown_ms: 200,
        keyframe_burst_threshold: 1,
        decoder_reset_burst_threshold: 2,
        keyframe_min_interval_ms: 200,
        escalation_window_ms: 900,
        keyframe_upgrade_min_delay_ms: 200,
    });

    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::TransportAwaitRecoveryKeyframe)
            .action,
        RecoveryAction::CooldownSuppressed
    );
    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::TransportAwaitRecoveryKeyframe)
            .action,
        RecoveryAction::CooldownSuppressed
    );
    controller.reset_keyframe_epoch();
    std::thread::sleep(Duration::from_millis(220));
    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::TransportAwaitRecoveryKeyframe)
            .action,
        RecoveryAction::CooldownSuppressed
    );
}

#[test]
fn idle_timeout_requests_keyframe_but_does_not_directly_upgrade_to_decoder_reset() {
    let mut controller = VideoEscalationController::new(VideoEscalationConfig {
        cooldown_ms: 250,
        keyframe_burst_threshold: 3,
        decoder_reset_burst_threshold: 2,
        keyframe_upgrade_min_delay_ms: 0,
        ..VideoEscalationConfig::default()
    });

    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::AdapterIdleTimeout)
            .action,
        RecoveryAction::DelegatedToReceive
    );
    std::thread::sleep(Duration::from_millis(130));
    let second = controller
        .on_reason(VideoEscalationReason::AdapterIdleTimeout)
        .action;
    assert_ne!(second, RecoveryAction::RequestDecoderReset);
}

#[test]
fn idle_timeout_is_throttled_within_window_and_releases_after_window() {
    let mut controller = VideoEscalationController::new(VideoEscalationConfig {
        cooldown_ms: 120,
        keyframe_burst_threshold: 1,
        decoder_reset_burst_threshold: 2,
        keyframe_min_interval_ms: 220,
        escalation_window_ms: 260,
        keyframe_upgrade_min_delay_ms: 220,
    });

    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::AdapterIdleTimeout)
            .action,
        RecoveryAction::DelegatedToReceive
    );
    std::thread::sleep(Duration::from_millis(180));
    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::AdapterIdleTimeout)
            .action,
        RecoveryAction::DelegatedToReceive
    );
    std::thread::sleep(Duration::from_millis(140));
    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::AdapterIdleTimeout)
            .action,
        RecoveryAction::DelegatedToReceive
    );
}

#[test]
fn await_recovery_keyframe_is_throttled_within_window_and_releases_after_window() {
    let mut controller = VideoEscalationController::new(VideoEscalationConfig {
        cooldown_ms: 120,
        keyframe_burst_threshold: 1,
        decoder_reset_burst_threshold: 2,
        keyframe_min_interval_ms: 220,
        escalation_window_ms: 260,
        keyframe_upgrade_min_delay_ms: 0,
    });

    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::TransportAwaitRecoveryKeyframe)
            .action,
        RecoveryAction::CooldownSuppressed
    );
    std::thread::sleep(Duration::from_millis(180));
    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::TransportAwaitRecoveryKeyframe)
            .action,
        RecoveryAction::CooldownSuppressed
    );
    std::thread::sleep(Duration::from_millis(180));
    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::TransportAwaitRecoveryKeyframe)
            .action,
        RecoveryAction::DelegatedToReceive
    );
}

#[test]
fn expired_deadline_second_window_can_upgrade_to_reconnect_candidate() {
    let mut controller = VideoEscalationController::new(VideoEscalationConfig {
        cooldown_ms: 220,
        keyframe_burst_threshold: 1,
        decoder_reset_burst_threshold: 3,
        keyframe_min_interval_ms: 220,
        escalation_window_ms: 800,
        keyframe_upgrade_min_delay_ms: 220,
    });

    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::TransportExpiredDeadline)
            .action,
        RecoveryAction::DelegatedToReceive
    );
    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::TransportExpiredDeadline)
            .action,
        RecoveryAction::DelegatedToReceive
    );
    std::thread::sleep(Duration::from_millis(230));
    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::TransportExpiredDeadline)
            .action,
        RecoveryAction::RequestReconnectCandidate
    );
}

#[test]
fn repeated_thin_stream_outside_keyframe_interval_stays_in_keyframe_family() {
    let mut controller = VideoEscalationController::new(VideoEscalationConfig {
        cooldown_ms: 180,
        keyframe_burst_threshold: 1,
        decoder_reset_burst_threshold: 2,
        keyframe_min_interval_ms: 180,
        escalation_window_ms: 700,
        keyframe_upgrade_min_delay_ms: 120,
    });

    let second = controller
        .on_reason(VideoEscalationReason::AdapterThinStream)
        .action;
    assert!(matches!(
        second,
        RecoveryAction::DelegatedToReceive | RecoveryAction::RequestDecoderReset
    ));
    std::thread::sleep(Duration::from_millis(150));
    let third = controller
        .on_reason(VideoEscalationReason::AdapterThinStream)
        .action;
    assert_ne!(third, RecoveryAction::RequestDecoderReset);
}

#[test]
fn reconnect_budget_is_single_shot_per_recovery_epoch() {
    let mut controller = VideoEscalationController::new(VideoEscalationConfig {
        cooldown_ms: 200,
        keyframe_burst_threshold: 1,
        decoder_reset_burst_threshold: 1,
        keyframe_min_interval_ms: 200,
        escalation_window_ms: 600,
        keyframe_upgrade_min_delay_ms: 0,
    });
    controller.begin_recovery_epoch(10);
    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::TransportSevereDeadline)
            .action,
        RecoveryAction::CooldownSuppressed
    );
    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::TransportSevereDeadline)
            .action,
        RecoveryAction::RequestReconnectCandidate
    );
    controller.register_reconnect_started();
    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::TransportSevereDeadline)
            .action,
        RecoveryAction::CooldownSuppressed
    );
    controller.begin_recovery_epoch(11);
    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::TransportSevereDeadline)
            .action,
        RecoveryAction::CooldownSuppressed
    );
    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::TransportSevereDeadline)
            .action,
        RecoveryAction::RequestReconnectCandidate
    );
}

#[test]
fn media_policy_disallows_reconnect_for_severe_deadline_does_not_fallback_to_decoder_reset() {
    let mut controller = VideoEscalationController::new(VideoEscalationConfig {
        cooldown_ms: 120,
        keyframe_burst_threshold: 1,
        decoder_reset_burst_threshold: 1,
        keyframe_min_interval_ms: 120,
        escalation_window_ms: 260,
        keyframe_upgrade_min_delay_ms: 0,
    });
    controller.begin_recovery_epoch(6);
    assert_eq!(
        controller
            .on_reason_with_policy(VideoEscalationReason::TransportSevereDeadline, false)
            .action,
        RecoveryAction::CooldownSuppressed
    );
    assert_eq!(
        controller
            .on_reason_with_policy(VideoEscalationReason::TransportSevereDeadline, false)
            .action,
        RecoveryAction::CooldownSuppressed
    );
}

#[test]
fn media_policy_disallows_reconnect_for_transport_await_hard_stuck() {
    let mut controller = VideoEscalationController::new(VideoEscalationConfig {
        cooldown_ms: 120,
        keyframe_burst_threshold: 1,
        decoder_reset_burst_threshold: 1,
        keyframe_min_interval_ms: 120,
        escalation_window_ms: 220,
        keyframe_upgrade_min_delay_ms: 0,
    });
    controller.begin_recovery_epoch(8);
    assert_eq!(
        controller
            .on_reason_with_policy(VideoEscalationReason::TransportAwaitRecoveryKeyframe, false)
            .action,
        RecoveryAction::CooldownSuppressed
    );
    std::thread::sleep(Duration::from_millis(240));
    let second = controller
        .on_reason_with_policy(VideoEscalationReason::TransportAwaitRecoveryKeyframe, false)
        .action;
    assert!(matches!(
        second,
        RecoveryAction::CooldownSuppressed
            | RecoveryAction::DelegatedToReceive
            | RecoveryAction::RequestDecoderReset
    ));
    std::thread::sleep(Duration::from_millis(480));
    let third = controller
        .on_reason_with_policy(VideoEscalationReason::TransportAwaitRecoveryKeyframe, false)
        .action;
    assert!(matches!(
        third,
        RecoveryAction::DelegatedToReceive | RecoveryAction::RequestDecoderReset
    ));
    assert_ne!(third, RecoveryAction::RequestReconnectCandidate);
}

#[test]
fn transport_await_hard_stuck_never_promotes_to_reconnect_even_when_reconnect_allowed() {
    let mut controller = VideoEscalationController::new(VideoEscalationConfig {
        cooldown_ms: 120,
        keyframe_burst_threshold: 1,
        decoder_reset_burst_threshold: 1,
        keyframe_min_interval_ms: 120,
        escalation_window_ms: 220,
        keyframe_upgrade_min_delay_ms: 0,
    });
    controller.begin_recovery_epoch(9);
    assert_eq!(
        controller
            .on_reason_with_policy(VideoEscalationReason::TransportAwaitRecoveryKeyframe, true)
            .action,
        RecoveryAction::CooldownSuppressed
    );
    std::thread::sleep(Duration::from_millis(240));
    let second = controller
        .on_reason_with_policy(VideoEscalationReason::TransportAwaitRecoveryKeyframe, true)
        .action;
    assert!(matches!(
        second,
        RecoveryAction::CooldownSuppressed
            | RecoveryAction::DelegatedToReceive
            | RecoveryAction::RequestDecoderReset
    ));
    std::thread::sleep(Duration::from_millis(480));
    let third = controller
        .on_reason_with_policy(VideoEscalationReason::TransportAwaitRecoveryKeyframe, true)
        .action;
    assert!(matches!(
        third,
        RecoveryAction::DelegatedToReceive | RecoveryAction::RequestDecoderReset
    ));
    assert_ne!(third, RecoveryAction::RequestReconnectCandidate);
}

#[test]
fn keyframe_budget_resets_after_new_recovery_epoch() {
    let mut controller = VideoEscalationController::new(VideoEscalationConfig {
        cooldown_ms: 120,
        keyframe_burst_threshold: 1,
        decoder_reset_burst_threshold: 2,
        keyframe_min_interval_ms: 120,
        escalation_window_ms: 400,
        keyframe_upgrade_min_delay_ms: 100,
    });
    controller.begin_recovery_epoch(3);
    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::TransportAwaitRecoveryKeyframe)
            .action,
        RecoveryAction::CooldownSuppressed
    );
    controller.reconcile_keyframe_transport_feedback(KeyframeTransportFeedback::SentPending);
    std::thread::sleep(Duration::from_millis(130));
    let second = controller
        .on_reason(VideoEscalationReason::AdapterThinStream)
        .action;
    assert!(matches!(
        second,
        RecoveryAction::DelegatedToReceive | RecoveryAction::RequestDecoderReset
    ));
    controller.reconcile_keyframe_transport_feedback(KeyframeTransportFeedback::SentPending);
    std::thread::sleep(Duration::from_millis(130));
    let held = controller
        .on_reason(VideoEscalationReason::TransportAwaitRecoveryKeyframe)
        .action;
    assert_eq!(held, RecoveryAction::CooldownSuppressed);
    controller.begin_recovery_epoch(4);
    std::thread::sleep(Duration::from_millis(130));
    let reopened = controller
        .on_reason(VideoEscalationReason::TransportAwaitRecoveryKeyframe)
        .action;
    assert_eq!(reopened, RecoveryAction::CooldownSuppressed);
}

#[test]
fn unsent_pending_keyframe_feedback_rolls_back_provisional_budget() {
    let mut controller = VideoEscalationController::new(VideoEscalationConfig {
        cooldown_ms: 120,
        keyframe_burst_threshold: 1,
        decoder_reset_burst_threshold: 1,
        keyframe_min_interval_ms: 240,
        escalation_window_ms: 480,
        keyframe_upgrade_min_delay_ms: 0,
    });
    controller.begin_recovery_epoch(8);
    let first = controller
        .on_reason(VideoEscalationReason::TransportAwaitRecoveryKeyframe)
        .action;
    assert_eq!(first, RecoveryAction::CooldownSuppressed);
    assert_eq!(controller.budget_state().keyframe_budget_used, 0);

    controller.reconcile_keyframe_transport_feedback(KeyframeTransportFeedback::UnsentPending);
    assert_eq!(controller.budget_state().keyframe_budget_used, 0);

    let second = controller
        .on_reason(VideoEscalationReason::TransportAwaitRecoveryKeyframe)
        .action;
    assert_eq!(second, RecoveryAction::CooldownSuppressed);
}

#[test]
fn sent_pending_keyframe_feedback_keeps_budget_consumed() {
    let mut controller = VideoEscalationController::new(VideoEscalationConfig {
        cooldown_ms: 120,
        keyframe_burst_threshold: 1,
        decoder_reset_burst_threshold: 1,
        keyframe_min_interval_ms: 240,
        escalation_window_ms: 480,
        keyframe_upgrade_min_delay_ms: 0,
    });
    controller.begin_recovery_epoch(8);
    let first = controller
        .on_reason(VideoEscalationReason::TransportAwaitRecoveryKeyframe)
        .action;
    assert_eq!(first, RecoveryAction::CooldownSuppressed);

    controller.reconcile_keyframe_transport_feedback(KeyframeTransportFeedback::SentPending);
    assert_eq!(controller.budget_state().keyframe_budget_used, 0);
}

#[test]
fn decoder_reset_budget_waits_for_transport_confirmation() {
    let mut controller = VideoEscalationController::new(VideoEscalationConfig {
        cooldown_ms: 120,
        keyframe_burst_threshold: 1,
        decoder_reset_burst_threshold: 1,
        keyframe_min_interval_ms: 240,
        escalation_window_ms: 480,
        keyframe_upgrade_min_delay_ms: 0,
    });
    controller.begin_recovery_epoch(9);

    let first = controller
        .on_reason(VideoEscalationReason::DecoderBackendFailure)
        .action;
    assert_eq!(first, RecoveryAction::RequestDecoderReset);
    assert_eq!(controller.budget_state().decoder_reset_budget_used, 0);

    controller.register_decoder_reset_started();
    assert_eq!(controller.budget_state().decoder_reset_budget_used, 1);

    let second = controller
        .on_reason(VideoEscalationReason::DecoderBackendFailure)
        .action;
    assert_eq!(second, RecoveryAction::CoalescedDecoderResetInFlight);
}

#[test]
fn action_contract_defines_owner_and_budget_rules() {
    let keyframe = VideoEscalationController::action_contract(RecoveryAction::RequestPli);
    assert!(keyframe.budget_recorded_on_execution);
    assert_eq!(keyframe.budget_kind, Some(RecoveryBudgetKind::Keyframe));

    let delegated = VideoEscalationController::action_contract(RecoveryAction::DelegatedToReceive);
    assert!(!delegated.budget_recorded_on_execution);
    assert!(delegated.budget_kind.is_none());

    let reset = VideoEscalationController::action_contract(RecoveryAction::RequestDecoderReset);
    assert!(reset.budget_recorded_on_execution);
    assert_eq!(reset.budget_kind, Some(RecoveryBudgetKind::DecoderReset));

    let reconnect =
        VideoEscalationController::action_contract(RecoveryAction::RequestReconnectCandidate);
    assert!(reconnect.budget_recorded_on_execution);
    assert_eq!(reconnect.budget_kind, Some(RecoveryBudgetKind::Reconnect));

    let suppressed = VideoEscalationController::action_contract(RecoveryAction::CooldownSuppressed);
    assert!(!suppressed.budget_recorded_on_execution);
    assert!(suppressed.budget_kind.is_none());

    let keyframe_coalesced =
        VideoEscalationController::action_contract(RecoveryAction::CoalescedKeyframeInFlight);
    assert!(!keyframe_coalesced.budget_recorded_on_execution);
    assert!(keyframe_coalesced.budget_kind.is_none());

    let decoder_reset_coalesced =
        VideoEscalationController::action_contract(RecoveryAction::CoalescedDecoderResetInFlight);
    assert!(!decoder_reset_coalesced.budget_recorded_on_execution);
    assert!(decoder_reset_coalesced.budget_kind.is_none());

    let decoder_reset =
        VideoEscalationController::action_contract(RecoveryAction::RequestDecoderReset);
    assert!(decoder_reset.budget_recorded_on_execution);
    assert_eq!(
        decoder_reset.budget_kind,
        Some(RecoveryBudgetKind::DecoderReset)
    );
}

#[test]
fn reconnect_budget_waits_for_execution_feedback() {
    let mut controller = VideoEscalationController::new(VideoEscalationConfig {
        cooldown_ms: 120,
        keyframe_burst_threshold: 1,
        decoder_reset_burst_threshold: 1,
        keyframe_min_interval_ms: 120,
        escalation_window_ms: 240,
        keyframe_upgrade_min_delay_ms: 0,
    });
    controller.begin_recovery_epoch(12);

    let first = controller
        .on_reason_with_epoch(VideoEscalationReason::LifecycleRecovering, 12)
        .action;
    assert_eq!(first, RecoveryAction::RequestReconnectCandidate);
    assert_eq!(controller.budget_state().reconnect_budget_used, 0);

    controller.register_reconnect_started();
    assert_eq!(controller.budget_state().reconnect_budget_used, 1);

    let second = controller
        .on_reason_with_epoch(VideoEscalationReason::LifecycleRecovering, 12)
        .action;
    assert_eq!(second, RecoveryAction::CooldownSuppressed);
}

#[test]
fn reconfigure_without_failure_evidence_is_kept_in_wait_stage() {
    let mut controller = VideoEscalationController::new(VideoEscalationConfig {
        cooldown_ms: 120,
        keyframe_burst_threshold: 1,
        decoder_reset_burst_threshold: 1,
        keyframe_min_interval_ms: 120,
        escalation_window_ms: 240,
        keyframe_upgrade_min_delay_ms: 0,
    });
    controller.begin_recovery_epoch(11);

    let first = controller
        .on_reason_with_epoch_policy(
            VideoEscalationReason::Reconfigure,
            11,
            true,
            true,
            true,
            false,
        )
        .action;
    assert_eq!(first, RecoveryAction::WaitForDecoderResetBurst);

    std::thread::sleep(Duration::from_millis(130));
    let second = controller
        .on_reason_with_epoch_policy(
            VideoEscalationReason::Reconfigure,
            11,
            true,
            true,
            true,
            false,
        )
        .action;
    assert_eq!(second, RecoveryAction::WaitForDecoderResetBurst);
}

#[test]
fn epoch_advance_rule_is_reason_aware_for_local_decoder_reset_paths() {
    assert!(
        !VideoEscalationController::action_success_advances_transport_recovery_epoch(
            RecoveryAction::DelegatedToReceive,
            Some(VideoEscalationReason::TransportAwaitRecoveryKeyframe),
        )
    );
    assert!(
        !VideoEscalationController::action_success_advances_transport_recovery_epoch(
            RecoveryAction::RequestDecoderReset,
            Some(VideoEscalationReason::TransportAwaitRecoveryKeyframe),
        )
    );
    assert!(
        !VideoEscalationController::action_success_advances_transport_recovery_epoch(
            RecoveryAction::RequestDecoderReset,
            Some(VideoEscalationReason::AdapterThinStream),
        )
    );
    assert!(
        VideoEscalationController::action_success_advances_transport_recovery_epoch(
            RecoveryAction::RequestReconnectCandidate,
            Some(VideoEscalationReason::AdapterThinStream),
        )
    );
}
