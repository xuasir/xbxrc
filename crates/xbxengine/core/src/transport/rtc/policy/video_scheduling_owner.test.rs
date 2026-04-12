use super::{
    RecoveryIntentSource, VideoHealthContract, VideoSchedulingOwner, VideoSchedulingOwnerInput,
    VideoSchedulingOwnerState,
};
use crate::transport::rtc::facts::ConnectionLifecycleStateFact;
use crate::transport::rtc::policy::display_supply::SchedulingDemandSignal;
use crate::transport::rtc::recovery::policy::DisplaySupplyThresholds;

fn thresholds() -> DisplaySupplyThresholds {
    DisplaySupplyThresholds {
        degraded_no_pending_streak: 48,
        critical_no_pending_streak: 96,
        degraded_present_age_ms: 180.0,
        degraded_decode_age_ms: 140.0,
        critical_present_age_ms: 600.0,
        critical_decode_age_ms: 320.0,
        degraded_present_drop_ratio: 0.03,
        critical_present_drop_ratio: 0.08,
        degraded_present_overwrite_ratio: 0.05,
        critical_present_overwrite_ratio: 0.12,
        degraded_pacer_drop_ratio: 0.02,
        critical_pacer_drop_ratio: 0.06,
        degraded_renderer_drop_ratio: 0.015,
        critical_renderer_drop_ratio: 0.05,
    }
}

fn home_thresholds() -> DisplaySupplyThresholds {
    DisplaySupplyThresholds {
        degraded_no_pending_streak: 80,
        critical_no_pending_streak: 150,
        degraded_present_age_ms: 240.0,
        degraded_decode_age_ms: 180.0,
        critical_present_age_ms: 720.0,
        critical_decode_age_ms: 420.0,
        degraded_present_drop_ratio: 0.04,
        critical_present_drop_ratio: 0.10,
        degraded_present_overwrite_ratio: 0.06,
        critical_present_overwrite_ratio: 0.14,
        degraded_pacer_drop_ratio: 0.03,
        critical_pacer_drop_ratio: 0.08,
        degraded_renderer_drop_ratio: 0.02,
        critical_renderer_drop_ratio: 0.06,
    }
}

fn input(
    connection_state: ConnectionLifecycleStateFact,
    anchor_reason_label: Option<&str>,
    demand: SchedulingDemandSignal,
    timeline_chain_state: Option<&str>,
    timeline_source_event: Option<&str>,
    track_state: Option<&str>,
    track_video_bytes_total: Option<u64>,
    observed_at_ms: f64,
    recovery_epoch: u64,
) -> VideoSchedulingOwnerInput {
    VideoSchedulingOwnerInput {
        connection_state,
        recovery_epoch,
        first_frame_acquisition_priority_allowed: true,
        anchor_reason_label: anchor_reason_label.map(str::to_string),
        demand,
        clean_anchor_epoch: None,
        clean_anchor_observed_at_ms: None,
        clean_anchor_source_event: None,
        latest_video_timeline_observation: None,
        latest_timeline_chain_state: timeline_chain_state.map(str::to_string),
        latest_timeline_source_event: timeline_source_event.map(str::to_string),
        latest_track_state: track_state.map(str::to_string),
        latest_track_video_bytes_total: track_video_bytes_total,
        latest_h264_bootstrap_ready: None,
        latest_h264_bootstrap_reject_reason: None,
        latest_h264_committed_sps_present: None,
        latest_h264_committed_pps_present: None,
        latest_h264_delta_continuation_ready: None,
        latest_h264_observed_at_ms: None,
        display_supply_thresholds: thresholds(),
        observed_at_ms,
        latest_anchor_candidate_ledger: None,
        latest_decode_candidate_detail: None,
        latest_decode_candidate_observed_at_ms: None,
    }
}

#[test]
fn anchor_broken_enters_rebuilding_supply() {
    let mut owner = VideoSchedulingOwner::new();
    let output = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        Some("transportAwaitRecoveryKeyframe"),
        SchedulingDemandSignal::default(),
        None,
        None,
        None,
        None,
        100.0,
        1,
    ));
    assert_eq!(output.state, VideoSchedulingOwnerState::RebuildingSupply);
    assert_eq!(output.health, VideoHealthContract::Recovering);
    let intent = output.recovery_intent.expect("recovery intent");
    assert_eq!(intent.source, RecoveryIntentSource::Anchor);
    assert_eq!(intent.reason_label, "transportAwaitRecoveryKeyframe");
    assert!(intent.emit);
}

#[test]
fn supply_starving_without_anchor_break_enters_supply_starved() {
    let mut owner = VideoSchedulingOwner::new();
    let output = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        None,
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("critical".to_string()),
            no_pending_streak: Some(160),
            present_age_ms: Some(1100.0),
            decode_age_ms: Some(600.0),
            video_renderer_stalled: true,
            ..SchedulingDemandSignal::default()
        },
        None,
        None,
        None,
        None,
        200.0,
        1,
    ));
    assert_eq!(output.state, VideoSchedulingOwnerState::SupplyStarved);
    assert_eq!(output.health, VideoHealthContract::Starved);
    let intent = output.recovery_intent.expect("recovery intent");
    assert_eq!(intent.source, RecoveryIntentSource::Supply);
    assert_eq!(intent.reason_label, "displaySupplyCritical");
    assert!(intent.emit);
}

#[test]
fn connected_soft_supply_spike_holds_in_degraded_serving_before_starved() {
    let mut owner = VideoSchedulingOwner::new();
    let stable = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        None,
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("normal".to_string()),
            no_pending_streak: Some(0),
            present_age_ms: Some(12.0),
            decode_age_ms: Some(10.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("healthy"),
        Some("frame-complete-candidate"),
        Some("remoteTrackAttached"),
        Some(64_000),
        100.0,
        1,
    ));
    assert_eq!(stable.state, VideoSchedulingOwnerState::Priming);

    let recovered = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        None,
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("normal".to_string()),
            no_pending_streak: Some(0),
            present_age_ms: Some(10.0),
            decode_age_ms: Some(8.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("healthy"),
        Some("frame-complete-candidate"),
        Some("remoteTrackAttached"),
        Some(64_000),
        140.0,
        1,
    ));
    assert_eq!(recovered.state, VideoSchedulingOwnerState::StableServing);

    let first_spike = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        None,
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("critical".to_string()),
            no_pending_streak: Some(220),
            present_age_ms: Some(1200.0),
            decode_age_ms: Some(18.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("healthy"),
        Some("frame-complete-candidate"),
        Some("remoteTrackAttached"),
        Some(80_000),
        180.0,
        1,
    ));
    assert_eq!(
        first_spike.state,
        VideoSchedulingOwnerState::DegradedServing
    );
    assert!(first_spike.recovery_intent.is_none());

    let sustained = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        None,
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("critical".to_string()),
            no_pending_streak: Some(260),
            present_age_ms: Some(1800.0),
            decode_age_ms: Some(20.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("healthy"),
        Some("frame-complete-candidate"),
        Some("remoteTrackAttached"),
        Some(80_000),
        380.0,
        1,
    ));
    assert_eq!(sustained.state, VideoSchedulingOwnerState::DegradedServing);
    assert!(sustained.recovery_intent.is_none());

    let starved = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        None,
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("critical".to_string()),
            no_pending_streak: Some(300),
            present_age_ms: Some(1_800.0),
            decode_age_ms: Some(20.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("healthy"),
        Some("frame-complete-candidate"),
        Some("remoteTrackAttached"),
        Some(80_000),
        520.0,
        1,
    ));
    assert_eq!(starved.state, VideoSchedulingOwnerState::SupplyStarved);
    let intent = starved.recovery_intent.expect("supply intent");
    assert_eq!(intent.reason_label, "displaySupplyCritical");
}

#[test]
fn anchor_cleared_and_supply_healthy_returns_to_stable_serving() {
    let mut owner = VideoSchedulingOwner::new();
    let _ = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        Some("transportAwaitRecoveryKeyframe"),
        SchedulingDemandSignal::default(),
        None,
        Some("frame-await-recovery-keyframe"),
        Some("remoteTrackAttached"),
        Some(10_000),
        300.0,
        1,
    ));
    let mut ready = input(
        ConnectionLifecycleStateFact::Connected,
        None,
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("normal".to_string()),
            no_pending_streak: Some(0),
            present_age_ms: Some(12.0),
            decode_age_ms: Some(9.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("healthy"),
        Some("frame-complete-candidate"),
        Some("remoteTrackAttached"),
        Some(20_000),
        520.0,
        1,
    );
    ready.clean_anchor_epoch = Some(1);
    ready.clean_anchor_observed_at_ms = Some(518.0);
    ready.clean_anchor_source_event = Some("chain-clean-keyframe-submitted".to_string());
    let output = owner.evaluate(&ready);
    assert_eq!(output.state, VideoSchedulingOwnerState::StableServing);
    assert_eq!(output.health, VideoHealthContract::Stable);
    assert!(output.recovery_intent.is_none());
}

#[test]
fn recent_non_idr_codec_evidence_keeps_owner_in_rebuilding_supply() {
    let mut owner = VideoSchedulingOwner::new();
    let _ = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        Some("transportAwaitRecoveryKeyframe"),
        SchedulingDemandSignal::default(),
        Some("recovering"),
        Some("frame-await-recovery-keyframe"),
        Some("remoteTrackAttached"),
        Some(10_000),
        300.0,
        1,
    ));

    let mut codec_blocked = input(
        ConnectionLifecycleStateFact::Connected,
        None,
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("normal".to_string()),
            no_pending_streak: Some(0),
            present_age_ms: Some(12.0),
            decode_age_ms: Some(9.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("healthy"),
        Some("frame-complete-candidate"),
        Some("remoteTrackAttached"),
        Some(64_000),
        340.0,
        1,
    );
    codec_blocked.clean_anchor_epoch = Some(1);
    codec_blocked.clean_anchor_observed_at_ms = Some(338.0);
    codec_blocked.clean_anchor_source_event = Some("chain-clean-keyframe-submitted".to_string());
    codec_blocked.latest_h264_bootstrap_ready = Some(false);
    codec_blocked.latest_h264_bootstrap_reject_reason = Some("NonIdrVcl".to_string());
    codec_blocked.latest_h264_observed_at_ms = Some(339.0);

    let blocked = owner.evaluate(&codec_blocked);
    assert_eq!(blocked.state, VideoSchedulingOwnerState::RebuildingSupply);
    assert_eq!(blocked.health, VideoHealthContract::Recovering);
    assert!(blocked
        .recovery_intent
        .as_ref()
        .is_some_and(|intent| intent.reason_label == "transportAwaitRecoveryKeyframe"));
}

#[test]
fn non_idr_with_committed_sets_and_delta_ready_does_not_block_recovery_exit() {
    let mut owner = VideoSchedulingOwner::new();
    let _ = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        Some("transportAwaitRecoveryKeyframe"),
        SchedulingDemandSignal::default(),
        Some("recovering"),
        Some("frame-await-recovery-keyframe"),
        Some("remoteTrackAttached"),
        Some(10_000),
        300.0,
        1,
    ));

    let mut recoverable = input(
        ConnectionLifecycleStateFact::Connected,
        None,
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("normal".to_string()),
            no_pending_streak: Some(0),
            present_age_ms: Some(12.0),
            decode_age_ms: Some(9.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("healthy"),
        Some("frame-complete-candidate"),
        Some("remoteTrackAttached"),
        Some(64_000),
        340.0,
        1,
    );
    recoverable.clean_anchor_epoch = Some(1);
    recoverable.clean_anchor_observed_at_ms = Some(338.0);
    recoverable.clean_anchor_source_event = Some("chain-clean-keyframe-submitted".to_string());
    recoverable.latest_h264_bootstrap_ready = Some(false);
    recoverable.latest_h264_bootstrap_reject_reason = Some("NonIdrVcl".to_string());
    recoverable.latest_h264_committed_sps_present = Some(true);
    recoverable.latest_h264_committed_pps_present = Some(true);
    recoverable.latest_h264_delta_continuation_ready = Some(true);
    recoverable.latest_h264_observed_at_ms = Some(339.0);

    let output = owner.evaluate(&recoverable);
    assert_eq!(output.state, VideoSchedulingOwnerState::StableServing);
    assert_eq!(output.health, VideoHealthContract::Stable);
}

#[test]
fn transient_anchor_noise_with_clean_anchor_and_delta_ready_does_not_stick_recovery() {
    let mut owner = VideoSchedulingOwner::new();
    let _ = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        Some("transportAwaitRecoveryKeyframe"),
        SchedulingDemandSignal::default(),
        Some("recovering"),
        Some("frame-await-recovery-keyframe"),
        Some("remoteTrackAttached"),
        Some(10_000),
        300.0,
        1,
    ));

    let mut recoverable = input(
        ConnectionLifecycleStateFact::Connected,
        Some("transportAwaitRecoveryKeyframe"),
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("normal".to_string()),
            no_pending_streak: Some(0),
            present_age_ms: Some(12.0),
            decode_age_ms: Some(9.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("repairing"),
        Some("nack-observation"),
        Some("remoteTrackAttached"),
        Some(64_000),
        340.0,
        1,
    );
    recoverable.clean_anchor_epoch = Some(1);
    recoverable.clean_anchor_observed_at_ms = Some(338.0);
    recoverable.clean_anchor_source_event = Some("chain-clean-keyframe-submitted".to_string());
    recoverable.latest_h264_bootstrap_ready = Some(false);
    recoverable.latest_h264_bootstrap_reject_reason = Some("NonIdrVcl".to_string());
    recoverable.latest_h264_committed_sps_present = Some(true);
    recoverable.latest_h264_committed_pps_present = Some(true);
    recoverable.latest_h264_delta_continuation_ready = Some(true);
    recoverable.latest_h264_observed_at_ms = Some(339.0);
    recoverable.latest_anchor_candidate_ledger = Some(crate::XbxEngineAnchorCandidateLedger {
        state: crate::XbxEngineAnchorCandidateState::Observed,
        source_event: "frame-complete-candidate".to_string(),
        frame_rtp_timestamp: Some(9_000),
        observed_at_ms: 339.0,
        recovery_epoch: 1,
        failure_reason: None,
    });

    let output = owner.evaluate(&recoverable);
    assert_eq!(output.state, VideoSchedulingOwnerState::StableServing);
    assert_eq!(output.health, VideoHealthContract::Stable);
}

#[test]
fn sustained_critical_pressure_without_clean_anchor_keeps_owner_in_rebuilding_supply() {
    let mut owner = VideoSchedulingOwner::new();
    let _ = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        Some("transportAwaitRecoveryKeyframe"),
        SchedulingDemandSignal::default(),
        Some("recovering"),
        Some("frame-await-recovery-keyframe"),
        Some("remoteTrackAttached"),
        Some(30_000),
        520.0,
        1,
    ));

    let ready = input(
        ConnectionLifecycleStateFact::Connected,
        None,
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("critical".to_string()),
            no_pending_streak: Some(180),
            present_age_ms: Some(12.0),
            decode_age_ms: Some(9.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("healthy"),
        Some("frame-complete-candidate"),
        Some("remoteTrackAttached"),
        Some(36_000),
        540.0,
        1,
    );
    let output = owner.evaluate(&ready);
    assert_eq!(output.state, VideoSchedulingOwnerState::RebuildingSupply);
    assert_eq!(output.health, VideoHealthContract::Recovering);
}

#[test]
fn connected_lingering_no_pending_with_clean_anchor_can_return_to_stable_serving() {
    let mut owner = VideoSchedulingOwner::new();
    let _ = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        Some("transportAwaitRecoveryKeyframe"),
        SchedulingDemandSignal::default(),
        Some("recovering"),
        Some("frame-await-recovery-keyframe"),
        Some("remoteTrackAttached"),
        Some(30_000),
        560.0,
        1,
    ));

    let mut ready = input(
        ConnectionLifecycleStateFact::Connected,
        None,
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("critical".to_string()),
            no_pending_streak: Some(220),
            present_age_ms: Some(14.0),
            decode_age_ms: Some(10.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("healthy"),
        Some("frame-observed"),
        Some("remoteTrackAttached"),
        Some(42_000),
        580.0,
        1,
    );
    ready.clean_anchor_epoch = Some(1);
    ready.clean_anchor_observed_at_ms = Some(579.0);
    ready.clean_anchor_source_event = Some("chain-clean-keyframe-submitted".to_string());

    let output = owner.evaluate(&ready);
    assert_eq!(output.state, VideoSchedulingOwnerState::StableServing);
    assert_eq!(output.health, VideoHealthContract::Stable);
    assert!(output.recovery_intent.is_none());
}

#[test]
fn first_present_feedback_lag_with_clean_anchor_exits_rebuilding_supply() {
    let mut owner = VideoSchedulingOwner::new();
    let _ = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        Some("transportAwaitRecoveryKeyframe"),
        SchedulingDemandSignal::default(),
        Some("recovering"),
        Some("frame-await-recovery-keyframe"),
        Some("remoteTrackAttached"),
        Some(24_000),
        620.0,
        1,
    ));

    let mut recoverable = input(
        ConnectionLifecycleStateFact::Connected,
        None,
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("critical".to_string()),
            no_pending_streak: Some(220),
            present_age_ms: None,
            decode_age_ms: Some(10.0),
            host_cadence_phase: Some("priming".to_string()),
            host_display_tick_epoch: Some(720),
            host_present_epoch: Some(0),
            present_submit_count_total: Some(0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("healthy"),
        Some("frame-complete-candidate"),
        Some("remoteTrackAttached"),
        Some(42_000),
        640.0,
        1,
    );
    recoverable.clean_anchor_epoch = Some(1);
    recoverable.clean_anchor_observed_at_ms = Some(639.0);
    recoverable.clean_anchor_source_event = Some("chain-clean-keyframe-submitted".to_string());

    let output = owner.evaluate(&recoverable);
    assert_eq!(output.state, VideoSchedulingOwnerState::DegradedServing);
    assert_eq!(output.health, VideoHealthContract::Stable);
}

#[test]
fn media_continuity_without_decode_or_present_feedback_can_exit_rebuilding_supply_as_degraded() {
    let mut owner = VideoSchedulingOwner::new();
    let _ = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        Some("transportAwaitRecoveryKeyframe"),
        SchedulingDemandSignal::default(),
        Some("recovering"),
        Some("frame-await-recovery-keyframe"),
        Some("remoteTrackAttached"),
        Some(24_000),
        700.0,
        8,
    ));

    let mut recoverable = input(
        ConnectionLifecycleStateFact::Connected,
        None,
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("critical".to_string()),
            no_pending_streak: Some(180),
            present_age_ms: None,
            decode_age_ms: None,
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("healthy"),
        Some("frame-complete-candidate"),
        Some("remoteTrackAttached"),
        Some(48_000),
        720.0,
        8,
    );
    recoverable.clean_anchor_epoch = Some(8);
    recoverable.clean_anchor_observed_at_ms = Some(719.0);
    recoverable.clean_anchor_source_event = Some("chain-clean-keyframe-submitted".to_string());
    recoverable.latest_h264_bootstrap_ready = Some(false);
    recoverable.latest_h264_bootstrap_reject_reason = Some("NonIdrVcl".to_string());
    recoverable.latest_h264_committed_sps_present = Some(true);
    recoverable.latest_h264_committed_pps_present = Some(true);
    recoverable.latest_h264_delta_continuation_ready = Some(true);
    recoverable.latest_h264_observed_at_ms = Some(719.0);

    let output = owner.evaluate(&recoverable);
    assert_eq!(output.state, VideoSchedulingOwnerState::DegradedServing);
    assert_eq!(output.health, VideoHealthContract::Stable);
    assert!(output.recovery_intent.is_none());
}

#[test]
fn missing_media_continuity_metadata_keeps_rebuilding_supply_without_decode_or_present_feedback() {
    let mut owner = VideoSchedulingOwner::new();
    let _ = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        Some("transportAwaitRecoveryKeyframe"),
        SchedulingDemandSignal::default(),
        Some("recovering"),
        Some("frame-await-recovery-keyframe"),
        Some("remoteTrackAttached"),
        Some(24_000),
        760.0,
        9,
    ));

    let mut blocked = input(
        ConnectionLifecycleStateFact::Connected,
        None,
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("critical".to_string()),
            no_pending_streak: Some(180),
            present_age_ms: None,
            decode_age_ms: None,
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("healthy"),
        Some("frame-complete-candidate"),
        Some("remoteTrackAttached"),
        Some(48_000),
        780.0,
        9,
    );
    blocked.clean_anchor_epoch = Some(9);
    blocked.clean_anchor_observed_at_ms = Some(779.0);
    blocked.clean_anchor_source_event = Some("chain-clean-keyframe-submitted".to_string());
    blocked.latest_h264_bootstrap_ready = Some(false);
    blocked.latest_h264_bootstrap_reject_reason = Some("NonIdrVcl".to_string());
    blocked.latest_h264_committed_sps_present = Some(true);
    blocked.latest_h264_committed_pps_present = Some(false);
    blocked.latest_h264_delta_continuation_ready = Some(true);
    blocked.latest_h264_observed_at_ms = Some(779.0);

    let output = owner.evaluate(&blocked);
    assert_eq!(output.state, VideoSchedulingOwnerState::RebuildingSupply);
    assert_eq!(output.health, VideoHealthContract::Recovering);
}

#[test]
fn soft_display_supply_critical_is_absorbed_before_reentering_supply_recovery() {
    let mut owner = VideoSchedulingOwner::new();
    let mut stable = input(
        ConnectionLifecycleStateFact::Connected,
        None,
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("normal".to_string()),
            no_pending_streak: Some(0),
            present_age_ms: Some(12.0),
            decode_age_ms: Some(9.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("healthy"),
        Some("frame-observed"),
        Some("remoteTrackAttached"),
        Some(42_000),
        1_000.0,
        1,
    );
    stable.clean_anchor_epoch = Some(1);
    stable.clean_anchor_observed_at_ms = Some(999.0);
    stable.clean_anchor_source_event = Some("chain-clean-keyframe-submitted".to_string());
    assert_eq!(
        owner.evaluate(&stable).state,
        VideoSchedulingOwnerState::Priming
    );

    let mut stable_again = stable.clone();
    stable_again.observed_at_ms = 1_020.0;
    stable_again.clean_anchor_observed_at_ms = Some(1_019.0);
    assert_eq!(
        owner.evaluate(&stable_again).state,
        VideoSchedulingOwnerState::StableServing
    );

    let mut burst = input(
        ConnectionLifecycleStateFact::Connected,
        None,
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("critical".to_string()),
            no_pending_streak: Some(120),
            present_age_ms: Some(980.0),
            decode_age_ms: Some(360.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("healthy"),
        Some("frame-observed"),
        Some("remoteTrackAttached"),
        Some(60_000),
        1_100.0,
        1,
    );
    burst.clean_anchor_epoch = Some(1);
    burst.clean_anchor_observed_at_ms = Some(1_095.0);
    burst.clean_anchor_source_event = Some("chain-clean-keyframe-submitted".to_string());
    let absorbed = owner.evaluate(&burst);
    assert_eq!(absorbed.state, VideoSchedulingOwnerState::DegradedServing);
    assert_eq!(absorbed.health, VideoHealthContract::Stable);
    assert!(absorbed.recovery_intent.is_none());

    let mut sustained = burst.clone();
    sustained.observed_at_ms = 1_360.0;
    sustained.clean_anchor_observed_at_ms = Some(1_355.0);
    let escalated = owner.evaluate(&sustained);
    assert_eq!(escalated.state, VideoSchedulingOwnerState::SupplyStarved);
    assert_eq!(escalated.health, VideoHealthContract::Starved);
    assert_eq!(
        escalated
            .recovery_intent
            .as_ref()
            .map(|intent| intent.reason_label.as_str()),
        Some("displaySupplyCritical")
    );
}

#[test]
fn transient_represent_recovery_gap_stays_degraded_until_present_feedback_catches_up() {
    let mut owner = VideoSchedulingOwner::new();
    let mut stable = input(
        ConnectionLifecycleStateFact::Connected,
        None,
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("normal".to_string()),
            no_pending_streak: Some(0),
            present_age_ms: Some(10.0),
            decode_age_ms: Some(8.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("healthy"),
        Some("frame-observed"),
        Some("remoteTrackAttached"),
        Some(48_000),
        2_000.0,
        4,
    );
    stable.clean_anchor_epoch = Some(4);
    stable.clean_anchor_observed_at_ms = Some(1_998.0);
    stable.clean_anchor_source_event = Some("chain-clean-keyframe-submitted".to_string());
    assert_eq!(
        owner.evaluate(&stable).state,
        VideoSchedulingOwnerState::Priming
    );

    let mut settled = stable.clone();
    settled.observed_at_ms = 2_020.0;
    settled.clean_anchor_observed_at_ms = Some(2_018.0);
    assert_eq!(
        owner.evaluate(&settled).state,
        VideoSchedulingOwnerState::StableServing
    );

    let mut burst = input(
        ConnectionLifecycleStateFact::Connected,
        None,
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("critical".to_string()),
            no_pending_streak: Some(132),
            present_age_ms: Some(814.0),
            decode_age_ms: Some(12.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("healthy"),
        Some("frame-complete-candidate"),
        Some("remoteTrackAttached"),
        Some(72_000),
        2_120.0,
        4,
    );
    burst.clean_anchor_epoch = Some(4);
    burst.clean_anchor_observed_at_ms = Some(2_118.0);
    burst.clean_anchor_source_event = Some("chain-clean-keyframe-submitted".to_string());
    let absorbed = owner.evaluate(&burst);
    assert_eq!(absorbed.state, VideoSchedulingOwnerState::DegradedServing);
    assert!(absorbed.recovery_intent.is_none());

    let mut feedback_gap = burst.clone();
    feedback_gap.observed_at_ms = 2_200.0;
    feedback_gap.clean_anchor_observed_at_ms = Some(2_198.0);
    feedback_gap.demand.no_pending_pressure_level = Some("high".to_string());
    feedback_gap.demand.no_pending_streak = Some(97);
    feedback_gap.demand.present_age_ms = Some(828.0);
    feedback_gap.demand.decode_age_ms = Some(9.0);
    feedback_gap.latest_timeline_source_event = Some("frame-complete-candidate".to_string());
    let held = owner.evaluate(&feedback_gap);
    assert_eq!(held.state, VideoSchedulingOwnerState::DegradedServing);
    assert!(held.recovery_intent.is_none());

    let mut recovered = feedback_gap.clone();
    recovered.observed_at_ms = 2_214.0;
    recovered.clean_anchor_observed_at_ms = Some(2_212.0);
    recovered.demand.no_pending_pressure_level = Some("normal".to_string());
    recovered.demand.no_pending_streak = Some(0);
    recovered.demand.present_age_ms = Some(15.0);
    recovered.demand.decode_age_ms = Some(6.0);
    let output = owner.evaluate(&recovered);
    assert_eq!(output.state, VideoSchedulingOwnerState::StableServing);
    assert!(output.recovery_intent.is_none());
}

#[test]
fn supply_starved_dwell_resets_when_supply_reason_label_changes() {
    let mut owner = VideoSchedulingOwner::new();
    let mut stable = input(
        ConnectionLifecycleStateFact::Connected,
        None,
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("normal".to_string()),
            no_pending_streak: Some(0),
            present_age_ms: Some(12.0),
            decode_age_ms: Some(9.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("healthy"),
        Some("frame-complete-candidate"),
        Some("remoteTrackAttached"),
        Some(64_000),
        2_000.0,
        1,
    );
    stable.clean_anchor_epoch = Some(1);
    stable.clean_anchor_observed_at_ms = Some(1_999.0);
    stable.clean_anchor_source_event = Some("chain-clean-keyframe-submitted".to_string());
    assert_eq!(
        owner.evaluate(&stable).state,
        VideoSchedulingOwnerState::Priming
    );

    let mut stable_again = stable.clone();
    stable_again.observed_at_ms = 2_020.0;
    stable_again.clean_anchor_observed_at_ms = Some(2_019.0);
    assert_eq!(
        owner.evaluate(&stable_again).state,
        VideoSchedulingOwnerState::StableServing
    );

    let mut degraded_pressure = stable.clone();
    degraded_pressure.observed_at_ms = 2_100.0;
    degraded_pressure.clean_anchor_observed_at_ms = Some(2_099.0);
    degraded_pressure.demand = SchedulingDemandSignal {
        no_pending_pressure_level: Some("high".to_string()),
        no_pending_streak: Some(96),
        present_age_ms: Some(220.0),
        decode_age_ms: Some(40.0),
        video_renderer_stalled: false,
        ..SchedulingDemandSignal::default()
    };
    let first = owner.evaluate(&degraded_pressure);
    assert_eq!(first.state, VideoSchedulingOwnerState::DegradedServing);
    assert!(first.recovery_intent.is_none());

    let mut critical_pressure = degraded_pressure.clone();
    critical_pressure.observed_at_ms = 2_220.0;
    critical_pressure.clean_anchor_observed_at_ms = Some(2_219.0);
    critical_pressure.demand = SchedulingDemandSignal {
        no_pending_pressure_level: Some("critical".to_string()),
        no_pending_streak: Some(220),
        present_age_ms: Some(1_050.0),
        decode_age_ms: Some(60.0),
        video_renderer_stalled: false,
        ..SchedulingDemandSignal::default()
    };
    let switched = owner.evaluate(&critical_pressure);
    assert_eq!(switched.state, VideoSchedulingOwnerState::DegradedServing);
    assert!(switched.recovery_intent.is_none());

    let mut after_switch_window = critical_pressure.clone();
    after_switch_window.observed_at_ms = 2_360.0;
    after_switch_window.clean_anchor_observed_at_ms = Some(2_359.0);
    let still_held = owner.evaluate(&after_switch_window);
    assert_eq!(still_held.state, VideoSchedulingOwnerState::DegradedServing);
    assert!(still_held.recovery_intent.is_none());

    let mut confirmed = critical_pressure.clone();
    // 须超过 DISPLAY_SUPPLY_STARVED_CONFIRM_MS（与 owner 内常量对齐）
    confirmed.observed_at_ms = 2_520.0;
    confirmed.clean_anchor_observed_at_ms = Some(2_519.0);
    let starved = owner.evaluate(&confirmed);
    assert_eq!(starved.state, VideoSchedulingOwnerState::SupplyStarved);
    assert_eq!(
        starved
            .recovery_intent
            .as_ref()
            .map(|intent| intent.reason_label.as_str()),
        Some("displaySupplyCritical")
    );
}

#[test]
fn supply_starved_dwell_clears_after_brief_recovery_before_restarting() {
    let mut owner = VideoSchedulingOwner::new();
    let mut stable = input(
        ConnectionLifecycleStateFact::Connected,
        None,
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("normal".to_string()),
            no_pending_streak: Some(0),
            present_age_ms: Some(12.0),
            decode_age_ms: Some(9.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("healthy"),
        Some("frame-complete-candidate"),
        Some("remoteTrackAttached"),
        Some(64_000),
        3_000.0,
        1,
    );
    stable.clean_anchor_epoch = Some(1);
    stable.clean_anchor_observed_at_ms = Some(2_999.0);
    stable.clean_anchor_source_event = Some("chain-clean-keyframe-submitted".to_string());
    assert_eq!(
        owner.evaluate(&stable).state,
        VideoSchedulingOwnerState::Priming
    );

    let mut stable_again = stable.clone();
    stable_again.observed_at_ms = 3_020.0;
    stable_again.clean_anchor_observed_at_ms = Some(3_019.0);
    assert_eq!(
        owner.evaluate(&stable_again).state,
        VideoSchedulingOwnerState::StableServing
    );

    let mut critical = stable.clone();
    critical.observed_at_ms = 3_100.0;
    critical.clean_anchor_observed_at_ms = Some(3_099.0);
    critical.demand = SchedulingDemandSignal {
        no_pending_pressure_level: Some("critical".to_string()),
        no_pending_streak: Some(220),
        present_age_ms: Some(1_050.0),
        decode_age_ms: Some(40.0),
        video_renderer_stalled: false,
        ..SchedulingDemandSignal::default()
    };
    let first = owner.evaluate(&critical);
    assert_eq!(first.state, VideoSchedulingOwnerState::DegradedServing);

    let mut recovered = stable.clone();
    recovered.observed_at_ms = 3_160.0;
    recovered.clean_anchor_observed_at_ms = Some(3_159.0);
    let back_to_stable = owner.evaluate(&recovered);
    assert_eq!(
        back_to_stable.state,
        VideoSchedulingOwnerState::StableServing
    );
    assert!(back_to_stable.recovery_intent.is_none());

    let mut critical_again = critical.clone();
    critical_again.observed_at_ms = 3_220.0;
    critical_again.clean_anchor_observed_at_ms = Some(3_219.0);
    let restarted_hold = owner.evaluate(&critical_again);
    assert_eq!(
        restarted_hold.state,
        VideoSchedulingOwnerState::DegradedServing
    );
    assert!(restarted_hold.recovery_intent.is_none());

    let mut before_confirm = critical_again.clone();
    before_confirm.observed_at_ms = 3_360.0;
    before_confirm.clean_anchor_observed_at_ms = Some(3_359.0);
    let still_held = owner.evaluate(&before_confirm);
    assert_eq!(still_held.state, VideoSchedulingOwnerState::DegradedServing);
    assert!(still_held.recovery_intent.is_none());

    let mut confirmed = critical_again.clone();
    confirmed.observed_at_ms = 3_520.0;
    confirmed.clean_anchor_observed_at_ms = Some(3_519.0);
    let starved = owner.evaluate(&confirmed);
    assert_eq!(starved.state, VideoSchedulingOwnerState::SupplyStarved);
    assert_eq!(
        starved
            .recovery_intent
            .as_ref()
            .map(|intent| intent.reason_label.as_str()),
        Some("displaySupplyCritical")
    );
}

#[test]
fn clean_anchor_and_fresh_supply_can_exit_rebuilding_even_with_recovery_noise() {
    let mut owner = VideoSchedulingOwner::new();
    let _ = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        Some("transportAwaitRecoveryKeyframe"),
        SchedulingDemandSignal::default(),
        Some("recovering"),
        Some("frame-await-recovery-keyframe"),
        Some("remoteTrackAttached"),
        Some(30_000),
        520.0,
        1,
    ));

    let mut ready = input(
        ConnectionLifecycleStateFact::Connected,
        None,
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("high".to_string()),
            no_pending_streak: Some(120),
            present_age_ms: Some(16.0),
            decode_age_ms: Some(12.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("healthy"),
        Some("frame-await-recovery-keyframe"),
        Some("remoteTrackAttached"),
        Some(36_000),
        540.0,
        1,
    );
    ready.clean_anchor_epoch = Some(1);
    ready.clean_anchor_observed_at_ms = Some(538.0);
    ready.clean_anchor_source_event = Some("chain-clean-keyframe-submitted".to_string());
    ready.display_supply_thresholds = home_thresholds();
    let output = owner.evaluate(&ready);
    assert_eq!(output.state, VideoSchedulingOwnerState::StableServing);
    assert_eq!(output.health, VideoHealthContract::Stable);
}

#[test]
fn disconnected_or_recovering_lifecycle_constraints_owner_state() {
    let mut owner = VideoSchedulingOwner::new();
    let connected = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        None,
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("normal".to_string()),
            no_pending_streak: Some(0),
            present_age_ms: Some(16.0),
            decode_age_ms: Some(8.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("healthy"),
        Some("frame-observed"),
        Some("remoteTrackAttached"),
        Some(10_000),
        620.0,
        1,
    ));
    assert_eq!(connected.state, VideoSchedulingOwnerState::Priming);
    let disconnected = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Closed,
        Some("transportAwaitRecoveryKeyframe"),
        SchedulingDemandSignal::default(),
        None,
        None,
        None,
        None,
        700.0,
        1,
    ));
    assert_eq!(disconnected.state, VideoSchedulingOwnerState::SeekingAnchor);
    assert_eq!(disconnected.health, VideoHealthContract::Startup);
    assert!(disconnected.recovery_intent.is_none());
}

#[test]
fn epoch_change_reopens_same_intent_even_within_suppression_window() {
    let mut owner = VideoSchedulingOwner::new();
    let first = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        None,
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("critical".to_string()),
            no_pending_streak: Some(220),
            present_age_ms: Some(1200.0),
            decode_age_ms: Some(640.0),
            video_renderer_stalled: true,
            ..SchedulingDemandSignal::default()
        },
        None,
        None,
        Some("audioOnly"),
        Some(0),
        800.0,
        7,
    ));
    assert!(first
        .recovery_intent
        .as_ref()
        .is_some_and(|intent| intent.emit));

    let suppressed = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        None,
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("critical".to_string()),
            no_pending_streak: Some(220),
            present_age_ms: Some(1200.0),
            decode_age_ms: Some(640.0),
            video_renderer_stalled: true,
            ..SchedulingDemandSignal::default()
        },
        None,
        None,
        Some("audioOnly"),
        Some(0),
        801.0,
        7,
    ));
    assert!(suppressed
        .recovery_intent
        .as_ref()
        .is_some_and(|intent| !intent.emit));

    let reopened = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        None,
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("critical".to_string()),
            no_pending_streak: Some(220),
            present_age_ms: Some(1200.0),
            decode_age_ms: Some(640.0),
            video_renderer_stalled: true,
            ..SchedulingDemandSignal::default()
        },
        None,
        None,
        Some("audioOnly"),
        Some(0),
        802.0,
        8,
    ));
    assert!(reopened
        .recovery_intent
        .as_ref()
        .is_some_and(|intent| intent.emit));
}

#[test]
fn critical_no_pending_without_real_present_cannot_enter_stable_serving() {
    let mut owner = VideoSchedulingOwner::new();
    let first = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        None,
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("critical".to_string()),
            no_pending_streak: Some(980),
            present_age_ms: None,
            decode_age_ms: None,
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        None,
        None,
        Some("audioOnly"),
        Some(0),
        900.0,
        1,
    ));
    assert_ne!(first.state, VideoSchedulingOwnerState::StableServing);
    assert_eq!(first.state, VideoSchedulingOwnerState::SupplyStarved);
}

#[test]
fn priming_without_first_present_stays_in_priming_during_host_grace_window() {
    let mut owner = VideoSchedulingOwner::new();
    let first = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        None,
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("critical".to_string()),
            no_pending_streak: Some(6),
            present_age_ms: None,
            decode_age_ms: None,
            video_renderer_stalled: false,
            host_display_tick_epoch: Some(6),
            host_present_epoch: Some(0),
            host_cadence_phase: Some("priming".to_string()),
            present_submit_count_total: Some(0),
            ..SchedulingDemandSignal::default()
        },
        Some("healthy"),
        Some("frame-observed"),
        Some("remoteTrackAttached"),
        Some(0),
        900.0,
        1,
    ));
    assert_eq!(first.state, VideoSchedulingOwnerState::Priming);
    assert_eq!(first.health, VideoHealthContract::Startup);
}

#[test]
fn priming_without_first_present_stays_in_priming_until_first_present_arrives() {
    let mut owner = VideoSchedulingOwner::new();
    let _ = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        None,
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("critical".to_string()),
            no_pending_streak: Some(6),
            present_age_ms: None,
            decode_age_ms: None,
            video_renderer_stalled: false,
            host_display_tick_epoch: Some(6),
            host_present_epoch: Some(0),
            host_cadence_phase: Some("priming".to_string()),
            present_submit_count_total: Some(0),
            ..SchedulingDemandSignal::default()
        },
        Some("healthy"),
        Some("frame-observed"),
        Some("remoteTrackAttached"),
        Some(0),
        900.0,
        1,
    ));
    let priming = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        None,
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("critical".to_string()),
            no_pending_streak: Some(180),
            present_age_ms: None,
            decode_age_ms: None,
            video_renderer_stalled: false,
            host_display_tick_epoch: Some(220),
            host_present_epoch: Some(0),
            host_cadence_phase: Some("priming".to_string()),
            present_submit_count_total: Some(0),
            ..SchedulingDemandSignal::default()
        },
        Some("healthy"),
        Some("frame-observed"),
        Some("remoteTrackAttached"),
        Some(0),
        1_200.0,
        1,
    ));
    assert_eq!(priming.state, VideoSchedulingOwnerState::Priming);
    assert_eq!(priming.health, VideoHealthContract::Startup);
}

#[test]
fn startup_bootstrap_missing_sps_stays_priming_and_does_not_emit_recovery_intent() {
    let mut owner = VideoSchedulingOwner::new();
    let mut bootstrap_pending = input(
        ConnectionLifecycleStateFact::Connected,
        Some("transportAwaitRecoveryKeyframe"),
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("critical".to_string()),
            no_pending_streak: Some(180),
            present_age_ms: None,
            decode_age_ms: None,
            video_renderer_stalled: false,
            host_display_tick_epoch: Some(120),
            host_present_epoch: Some(0),
            host_cadence_phase: Some("priming".to_string()),
            present_submit_count_total: Some(0),
            ..SchedulingDemandSignal::default()
        },
        Some("recovering"),
        Some("frame-inspection-rejected-await-keyframe"),
        Some("remoteTrackAttached"),
        Some(20_000),
        1_100.0,
        1,
    );
    bootstrap_pending.latest_h264_bootstrap_ready = Some(false);
    bootstrap_pending.latest_h264_bootstrap_reject_reason = Some("bootstrapMissingSps".to_string());
    bootstrap_pending.latest_h264_observed_at_ms = Some(1_099.0);

    let output = owner.evaluate(&bootstrap_pending);
    assert_eq!(output.state, VideoSchedulingOwnerState::Priming);
    assert_eq!(output.health, VideoHealthContract::Startup);
    assert!(output.recovery_intent.is_none());
}

#[test]
fn startup_bootstrap_non_idr_stays_priming_and_does_not_emit_recovery_intent() {
    let mut owner = VideoSchedulingOwner::new();
    let mut bootstrap_pending = input(
        ConnectionLifecycleStateFact::Connected,
        Some("transportAwaitRecoveryKeyframe"),
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("critical".to_string()),
            no_pending_streak: Some(180),
            present_age_ms: None,
            decode_age_ms: None,
            video_renderer_stalled: false,
            host_display_tick_epoch: Some(120),
            host_present_epoch: Some(0),
            host_cadence_phase: Some("priming".to_string()),
            present_submit_count_total: Some(0),
            ..SchedulingDemandSignal::default()
        },
        Some("recovering"),
        Some("frame-await-recovery-keyframe"),
        Some("remoteTrackAttached"),
        Some(20_000),
        1_100.0,
        1,
    );
    bootstrap_pending.latest_h264_bootstrap_ready = Some(false);
    bootstrap_pending.latest_h264_bootstrap_reject_reason = Some("NonIdrVcl".to_string());
    bootstrap_pending.latest_h264_committed_sps_present = Some(true);
    bootstrap_pending.latest_h264_committed_pps_present = Some(true);
    bootstrap_pending.latest_h264_delta_continuation_ready = Some(false);
    bootstrap_pending.latest_h264_observed_at_ms = Some(1_099.0);

    let output = owner.evaluate(&bootstrap_pending);
    assert_eq!(output.state, VideoSchedulingOwnerState::Priming);
    assert_eq!(output.health, VideoHealthContract::Startup);
    assert!(output.recovery_intent.is_none());
}

#[test]
fn startup_bootstrap_pending_with_gap_repair_in_flight_stays_priming_without_recovery_intent() {
    let mut owner = VideoSchedulingOwner::new();
    let mut bootstrap_pending = input(
        ConnectionLifecycleStateFact::Connected,
        Some("transportAwaitRecoveryKeyframe"),
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("critical".to_string()),
            no_pending_streak: Some(180),
            present_age_ms: None,
            decode_age_ms: None,
            video_renderer_stalled: false,
            host_display_tick_epoch: Some(120),
            host_present_epoch: Some(0),
            host_cadence_phase: Some("priming".to_string()),
            present_submit_count_total: Some(0),
            ..SchedulingDemandSignal::default()
        },
        Some("recovering"),
        Some("gap-repair-in-flight"),
        Some("remoteTrackAttached"),
        Some(20_000),
        1_100.0,
        1,
    );
    bootstrap_pending.latest_h264_bootstrap_ready = Some(false);
    bootstrap_pending.latest_h264_bootstrap_reject_reason = Some("NonIdrVcl".to_string());
    bootstrap_pending.latest_h264_committed_sps_present = Some(true);
    bootstrap_pending.latest_h264_committed_pps_present = Some(true);
    bootstrap_pending.latest_h264_delta_continuation_ready = Some(true);
    bootstrap_pending.latest_h264_observed_at_ms = Some(1_099.0);

    let output = owner.evaluate(&bootstrap_pending);
    assert_eq!(output.state, VideoSchedulingOwnerState::Priming);
    assert_eq!(output.health, VideoHealthContract::Startup);
    assert!(output.recovery_intent.is_none());
}

#[test]
fn first_transport_await_probe_with_clean_anchor_stays_out_of_rebuilding_supply() {
    let mut owner = VideoSchedulingOwner::new();
    let mut priming = input(
        ConnectionLifecycleStateFact::Connected,
        None,
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("normal".to_string()),
            no_pending_streak: Some(0),
            present_age_ms: Some(12.0),
            decode_age_ms: Some(10.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("healthy"),
        Some("frame-complete-candidate"),
        Some("remoteTrackAttached"),
        Some(120_000),
        1_000.0,
        5,
    );
    priming.clean_anchor_epoch = Some(5);
    priming.clean_anchor_observed_at_ms = Some(999.0);
    priming.clean_anchor_source_event = Some("chain-clean-keyframe-submitted".to_string());
    assert_eq!(
        owner.evaluate(&priming).state,
        VideoSchedulingOwnerState::Priming
    );

    let stable = owner.evaluate(&priming);
    assert_eq!(stable.state, VideoSchedulingOwnerState::StableServing);

    let mut weak_probe = input(
        ConnectionLifecycleStateFact::Connected,
        Some("transportAwaitRecoveryKeyframe"),
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("critical".to_string()),
            no_pending_streak: Some(220),
            present_age_ms: Some(18.0),
            decode_age_ms: Some(14.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("healthy"),
        Some("frame-await-recovery-keyframe"),
        Some("remoteTrackAttached"),
        Some(128_000),
        1_060.0,
        5,
    );
    weak_probe.clean_anchor_epoch = Some(5);
    weak_probe.clean_anchor_observed_at_ms = Some(1_058.0);
    weak_probe.clean_anchor_source_event = Some("chain-clean-keyframe-submitted".to_string());

    let output = owner.evaluate(&weak_probe);
    assert_eq!(output.state, VideoSchedulingOwnerState::StableServing);
    assert_eq!(output.health, VideoHealthContract::Stable);
    assert!(output.recovery_intent.is_none());
}

#[test]
fn transport_await_with_rejected_anchor_candidate_enters_rebuilding_supply() {
    let mut owner = VideoSchedulingOwner::new();
    let mut hard_probe = input(
        ConnectionLifecycleStateFact::Connected,
        Some("transportAwaitRecoveryKeyframe"),
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("critical".to_string()),
            no_pending_streak: Some(220),
            present_age_ms: Some(420.0),
            decode_age_ms: Some(380.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("recovering"),
        Some("frame-await-recovery-keyframe"),
        Some("remoteTrackAttached"),
        Some(120_000),
        1_400.0,
        7,
    );
    hard_probe.latest_anchor_candidate_ledger = Some(crate::XbxEngineAnchorCandidateLedger {
        state: crate::XbxEngineAnchorCandidateState::Rejected,
        source_event: "frame-await-recovery-keyframe".to_string(),
        frame_rtp_timestamp: Some(7_001),
        recovery_epoch: 7,
        failure_reason: Some(
            crate::XbxEngineAnchorCandidateFailureReason::AwaitingRecoveryKeyframe,
        ),
        observed_at_ms: 1_399.0,
    });

    let output = owner.evaluate(&hard_probe);
    assert_eq!(output.state, VideoSchedulingOwnerState::RebuildingSupply);
    assert_eq!(output.health, VideoHealthContract::Recovering);
    let intent = output.recovery_intent.expect("anchor intent");
    assert_eq!(intent.source, RecoveryIntentSource::Anchor);
    assert_eq!(intent.reason_label, "transportAwaitRecoveryKeyframe");
}

#[test]
fn startup_bootstrap_reject_source_without_persisted_h264_state_stays_priming() {
    let mut owner = VideoSchedulingOwner::new();
    let bootstrap_pending = input(
        ConnectionLifecycleStateFact::Connected,
        Some("transportAwaitRecoveryKeyframe"),
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("critical".to_string()),
            no_pending_streak: Some(180),
            present_age_ms: None,
            decode_age_ms: None,
            video_renderer_stalled: false,
            host_display_tick_epoch: Some(120),
            host_present_epoch: Some(0),
            host_cadence_phase: Some("priming".to_string()),
            present_submit_count_total: Some(0),
            ..SchedulingDemandSignal::default()
        },
        Some("recovering"),
        Some("frame-inspection-rejected-await-keyframe"),
        Some("remoteTrackAttached"),
        Some(69_029),
        1_100.0,
        1,
    );

    let output = owner.evaluate(&bootstrap_pending);
    assert_eq!(output.state, VideoSchedulingOwnerState::Priming);
    assert_eq!(output.health, VideoHealthContract::Startup);
    assert!(output.recovery_intent.is_none());
}

#[test]
fn post_first_present_bootstrap_missing_sps_still_enters_rebuilding_supply() {
    let mut owner = VideoSchedulingOwner::new();
    let mut post_first_present = input(
        ConnectionLifecycleStateFact::Connected,
        Some("transportAwaitRecoveryKeyframe"),
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("critical".to_string()),
            no_pending_streak: Some(180),
            present_age_ms: Some(420.0),
            decode_age_ms: Some(380.0),
            video_renderer_stalled: false,
            host_display_tick_epoch: Some(360),
            host_present_epoch: Some(8),
            host_cadence_phase: Some("steady".to_string()),
            present_submit_count_total: Some(64),
            ..SchedulingDemandSignal::default()
        },
        Some("recovering"),
        Some("frame-inspection-rejected-await-keyframe"),
        Some("remoteTrackAttached"),
        Some(120_000),
        2_200.0,
        3,
    );
    post_first_present.latest_h264_bootstrap_ready = Some(false);
    post_first_present.latest_h264_bootstrap_reject_reason =
        Some("bootstrapMissingSps".to_string());
    post_first_present.latest_h264_observed_at_ms = Some(2_199.0);

    let output = owner.evaluate(&post_first_present);
    assert_eq!(output.state, VideoSchedulingOwnerState::RebuildingSupply);
    assert_eq!(output.health, VideoHealthContract::Recovering);
    assert!(output.recovery_intent.is_some());
}

#[test]
fn healthy_candidate_without_supply_recovery_keeps_owner_rebuilding_supply() {
    let mut owner = VideoSchedulingOwner::new();
    let _ = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        Some("transportAwaitRecoveryKeyframe"),
        SchedulingDemandSignal::default(),
        Some("recovering"),
        Some("frame-await-recovery-keyframe"),
        Some("remoteTrackAttached"),
        Some(30_000),
        1000.0,
        1,
    ));
    let output = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        None,
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("normal".to_string()),
            no_pending_streak: Some(12),
            present_age_ms: Some(220.0),
            decode_age_ms: Some(180.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("healthy"),
        Some("frame-complete-candidate"),
        Some("remoteTrackAttached"),
        Some(80_000),
        1015.0,
        1,
    ));
    assert_eq!(output.state, VideoSchedulingOwnerState::RebuildingSupply);
    assert_eq!(output.health, VideoHealthContract::Recovering);
}

#[test]
fn rebuilding_supply_can_exit_without_clean_anchor_when_supply_is_fresh_and_connected() {
    let mut owner = VideoSchedulingOwner::new();
    let _ = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        Some("transportAwaitRecoveryKeyframe"),
        SchedulingDemandSignal::default(),
        Some("recovering"),
        Some("frame-await-recovery-keyframe"),
        Some("remoteTrackAttached"),
        Some(30_000),
        520.0,
        1,
    ));

    let mut ready = input(
        ConnectionLifecycleStateFact::Connected,
        None,
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("normal".to_string()),
            no_pending_streak: Some(0),
            present_age_ms: Some(15.0),
            decode_age_ms: Some(17.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("healthy"),
        Some("frame-complete-candidate"),
        Some("remoteTrackAttached"),
        Some(42_000),
        540.0,
        1,
    );
    ready.display_supply_thresholds = home_thresholds();

    let output = owner.evaluate(&ready);
    assert_eq!(output.state, VideoSchedulingOwnerState::StableServing);
    assert_eq!(output.health, VideoHealthContract::Stable);
    assert!(output.recovery_intent.is_none());
}

#[test]
fn supply_starved_can_exit_with_fresh_connected_supply_even_without_clean_anchor() {
    let mut owner = VideoSchedulingOwner::new();
    let first = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        None,
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("critical".to_string()),
            no_pending_streak: Some(140),
            present_age_ms: Some(1_100.0),
            decode_age_ms: Some(640.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("healthy"),
        Some("frame-complete-candidate"),
        Some("remoteTrackAttached"),
        Some(96_000),
        1_000.0,
        4,
    ));
    assert_eq!(first.state, VideoSchedulingOwnerState::SupplyStarved);

    let ready = input(
        ConnectionLifecycleStateFact::Connected,
        None,
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("normal".to_string()),
            no_pending_streak: Some(2),
            present_age_ms: Some(16.0),
            decode_age_ms: Some(12.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("healthy"),
        Some("frame-observed"),
        Some("remoteTrackAttached"),
        Some(120_000),
        1_020.0,
        4,
    );
    let output = owner.evaluate(&ready);
    assert_eq!(output.state, VideoSchedulingOwnerState::StableServing);
    assert_eq!(output.health, VideoHealthContract::Stable);
    assert!(output.recovery_intent.is_none());
}

#[test]
fn owner_exits_recovering_only_after_strong_recovery_evidence() {
    let mut owner = VideoSchedulingOwner::new();
    let _ = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        Some("transportAwaitRecoveryKeyframe"),
        SchedulingDemandSignal::default(),
        Some("recovering"),
        Some("frame-await-recovery-keyframe"),
        Some("remoteTrackAttached"),
        Some(60_000),
        1200.0,
        1,
    ));
    let still_recovering = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        None,
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("normal".to_string()),
            no_pending_streak: Some(0),
            present_age_ms: Some(260.0),
            decode_age_ms: Some(210.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("healthy"),
        Some("frame-complete-candidate"),
        Some("remoteTrackAttached"),
        Some(90_000),
        1210.0,
        1,
    ));
    assert_eq!(
        still_recovering.state,
        VideoSchedulingOwnerState::RebuildingSupply
    );

    let mut ready = input(
        ConnectionLifecycleStateFact::Connected,
        None,
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("normal".to_string()),
            no_pending_streak: Some(0),
            present_age_ms: Some(20.0),
            decode_age_ms: Some(16.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("healthy"),
        Some("frame-observed"),
        Some("remoteTrackAttached"),
        Some(120_000),
        1230.0,
        1,
    );
    ready.clean_anchor_epoch = Some(1);
    ready.clean_anchor_observed_at_ms = Some(1225.0);
    ready.clean_anchor_source_event = Some("chain-clean-keyframe-submitted".to_string());
    let stable = owner.evaluate(&ready);
    assert_eq!(stable.state, VideoSchedulingOwnerState::StableServing);
    assert_eq!(stable.health, VideoHealthContract::Stable);
}

#[test]
fn rebuilding_supply_cannot_close_to_stable_without_explicit_healthy_chain() {
    let mut owner = VideoSchedulingOwner::new();
    let _ = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        Some("transportAwaitRecoveryKeyframe"),
        SchedulingDemandSignal::default(),
        Some("recovering"),
        Some("frame-await-recovery-keyframe"),
        Some("remoteTrackAttached"),
        Some(60_000),
        1_200.0,
        1,
    ));
    let mut ready = input(
        ConnectionLifecycleStateFact::Connected,
        None,
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("normal".to_string()),
            no_pending_streak: Some(0),
            present_age_ms: Some(20.0),
            decode_age_ms: Some(16.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        None,
        Some("frame-observed"),
        Some("remoteTrackAttached"),
        Some(120_000),
        1_230.0,
        1,
    );
    ready.clean_anchor_epoch = Some(1);
    ready.clean_anchor_observed_at_ms = Some(1_225.0);
    ready.clean_anchor_source_event = Some("chain-clean-keyframe-submitted".to_string());
    let not_ready = owner.evaluate(&ready);
    assert_eq!(not_ready.state, VideoSchedulingOwnerState::RebuildingSupply);
    assert_eq!(not_ready.health, VideoHealthContract::Recovering);
    assert!(not_ready.diagnostics.temporary_diagnostic_summary.is_some());
}

#[test]
fn rebuilding_supply_cannot_exit_with_stale_clean_anchor_fact_outside_grace_window() {
    let mut owner = VideoSchedulingOwner::new();
    let _ = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        Some("transportAwaitRecoveryKeyframe"),
        SchedulingDemandSignal::default(),
        Some("recovering"),
        Some("frame-await-recovery-keyframe"),
        Some("remoteTrackAttached"),
        Some(10_000),
        1300.0,
        3,
    ));

    let mut stale_anchor = input(
        ConnectionLifecycleStateFact::Connected,
        None,
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("normal".to_string()),
            no_pending_streak: Some(0),
            present_age_ms: Some(16.0),
            decode_age_ms: Some(12.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("healthy"),
        Some("frame-observed"),
        Some("remoteTrackAttached"),
        Some(20_000),
        1310.0,
        3,
    );
    stale_anchor.clean_anchor_epoch = Some(2);
    stale_anchor.clean_anchor_observed_at_ms = Some(300.0);
    stale_anchor.clean_anchor_source_event = Some("chain-clean-keyframe-submitted".to_string());
    stale_anchor.observed_at_ms = 2_300.0;
    let not_ready = owner.evaluate(&stale_anchor);
    assert_eq!(not_ready.state, VideoSchedulingOwnerState::RebuildingSupply);
}

#[test]
fn critical_wait_keyframe_noise_prefers_rebuilding_over_supply_starved_even_with_clean_anchor() {
    let mut owner = VideoSchedulingOwner::new();
    let mut stable = input(
        ConnectionLifecycleStateFact::Connected,
        None,
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("normal".to_string()),
            no_pending_streak: Some(0),
            present_age_ms: Some(18.0),
            decode_age_ms: Some(12.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("healthy"),
        Some("frame-observed"),
        Some("remoteTrackAttached"),
        Some(90_000),
        3_000.0,
        5,
    );
    stable.clean_anchor_epoch = Some(5);
    stable.clean_anchor_observed_at_ms = Some(2_999.0);
    stable.clean_anchor_source_event = Some("chain-clean-keyframe-submitted".to_string());
    let stable_output = owner.evaluate(&stable);
    assert_eq!(stable_output.state, VideoSchedulingOwnerState::Priming);

    let mut noisy = input(
        ConnectionLifecycleStateFact::Connected,
        None,
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("critical".to_string()),
            no_pending_streak: Some(220),
            present_age_ms: Some(22.0),
            decode_age_ms: Some(16.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("healthy"),
        Some("frame-await-recovery-keyframe"),
        Some("remoteTrackAttached"),
        Some(120_000),
        3_020.0,
        5,
    );
    noisy.clean_anchor_epoch = Some(5);
    noisy.clean_anchor_observed_at_ms = Some(3_018.0);
    noisy.clean_anchor_source_event = Some("chain-clean-keyframe-submitted".to_string());

    let output = owner.evaluate(&noisy);
    assert_eq!(output.state, VideoSchedulingOwnerState::StableServing);
    assert_eq!(output.health, VideoHealthContract::Stable);
    assert!(output.recovery_intent.is_none());
}

#[test]
fn submitted_clean_anchor_marks_rebuilding_supply_as_bootstrap_in_flight() {
    let mut owner = VideoSchedulingOwner::new();
    let _ = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        Some("transportAwaitRecoveryKeyframe"),
        SchedulingDemandSignal::default(),
        Some("recovering"),
        Some("frame-await-recovery-keyframe"),
        Some("remoteTrackAttached"),
        Some(64_000),
        1_000.0,
        7,
    ));

    let mut pending = input(
        ConnectionLifecycleStateFact::Connected,
        Some("transportAwaitRecoveryKeyframe"),
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("critical".to_string()),
            no_pending_streak: Some(180),
            present_age_ms: Some(420.0),
            decode_age_ms: Some(380.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("recovering"),
        Some("frame-await-recovery-keyframe"),
        Some("remoteTrackAttached"),
        Some(120_000),
        1_280.0,
        7,
    );
    pending.clean_anchor_epoch = Some(7);
    pending.clean_anchor_observed_at_ms = Some(1_250.0);
    pending.clean_anchor_source_event = Some("chain-clean-keyframe-submitted".to_string());
    pending.latest_anchor_candidate_ledger = Some(crate::XbxEngineAnchorCandidateLedger {
        recovery_epoch: 7,
        frame_rtp_timestamp: Some(7_001),
        state: crate::XbxEngineAnchorCandidateState::SubmittedCleanAnchor,
        source_event: "chain-clean-keyframe-submitted".to_string(),
        failure_reason: None,
        observed_at_ms: 1_255.0,
    });

    let output = owner.evaluate(&pending);
    assert_eq!(output.state, VideoSchedulingOwnerState::RebuildingSupply);
    let intent = output.recovery_intent.expect("bootstrap in flight intent");
    assert_eq!(intent.reason_label, "recoverySustaining");
}

#[test]
fn submitted_clean_anchor_within_sustaining_phase_keeps_bootstrap_in_flight_reason() {
    let mut owner = VideoSchedulingOwner::new();
    let _ = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        Some("transportAwaitRecoveryKeyframe"),
        SchedulingDemandSignal::default(),
        Some("recovering"),
        Some("frame-await-recovery-keyframe"),
        Some("remoteTrackAttached"),
        Some(64_000),
        1_000.0,
        7,
    ));

    let mut pending = input(
        ConnectionLifecycleStateFact::Connected,
        Some("transportAwaitRecoveryKeyframe"),
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("critical".to_string()),
            no_pending_streak: Some(180),
            present_age_ms: Some(420.0),
            decode_age_ms: Some(380.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("recovering"),
        Some("frame-await-recovery-keyframe"),
        Some("remoteTrackAttached"),
        Some(120_000),
        1_650.0,
        7,
    );
    pending.clean_anchor_epoch = Some(7);
    pending.clean_anchor_observed_at_ms = Some(1_250.0);
    pending.clean_anchor_source_event = Some("chain-clean-keyframe-submitted".to_string());
    pending.latest_anchor_candidate_ledger = Some(crate::XbxEngineAnchorCandidateLedger {
        recovery_epoch: 7,
        frame_rtp_timestamp: Some(7_001),
        state: crate::XbxEngineAnchorCandidateState::SubmittedCleanAnchor,
        source_event: "chain-clean-keyframe-submitted".to_string(),
        failure_reason: None,
        observed_at_ms: 1_255.0,
    });

    let output = owner.evaluate(&pending);
    assert_eq!(output.state, VideoSchedulingOwnerState::RebuildingSupply);
    let intent = output.recovery_intent.expect("transport await intent");
    assert_eq!(intent.reason_label, "recoverySustaining");
}

#[test]
fn sustaining_recovery_with_serviceable_output_exits_rebuilding_supply_as_degraded() {
    let mut owner = VideoSchedulingOwner::new();
    let _ = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        Some("transportAwaitRecoveryKeyframe"),
        SchedulingDemandSignal::default(),
        Some("recovering"),
        Some("frame-await-recovery-keyframe"),
        Some("remoteTrackAttached"),
        Some(64_000),
        1_000.0,
        17,
    ));

    let mut sustaining = input(
        ConnectionLifecycleStateFact::Connected,
        Some("transportAwaitRecoveryKeyframe"),
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("critical".to_string()),
            no_pending_streak: Some(180),
            present_age_ms: Some(420.0),
            decode_age_ms: Some(260.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("sustaining-recovery"),
        Some("frame-observed"),
        Some("remoteTrackAttached"),
        Some(120_000),
        1_180.0,
        17,
    );
    sustaining.clean_anchor_epoch = Some(17);
    sustaining.clean_anchor_observed_at_ms = Some(1_120.0);
    sustaining.clean_anchor_source_event = Some("chain-clean-keyframe-submitted".to_string());
    sustaining.latest_anchor_candidate_ledger = Some(crate::XbxEngineAnchorCandidateLedger {
        recovery_epoch: 17,
        frame_rtp_timestamp: Some(17_001),
        state: crate::XbxEngineAnchorCandidateState::SubmittedCleanAnchor,
        source_event: "chain-clean-keyframe-submitted".to_string(),
        failure_reason: None,
        observed_at_ms: 1_120.0,
    });

    let output = owner.evaluate(&sustaining);
    assert_eq!(output.state, VideoSchedulingOwnerState::DegradedServing);
    assert_eq!(output.health, VideoHealthContract::Stable);
    assert!(output.recovery_intent.is_none());
}

#[test]
fn sustaining_phase_stops_when_hard_rebuild_evidence_reappears() {
    let mut owner = VideoSchedulingOwner::new();
    let _ = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        Some("transportAwaitRecoveryKeyframe"),
        SchedulingDemandSignal::default(),
        Some("recovering"),
        Some("frame-await-recovery-keyframe"),
        Some("remoteTrackAttached"),
        Some(64_000),
        1_000.0,
        7,
    ));

    let mut pending = input(
        ConnectionLifecycleStateFact::Connected,
        Some("transportAwaitRecoveryKeyframe"),
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("critical".to_string()),
            no_pending_streak: Some(180),
            present_age_ms: Some(420.0),
            decode_age_ms: Some(380.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("broken"),
        Some("frame-await-recovery-keyframe"),
        Some("remoteTrackAttached"),
        Some(120_000),
        1_650.0,
        7,
    );
    pending.clean_anchor_epoch = Some(7);
    pending.clean_anchor_observed_at_ms = Some(1_250.0);
    pending.clean_anchor_source_event = Some("chain-clean-keyframe-submitted".to_string());
    pending.latest_anchor_candidate_ledger = Some(crate::XbxEngineAnchorCandidateLedger {
        recovery_epoch: 7,
        frame_rtp_timestamp: Some(7_001),
        state: crate::XbxEngineAnchorCandidateState::Rejected,
        source_event: "frame-await-recovery-keyframe".to_string(),
        failure_reason: Some(crate::XbxEngineAnchorCandidateFailureReason::GapExpiredDeadline),
        observed_at_ms: 1_640.0,
    });

    let output = owner.evaluate(&pending);
    assert_eq!(output.state, VideoSchedulingOwnerState::RebuildingSupply);
    let intent = output.recovery_intent.expect("transport await intent");
    assert_eq!(intent.reason_label, "transportAwaitRecoveryKeyframe");
}

#[test]
fn sustaining_phase_stops_when_transport_await_reappears_after_clean_anchor() {
    let mut owner = VideoSchedulingOwner::new();
    let _ = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        Some("transportAwaitRecoveryKeyframe"),
        SchedulingDemandSignal::default(),
        Some("recovering"),
        Some("frame-await-recovery-keyframe"),
        Some("remoteTrackAttached"),
        Some(64_000),
        1_000.0,
        7,
    ));

    let mut pending = input(
        ConnectionLifecycleStateFact::Connected,
        Some("transportAwaitRecoveryKeyframe"),
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("critical".to_string()),
            no_pending_streak: Some(180),
            present_age_ms: Some(420.0),
            decode_age_ms: Some(380.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("recovering"),
        Some("frame-await-recovery-keyframe"),
        Some("remoteTrackAttached"),
        Some(120_000),
        1_650.0,
        7,
    );
    pending.clean_anchor_epoch = Some(7);
    pending.clean_anchor_observed_at_ms = Some(1_250.0);
    pending.clean_anchor_source_event = Some("chain-clean-keyframe-submitted".to_string());
    pending.latest_anchor_candidate_ledger = Some(crate::XbxEngineAnchorCandidateLedger {
        recovery_epoch: 7,
        frame_rtp_timestamp: Some(7_001),
        state: crate::XbxEngineAnchorCandidateState::SubmittedCleanAnchor,
        source_event: "chain-clean-keyframe-submitted".to_string(),
        failure_reason: None,
        observed_at_ms: 1_255.0,
    });
    pending.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
        observation_id: 77,
        source_event: "gap-repair-in-flight".to_string(),
        gap: Some(crate::XbxEngineVideoTimelineGapSnapshot {
            state: "repair-in-flight".to_string(),
            sequence: Some(701),
            frame_rtp_timestamp: Some(70_100),
            frame_importance: Some("keyframe".to_string()),
            budget_importance: Some("reference".to_string()),
            evidence_importance: Some("keyframe".to_string()),
            gap_dependency_confidence: Some("bound".to_string()),
            observed_at_ms: 1_640.0,
        }),
        frame: None,
        chain: crate::XbxEngineVideoTimelineChainSnapshot {
            state: "recovering".to_string(),
            reason: Some("transportAwaitRecoveryKeyframe".to_string()),
            chain_break_evidence: None,
            observed_at_ms: 1_640.0,
        },
        observed_at_ms: 1_640.0,
    });

    let output = owner.evaluate(&pending);
    assert_eq!(output.state, VideoSchedulingOwnerState::RebuildingSupply);
    let intent = output.recovery_intent.expect("transport await intent");
    assert_eq!(intent.reason_label, "transportAwaitRecoveryKeyframe");
}

#[test]
fn rebuilding_supply_cannot_close_by_clean_anchor_candidate_without_explicit_healthy_chain() {
    let mut owner = VideoSchedulingOwner::new();
    let _ = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        Some("transportAwaitRecoveryKeyframe"),
        SchedulingDemandSignal::default(),
        Some("recovering"),
        Some("frame-await-recovery-keyframe"),
        Some("remoteTrackAttached"),
        Some(10_000),
        1300.0,
        3,
    ));

    let mut ready = input(
        ConnectionLifecycleStateFact::Connected,
        None,
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("normal".to_string()),
            no_pending_streak: Some(0),
            present_age_ms: Some(16.0),
            decode_age_ms: Some(12.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("broken"),
        Some("gap-reorder-pending"),
        Some("remoteTrackAttached"),
        Some(120_000),
        1310.0,
        3,
    );
    ready.latest_anchor_candidate_ledger = Some(crate::XbxEngineAnchorCandidateLedger {
        recovery_epoch: 3,
        frame_rtp_timestamp: Some(120_000),
        state: crate::XbxEngineAnchorCandidateState::SubmittedCleanAnchor,
        source_event: "chain-clean-keyframe-submitted".to_string(),
        failure_reason: None,
        observed_at_ms: 1309.0,
    });

    let not_ready = owner.evaluate(&ready);
    assert_eq!(not_ready.state, VideoSchedulingOwnerState::RebuildingSupply);
    assert_eq!(not_ready.health, VideoHealthContract::Recovering);
}

#[test]
fn rebuilding_supply_allows_current_clean_anchor_candidate() {
    let mut owner = VideoSchedulingOwner::new();
    let _ = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        Some("transportAwaitRecoveryKeyframe"),
        SchedulingDemandSignal::default(),
        Some("recovering"),
        Some("frame-await-recovery-keyframe"),
        Some("remoteTrackAttached"),
        Some(40_000),
        2_000.0,
        10,
    ));

    let mut ready = input(
        ConnectionLifecycleStateFact::Connected,
        None,
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("normal".to_string()),
            no_pending_streak: Some(0),
            present_age_ms: Some(14.0),
            decode_age_ms: Some(11.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("healthy"),
        Some("frame-observed"),
        Some("remoteTrackAttached"),
        Some(150_000),
        2_700.0,
        10,
    );
    ready.latest_anchor_candidate_ledger = Some(crate::XbxEngineAnchorCandidateLedger {
        recovery_epoch: 10,
        frame_rtp_timestamp: Some(149_900),
        state: crate::XbxEngineAnchorCandidateState::SubmittedCleanAnchor,
        source_event: "chain-clean-keyframe-submitted".to_string(),
        failure_reason: None,
        observed_at_ms: 2_000.0,
    });

    let stable = owner.evaluate(&ready);
    assert_eq!(stable.state, VideoSchedulingOwnerState::StableServing);
    assert_eq!(stable.health, VideoHealthContract::Stable);
    assert!(stable.diagnostics.temporary_diagnostic_summary.is_none());
}

#[test]
fn clean_anchor_recovery_can_close_on_transient_present_feedback_gap() {
    let mut owner = VideoSchedulingOwner::new();
    let _ = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        Some("transportAwaitRecoveryKeyframe"),
        SchedulingDemandSignal::default(),
        Some("recovering"),
        Some("frame-await-recovery-keyframe"),
        Some("remoteTrackAttached"),
        Some(20_000),
        1_500.0,
        7,
    ));

    let mut missing_present = input(
        ConnectionLifecycleStateFact::Connected,
        None,
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("normal".to_string()),
            no_pending_streak: Some(0),
            present_age_ms: None,
            decode_age_ms: Some(9.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("healthy"),
        Some("frame-observed"),
        Some("remoteTrackAttached"),
        Some(88_000),
        1_520.0,
        7,
    );
    missing_present.clean_anchor_epoch = Some(7);
    missing_present.clean_anchor_observed_at_ms = Some(1_518.0);
    missing_present.clean_anchor_source_event = Some("chain-clean-keyframe-submitted".to_string());
    let healed = owner.evaluate(&missing_present);
    assert_eq!(healed.state, VideoSchedulingOwnerState::RebuildingSupply);
    assert_eq!(healed.health, VideoHealthContract::Recovering);
}

#[test]
fn clean_anchor_recovery_can_exit_supply_starved_to_degraded_serving_before_full_supply_reset() {
    let mut owner = VideoSchedulingOwner::new();
    let _ = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        None,
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("critical".to_string()),
            no_pending_streak: Some(160),
            present_age_ms: Some(1_100.0),
            decode_age_ms: Some(620.0),
            video_renderer_stalled: true,
            ..SchedulingDemandSignal::default()
        },
        Some("healthy"),
        Some("frame-complete-candidate"),
        Some("remoteTrackAttached"),
        Some(20_000),
        1_500.0,
        8,
    ));

    let mut recovering = input(
        ConnectionLifecycleStateFact::Connected,
        None,
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("normal".to_string()),
            no_pending_streak: Some(0),
            present_age_ms: Some(18.0),
            decode_age_ms: Some(12.0),
            video_renderer_stalled: false,
            present_submit_count_total: Some(10),
            present_drop_count_total: Some(2),
            ..SchedulingDemandSignal::default()
        },
        Some("healthy"),
        Some("frame-observed"),
        Some("remoteTrackAttached"),
        Some(96_000),
        1_540.0,
        8,
    );
    recovering.clean_anchor_epoch = Some(8);
    recovering.clean_anchor_observed_at_ms = Some(1_538.0);
    recovering.clean_anchor_source_event = Some("chain-clean-keyframe-submitted".to_string());

    let output = owner.evaluate(&recovering);
    assert_eq!(output.state, VideoSchedulingOwnerState::DegradedServing);
    assert_eq!(output.health, VideoHealthContract::Stable);
    assert!(output.recovery_intent.is_none());
}

#[test]
fn clean_anchor_recovery_stays_rebuilding_when_present_feedback_gap_is_not_settled() {
    let mut owner = VideoSchedulingOwner::new();
    let _ = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        Some("transportAwaitRecoveryKeyframe"),
        SchedulingDemandSignal::default(),
        Some("recovering"),
        Some("frame-await-recovery-keyframe"),
        Some("remoteTrackAttached"),
        Some(20_000),
        1_500.0,
        7,
    ));

    let mut missing_present = input(
        ConnectionLifecycleStateFact::Connected,
        None,
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("high".to_string()),
            no_pending_streak: Some(6),
            present_age_ms: None,
            decode_age_ms: Some(9.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("healthy"),
        Some("frame-observed"),
        Some("remoteTrackAttached"),
        Some(88_000),
        1_520.0,
        7,
    );
    missing_present.clean_anchor_epoch = Some(7);
    missing_present.clean_anchor_observed_at_ms = Some(1_518.0);
    missing_present.clean_anchor_source_event = Some("chain-clean-keyframe-submitted".to_string());
    let blocked = owner.evaluate(&missing_present);
    assert_eq!(blocked.state, VideoSchedulingOwnerState::RebuildingSupply);
    assert_eq!(blocked.health, VideoHealthContract::Recovering);

    let mut fresh_present = input(
        ConnectionLifecycleStateFact::Connected,
        None,
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("normal".to_string()),
            no_pending_streak: Some(0),
            present_age_ms: Some(14.0),
            decode_age_ms: Some(9.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("healthy"),
        Some("frame-observed"),
        Some("remoteTrackAttached"),
        Some(90_000),
        1_540.0,
        7,
    );
    fresh_present.clean_anchor_epoch = Some(7);
    fresh_present.clean_anchor_observed_at_ms = Some(1_538.0);
    fresh_present.clean_anchor_source_event = Some("chain-clean-keyframe-submitted".to_string());
    let healed = owner.evaluate(&fresh_present);
    assert_eq!(healed.state, VideoSchedulingOwnerState::StableServing);
    assert_eq!(healed.health, VideoHealthContract::Stable);
}

#[test]
fn clean_anchor_with_terminal_invalid_bootstrap_releases_rebuilding_supply() {
    let mut owner = VideoSchedulingOwner::new();
    let first = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        Some("transportAwaitRecoveryKeyframe"),
        SchedulingDemandSignal::default(),
        None,
        None,
        None,
        None,
        100.0,
        7,
    ));
    assert_eq!(first.state, VideoSchedulingOwnerState::RebuildingSupply);

    let mut recovered = input(
        ConnectionLifecycleStateFact::Connected,
        Some("transportAwaitRecoveryKeyframe"),
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("normal".to_string()),
            no_pending_streak: Some(0),
            present_age_ms: Some(24.0),
            decode_age_ms: Some(18.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("broken"),
        Some("gap-reorder-pending"),
        Some("remoteTrackAttached"),
        Some(84_000),
        140.0,
        7,
    );
    recovered.clean_anchor_epoch = Some(7);
    recovered.clean_anchor_observed_at_ms = Some(132.0);
    recovered.clean_anchor_source_event = Some("chain-clean-keyframe-submitted".to_string());
    recovered.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
        observation_id: 1,
        source_event: "gap-reorder-pending".to_string(),
        gap: Some(crate::XbxEngineVideoTimelineGapSnapshot {
            state: "pending".to_string(),
            sequence: Some(33),
            frame_rtp_timestamp: None,
            frame_importance: Some("reference".to_string()),
            budget_importance: None,

            evidence_importance: None,

            gap_dependency_confidence: None,

            observed_at_ms: 139.0,
        }),
        frame: None,
        chain: crate::XbxEngineVideoTimelineChainSnapshot {
            state: "broken".to_string(),
            reason: None,
            chain_break_evidence: None,

            observed_at_ms: 139.0,
        },
        observed_at_ms: 139.0,
    });
    recovered.latest_h264_bootstrap_ready = Some(false);
    recovered.latest_h264_bootstrap_reject_reason = Some("NonIdrVcl".to_string());
    recovered.latest_h264_committed_sps_present = Some(true);
    recovered.latest_h264_committed_pps_present = Some(true);
    recovered.latest_h264_delta_continuation_ready = Some(true);
    recovered.latest_h264_observed_at_ms = Some(139.0);

    let second = owner.evaluate(&recovered);
    assert_eq!(second.state, VideoSchedulingOwnerState::DegradedServing);
    assert!(second.recovery_intent.is_none());
}

#[test]
fn terminal_invalid_bootstrap_without_clean_anchor_releases_rebuilding_supply_when_output_serviceable(
) {
    let mut owner = VideoSchedulingOwner::new();
    let first = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        Some("transportAwaitRecoveryKeyframe"),
        SchedulingDemandSignal::default(),
        None,
        None,
        None,
        None,
        100.0,
        10,
    ));
    assert_eq!(first.state, VideoSchedulingOwnerState::RebuildingSupply);

    let mut recovered = input(
        ConnectionLifecycleStateFact::Connected,
        Some("transportAwaitRecoveryKeyframe"),
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("normal".to_string()),
            no_pending_streak: Some(0),
            present_age_ms: Some(26.0),
            decode_age_ms: Some(18.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("broken"),
        Some("gap-reorder-pending"),
        Some("remoteTrackAttached"),
        Some(88_000),
        140.0,
        10,
    );
    recovered.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
        observation_id: 3,
        source_event: "gap-reorder-pending".to_string(),
        gap: Some(crate::XbxEngineVideoTimelineGapSnapshot {
            state: "pending".to_string(),
            sequence: Some(61),
            frame_rtp_timestamp: None,
            frame_importance: Some("reference".to_string()),
            budget_importance: None,

            evidence_importance: None,

            gap_dependency_confidence: None,

            observed_at_ms: 139.0,
        }),
        frame: None,
        chain: crate::XbxEngineVideoTimelineChainSnapshot {
            state: "broken".to_string(),
            reason: None,
            chain_break_evidence: None,

            observed_at_ms: 139.0,
        },
        observed_at_ms: 139.0,
    });
    recovered.latest_h264_bootstrap_ready = Some(false);
    recovered.latest_h264_bootstrap_reject_reason = Some("NonIdrVcl".to_string());
    recovered.latest_h264_committed_sps_present = Some(true);
    recovered.latest_h264_committed_pps_present = Some(true);
    recovered.latest_h264_delta_continuation_ready = Some(true);
    recovered.latest_h264_observed_at_ms = Some(139.0);

    let second = owner.evaluate(&recovered);
    assert_eq!(second.state, VideoSchedulingOwnerState::DegradedServing);
    assert!(second.recovery_intent.is_none());
}

#[test]
fn terminal_invalid_bootstrap_without_clean_anchor_and_without_serviceable_output_stays_rebuilding_supply(
) {
    let mut owner = VideoSchedulingOwner::new();
    let first = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        Some("transportAwaitRecoveryKeyframe"),
        SchedulingDemandSignal::default(),
        None,
        None,
        None,
        None,
        100.0,
        11,
    ));
    assert_eq!(first.state, VideoSchedulingOwnerState::RebuildingSupply);

    let mut blocked = input(
        ConnectionLifecycleStateFact::Connected,
        Some("transportAwaitRecoveryKeyframe"),
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("normal".to_string()),
            no_pending_streak: Some(0),
            present_age_ms: Some(420.0),
            decode_age_ms: Some(380.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("broken"),
        Some("gap-reorder-pending"),
        Some("remoteTrackAttached"),
        Some(88_000),
        140.0,
        11,
    );
    blocked.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
        observation_id: 4,
        source_event: "gap-reorder-pending".to_string(),
        gap: Some(crate::XbxEngineVideoTimelineGapSnapshot {
            state: "pending".to_string(),
            sequence: Some(62),
            frame_rtp_timestamp: None,
            frame_importance: Some("reference".to_string()),
            budget_importance: None,

            evidence_importance: None,

            gap_dependency_confidence: None,

            observed_at_ms: 139.0,
        }),
        frame: None,
        chain: crate::XbxEngineVideoTimelineChainSnapshot {
            state: "broken".to_string(),
            reason: None,
            chain_break_evidence: None,

            observed_at_ms: 139.0,
        },
        observed_at_ms: 139.0,
    });
    blocked.latest_h264_bootstrap_ready = Some(false);
    blocked.latest_h264_bootstrap_reject_reason = Some("NonIdrVcl".to_string());
    blocked.latest_h264_committed_sps_present = Some(true);
    blocked.latest_h264_committed_pps_present = Some(true);
    blocked.latest_h264_delta_continuation_ready = Some(true);
    blocked.latest_h264_observed_at_ms = Some(139.0);

    let second = owner.evaluate(&blocked);
    assert_eq!(second.state, VideoSchedulingOwnerState::RebuildingSupply);
    assert!(second.recovery_intent.is_some());
}

#[test]
fn degraded_supply_still_releases_terminal_invalid_bootstrap_waiting() {
    let mut owner = VideoSchedulingOwner::new();
    let first = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        Some("transportAwaitRecoveryKeyframe"),
        SchedulingDemandSignal::default(),
        None,
        None,
        None,
        None,
        100.0,
        8,
    ));
    assert_eq!(first.state, VideoSchedulingOwnerState::RebuildingSupply);

    let mut recovered = input(
        ConnectionLifecycleStateFact::Connected,
        Some("transportAwaitRecoveryKeyframe"),
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("normal".to_string()),
            no_pending_streak: Some(0),
            present_age_ms: Some(210.0),
            decode_age_ms: Some(150.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("broken"),
        Some("gap-reorder-pending"),
        Some("remoteTrackAttached"),
        Some(96_000),
        140.0,
        8,
    );
    recovered.clean_anchor_epoch = Some(8);
    recovered.clean_anchor_observed_at_ms = Some(132.0);
    recovered.clean_anchor_source_event = Some("chain-clean-keyframe-submitted".to_string());
    recovered.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
        observation_id: 2,
        source_event: "gap-reorder-pending".to_string(),
        gap: Some(crate::XbxEngineVideoTimelineGapSnapshot {
            state: "pending".to_string(),
            sequence: Some(52),
            frame_rtp_timestamp: None,
            frame_importance: Some("reference".to_string()),
            budget_importance: None,

            evidence_importance: None,

            gap_dependency_confidence: None,

            observed_at_ms: 139.0,
        }),
        frame: None,
        chain: crate::XbxEngineVideoTimelineChainSnapshot {
            state: "broken".to_string(),
            reason: None,
            chain_break_evidence: None,

            observed_at_ms: 139.0,
        },
        observed_at_ms: 139.0,
    });
    recovered.latest_h264_bootstrap_ready = Some(false);
    recovered.latest_h264_bootstrap_reject_reason = Some("NonIdrVcl".to_string());
    recovered.latest_h264_committed_sps_present = Some(true);
    recovered.latest_h264_committed_pps_present = Some(true);
    recovered.latest_h264_delta_continuation_ready = Some(true);
    recovered.latest_h264_observed_at_ms = Some(139.0);

    let second = owner.evaluate(&recovered);
    assert_eq!(second.state, VideoSchedulingOwnerState::DegradedServing);
    assert!(second.recovery_intent.is_none());
}

#[test]
fn supply_starved_probe_with_clean_anchor_stays_out_of_rebuilding_supply() {
    let mut owner = VideoSchedulingOwner::new();
    let _ = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        None,
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("critical".to_string()),
            no_pending_streak: Some(320),
            present_age_ms: Some(1_500.0),
            decode_age_ms: Some(18.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("healthy"),
        Some("frame-complete-candidate"),
        Some("remoteTrackAttached"),
        Some(96_000),
        100.0,
        9,
    ));
    let _ = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        None,
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("critical".to_string()),
            no_pending_streak: Some(340),
            present_age_ms: Some(1_600.0),
            decode_age_ms: Some(20.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("healthy"),
        Some("frame-complete-candidate"),
        Some("remoteTrackAttached"),
        Some(96_000),
        420.0,
        9,
    ));

    let mut probed = input(
        ConnectionLifecycleStateFact::Connected,
        None,
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("critical".to_string()),
            no_pending_streak: Some(360),
            present_age_ms: Some(1_700.0),
            decode_age_ms: Some(22.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("healthy"),
        Some("frame-await-recovery-keyframe"),
        Some("remoteTrackAttached"),
        Some(96_000),
        700.0,
        9,
    );
    probed.clean_anchor_epoch = Some(9);
    probed.clean_anchor_observed_at_ms = Some(698.0);
    probed.clean_anchor_source_event = Some("chain-clean-keyframe-submitted".to_string());

    let output = owner.evaluate(&probed);
    assert_ne!(output.state, VideoSchedulingOwnerState::RebuildingSupply);
    assert!(output.recovery_intent.is_none());
}
