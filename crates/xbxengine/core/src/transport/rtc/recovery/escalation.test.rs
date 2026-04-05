use super::{
    RecoveryAction, RecoveryBudgetKind, VideoEscalationConfig, VideoEscalationController,
    VideoEscalationReason,
};
use std::time::Duration;

#[test]
fn waits_for_burst_before_requesting_keyframe() {
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
        RecoveryAction::WaitForBurst
    );
    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::AdapterIdleTimeout)
            .action,
        RecoveryAction::RequestKeyframe
    );
}

#[test]
fn idle_timeout_requests_keyframe_immediately() {
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
        RecoveryAction::RequestKeyframe
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
        RecoveryAction::RequestKeyframe
    );
    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::TransportExpiredDeadline)
            .action,
        RecoveryAction::CooldownSuppressed
    );
    // keyframe_min_interval 在控制器内部有最小下限，需跨过窗口后才能再次发 keyframe。
    std::thread::sleep(Duration::from_millis(130));
    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::TransportExpiredDeadline)
            .action,
        RecoveryAction::CooldownSuppressed
    );
    std::thread::sleep(Duration::from_millis(130));
    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::TransportExpiredDeadline)
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
        RecoveryAction::RequestKeyframe
    );
    for _ in 0..4 {
        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::TransportExpiredDeadline)
                .action,
            RecoveryAction::CooldownSuppressed
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
fn adapter_idle_after_severe_deadline_escalates_to_reconnect_candidate() {
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
fn persistent_wait_keyframe_escalates_to_decoder_reset() {
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
        RecoveryAction::RequestKeyframe
    );
    std::thread::sleep(Duration::from_millis(210));
    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::WaitKeyframe)
            .action,
        RecoveryAction::CooldownSuppressed
    );
    std::thread::sleep(Duration::from_millis(210));
    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::WaitKeyframe)
            .action,
        RecoveryAction::RequestDecoderReset
    );
}

#[test]
fn transport_sample_loss_requests_keyframe_immediately() {
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
        RecoveryAction::RequestKeyframe
    );
}

#[test]
fn repeated_transport_sample_loss_after_keyframe_escalates_to_decoder_reset() {
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
        RecoveryAction::RequestKeyframe
    );
    std::thread::sleep(Duration::from_millis(130));
    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::TransportSampleLoss)
            .action,
        RecoveryAction::RequestDecoderReset
    );
}

#[test]
fn thin_stream_requests_keyframe_then_decoder_reset_quickly() {
    let mut controller = VideoEscalationController::new(VideoEscalationConfig {
        cooldown_ms: 250,
        keyframe_burst_threshold: 3,
        decoder_reset_burst_threshold: 2,
        keyframe_upgrade_min_delay_ms: 0,
        ..VideoEscalationConfig::default()
    });

    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::AdapterThinStream)
            .action,
        RecoveryAction::RequestKeyframe
    );
    std::thread::sleep(Duration::from_millis(130));
    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::AdapterThinStream)
            .action,
        RecoveryAction::RequestDecoderReset
    );
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
        RecoveryAction::RequestKeyframe
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
fn await_recovery_keyframe_is_throttled_within_same_epoch() {
    let mut controller = VideoEscalationController::new(VideoEscalationConfig {
        cooldown_ms: 250,
        keyframe_burst_threshold: 3,
        decoder_reset_burst_threshold: 2,
        keyframe_upgrade_min_delay_ms: 0,
        ..VideoEscalationConfig::default()
    });

    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::TransportAwaitRecoveryKeyframe)
            .action,
        RecoveryAction::RequestKeyframe
    );
    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::TransportAwaitRecoveryKeyframe)
            .action,
        RecoveryAction::CooldownSuppressed
    );
    std::thread::sleep(Duration::from_millis(130));
    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::TransportAwaitRecoveryKeyframe)
            .action,
        RecoveryAction::CooldownSuppressed
    );
    std::thread::sleep(Duration::from_millis(130));
    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::TransportAwaitRecoveryKeyframe)
            .action,
        RecoveryAction::CooldownSuppressed
    );
}

#[test]
fn persistent_await_recovery_keyframe_escalates_to_decoder_reset_then_reconnect() {
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
        RecoveryAction::RequestKeyframe
    );
    std::thread::sleep(Duration::from_millis(240));
    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::TransportAwaitRecoveryKeyframe)
            .action,
        RecoveryAction::RequestDecoderReset
    );
    std::thread::sleep(Duration::from_millis(480));
    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::TransportAwaitRecoveryKeyframe)
            .action,
        RecoveryAction::RequestReconnectCandidate
    );
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
        RecoveryAction::RequestKeyframe
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
        RecoveryAction::RequestKeyframe
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
        RecoveryAction::RequestKeyframe
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
        RecoveryAction::RequestKeyframe
    );
}

#[test]
fn idle_timeout_requests_keyframe_then_decoder_reset_quickly() {
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
        RecoveryAction::RequestKeyframe
    );
    std::thread::sleep(Duration::from_millis(130));
    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::AdapterIdleTimeout)
            .action,
        RecoveryAction::RequestDecoderReset
    );
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
        RecoveryAction::RequestKeyframe
    );
    std::thread::sleep(Duration::from_millis(180));
    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::AdapterIdleTimeout)
            .action,
        RecoveryAction::CooldownSuppressed
    );
    std::thread::sleep(Duration::from_millis(140));
    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::AdapterIdleTimeout)
            .action,
        RecoveryAction::RequestKeyframe
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
        RecoveryAction::RequestKeyframe
    );
    std::thread::sleep(Duration::from_millis(180));
    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::TransportAwaitRecoveryKeyframe)
            .action,
        RecoveryAction::CooldownSuppressed
    );
    std::thread::sleep(Duration::from_millis(140));
    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::TransportAwaitRecoveryKeyframe)
            .action,
        RecoveryAction::RequestDecoderReset
    );
}

#[test]
fn cooldown_window_prevents_keyframe_storm() {
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
        RecoveryAction::RequestKeyframe
    );
    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::TransportExpiredDeadline)
            .action,
        RecoveryAction::CooldownSuppressed
    );
    std::thread::sleep(Duration::from_millis(230));
    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::TransportExpiredDeadline)
            .action,
        RecoveryAction::CooldownSuppressed
    );
}

#[test]
fn repeated_reason_outside_keyframe_interval_can_upgrade_to_decoder_reset() {
    let mut controller = VideoEscalationController::new(VideoEscalationConfig {
        cooldown_ms: 180,
        keyframe_burst_threshold: 1,
        decoder_reset_burst_threshold: 2,
        keyframe_min_interval_ms: 180,
        escalation_window_ms: 700,
        keyframe_upgrade_min_delay_ms: 120,
    });

    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::AdapterThinStream)
            .action,
        RecoveryAction::RequestKeyframe
    );
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::AdapterThinStream)
            .action,
        RecoveryAction::RequestDecoderReset
    );
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
fn media_policy_disallows_reconnect_for_severe_deadline() {
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
        RecoveryAction::RequestDecoderReset
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
        RecoveryAction::RequestKeyframe
    );
    std::thread::sleep(Duration::from_millis(240));
    let second = controller
        .on_reason_with_policy(VideoEscalationReason::TransportAwaitRecoveryKeyframe, false)
        .action;
    assert_ne!(second, RecoveryAction::RequestReconnectCandidate);
    std::thread::sleep(Duration::from_millis(480));
    let third = controller
        .on_reason_with_policy(VideoEscalationReason::TransportAwaitRecoveryKeyframe, false)
        .action;
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
    controller.register_action_applied(RecoveryAction::RequestKeyframe);
    controller.register_action_applied(RecoveryAction::RequestKeyframe);
    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::TransportAwaitRecoveryKeyframe)
            .action,
        RecoveryAction::CooldownSuppressed
    );
    controller.begin_recovery_epoch(4);
    std::thread::sleep(Duration::from_millis(130));
    assert_eq!(
        controller
            .on_reason(VideoEscalationReason::TransportAwaitRecoveryKeyframe)
            .action,
        RecoveryAction::RequestKeyframe
    );
}

#[test]
fn action_contract_defines_owner_budget_and_epoch_rules() {
    let keyframe = VideoEscalationController::action_contract(RecoveryAction::RequestKeyframe);
    assert!(keyframe.budget_consumed_on_proposal);
    assert_eq!(keyframe.budget_kind, Some(RecoveryBudgetKind::Keyframe));
    assert!(!keyframe.advances_recovery_epoch_on_success);

    let reset = VideoEscalationController::action_contract(RecoveryAction::RequestDecoderReset);
    assert!(reset.budget_consumed_on_proposal);
    assert_eq!(reset.budget_kind, Some(RecoveryBudgetKind::DecoderReset));
    assert!(reset.advances_recovery_epoch_on_success);

    let reconnect =
        VideoEscalationController::action_contract(RecoveryAction::RequestReconnectCandidate);
    assert!(reconnect.budget_consumed_on_proposal);
    assert_eq!(reconnect.budget_kind, Some(RecoveryBudgetKind::Reconnect));
    assert!(reconnect.advances_recovery_epoch_on_success);

    let suppressed = VideoEscalationController::action_contract(RecoveryAction::CooldownSuppressed);
    assert!(!suppressed.budget_consumed_on_proposal);
    assert!(suppressed.budget_kind.is_none());
    assert!(!suppressed.advances_recovery_epoch_on_success);
}
