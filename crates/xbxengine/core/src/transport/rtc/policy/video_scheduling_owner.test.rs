use super::{
    RecoveryIntentSource, VideoHealthContract, VideoSchedulingOwner,
    VideoSchedulingOwnerContractState, VideoSchedulingOwnerInput, VideoSchedulingOwnerState,
};
use crate::transport::rtc::facts::ConnectionLifecycleStateFact;
use crate::transport::rtc::policy::display_supply::SchedulingDemandSignal;
use crate::transport::rtc::recovery::contract::{
    DerivedDecoderHealth, RecoveryExitPath, RecoverySurfacePhase,
};
use crate::transport::rtc::recovery::policy::DisplaySupplyThresholds;

/// 为 displayed-idr / clean-anchor 退出 rebuilding 的用例补齐 receive ledger 投影。
fn seed_ledger_display_recovery(input: &mut VideoSchedulingOwnerInput) {
    input.receive_keyframe_required = Some(false);
    input.receive_keyframe_response_state = Some("usable-idr".to_string());
    input.receive_display_state = Some("display-stable".to_string());

    if input.clean_anchor_epoch.is_none() {
        input.clean_anchor_epoch = Some(input.recovery_epoch);
    }
    if input.recovery_decoder_reference_synced_at_ms.is_none() {
        input.recovery_decoder_reference_synced_at_ms =
            Some((input.observed_at_ms - 50.0).max(0.0));
    }
    if input.transport_recovery_episode_opened_at_ms.is_none() {
        input.transport_recovery_episode_opened_at_ms =
            Some((input.observed_at_ms - 500.0).max(0.0));
    }
}

#[test]
fn clean_anchor_without_receive_ledger_projection_does_not_release() {
    let mut owner = VideoSchedulingOwner::new();
    let _ = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal::default(),
        Some("recovering"),
        Some("frame-await-recovery-anchor"),
        Some("remoteTrackAttached"),
        Some(24_000),
        620.0,
        1,
    ));

    let mut anchor_only = input(
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
        Some("receiving"),
        Some("frame-observed"),
        Some("remoteTrackAttached"),
        Some(42_000),
        640.0,
        1,
    );
    anchor_only.clean_anchor_epoch = Some(1);
    anchor_only.clean_anchor_observed_at_ms = Some(639.0);
    anchor_only.clean_anchor_source_event = Some("displayed-idr".to_string());
    anchor_only.recovery_displayed_idr_at_ms = anchor_only.clean_anchor_observed_at_ms;
    anchor_only.recovery_fresh_anchor_recovered_at_ms = anchor_only.clean_anchor_observed_at_ms;

    let output = owner.evaluate(&anchor_only);
    assert_eq!(output.state, VideoSchedulingOwnerState::RebuildingSupply);
}

#[test]
fn displayed_idr_playback_release_without_control_plane_exits_rebuilding_supply() {
    let mut owner = VideoSchedulingOwner::new();
    let _ = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal::default(),
        Some("waiting-keyframe"),
        Some("frame-await-recovery-anchor"),
        Some("remoteTrackAttached"),
        Some(24_000),
        620.0,
        1,
    ));

    let mut playback_ready = input(
        ConnectionLifecycleStateFact::Connected,
        None,
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("normal".to_string()),
            no_pending_streak: Some(0),
            present_age_ms: Some(16.0),
            decode_age_ms: Some(12.0),
            video_renderer_stalled: false,
            host_display_tick_epoch: Some(42),
            host_frame_present_epoch: Some(7),
            host_cadence_phase: Some("steady".to_string()),
            submit_age_ms: Some(24.0),
            ..SchedulingDemandSignal::default()
        },
        Some("waiting-keyframe"),
        Some("insert-gate-need-keyframe"),
        Some("remoteTrackAttached"),
        Some(48_000),
        720.0,
        1,
    );
    playback_ready.clean_anchor_epoch = Some(1);
    playback_ready.clean_anchor_observed_at_ms = Some(710.0);
    playback_ready.clean_anchor_source_event = Some("displayed-idr".to_string());
    playback_ready.recovery_displayed_idr_at_ms = playback_ready.clean_anchor_observed_at_ms;
    playback_ready.recovery_fresh_anchor_recovered_at_ms =
        playback_ready.clean_anchor_observed_at_ms;
    playback_ready.receive_keyframe_required = Some(true);
    playback_ready.receive_keyframe_response_state = Some("non-idr-only".to_string());
    playback_ready.recovery_decoder_reference_synced_at_ms = Some(715.0);
    playback_ready.transport_recovery_episode_opened_at_ms = Some(200.0);

    let output = owner.evaluate(&playback_ready);
    assert_ne!(output.state, VideoSchedulingOwnerState::RebuildingSupply);
    assert!(output.recovery_intent.is_none());
}

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
        receiver_state: timeline_chain_state.map(str::to_string),
        first_frame_acquisition_priority_allowed: true,
        anchor_reason_label: anchor_reason_label.map(str::to_string),
        demand,
        clean_anchor_epoch: None,
        clean_anchor_observed_at_ms: None,
        clean_anchor_source_event: None,
        clean_anchor_bridge_epoch: None,
        clean_anchor_bridge_observed_at_ms: None,
        clean_anchor_bridge_source_event: None,
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
        recovery_displayed_idr_at_ms: None,
        recovery_fresh_anchor_recovered_at_ms: None,
        recovery_exit_path: RecoveryExitPath::AwaitingAnchor,
        recovery_surface_phase: RecoverySurfacePhase::Steady,
        derived_decoder_health: DerivedDecoderHealth::Nominal,
        display_supply_thresholds: thresholds(),
        observed_at_ms,
        latest_anchor_candidate_ledger: None,
        receive_keyframe_required: None,
        receive_keyframe_response_state: None,
        receive_display_state: None,
        recovery_decoder_reference_synced_at_ms: None,
        transport_recovery_episode_opened_at_ms: None,
    }
}

#[test]
fn supply_break_surface_stays_projection_only_without_forcing_supply_starved() {
    let mut owner = VideoSchedulingOwner::new();
    let _ = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal::default(),
        None,
        Some("frame-await-recovery-anchor"),
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
        Some("receiving"),
        Some("frame-complete-candidate"),
        Some("remoteTrackAttached"),
        Some(20_000),
        520.0,
        1,
    );
    ready.clean_anchor_epoch = Some(1);
    ready.clean_anchor_observed_at_ms = Some(518.0);
    ready.clean_anchor_source_event = Some("displayed-idr".to_string());
    ready.recovery_displayed_idr_at_ms = ready.clean_anchor_observed_at_ms;
    ready.recovery_fresh_anchor_recovered_at_ms = ready.clean_anchor_observed_at_ms;
    let _ = owner.evaluate(&ready);

    let mut inp = input(
        ConnectionLifecycleStateFact::Connected,
        Some("frame-await-recovery-anchor"),
        SchedulingDemandSignal::default(),
        Some("waiting-keyframe"),
        Some("frame-await-recovery-anchor"),
        Some("remoteTrackAttached"),
        Some(64_000),
        600.0,
        2,
    );
    inp.recovery_surface_phase = RecoverySurfacePhase::SupplyBreak;
    inp.derived_decoder_health = DerivedDecoderHealth::Nominal;

    let output = owner.evaluate(&inp);
    assert_ne!(output.state, VideoSchedulingOwnerState::SupplyStarved);
    assert!(output
        .recovery_intent
        .as_ref()
        .is_none_or(|intent| intent.reason_label != "displaySupplyCritical"));
}

#[test]
fn surface_matrix_steady_stable_serving_emits_no_recovery_intent() {
    let mut owner = VideoSchedulingOwner::new();
    let _ = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal::default(),
        None,
        Some("frame-await-recovery-anchor"),
        Some("remoteTrackAttached"),
        Some(10_000),
        300.0,
        1,
    ));
    let mut settled = input(
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
        Some("receiving"),
        Some("frame-complete-candidate"),
        Some("remoteTrackAttached"),
        Some(20_000),
        520.0,
        1,
    );
    settled.clean_anchor_epoch = Some(1);
    settled.clean_anchor_observed_at_ms = Some(518.0);
    settled.clean_anchor_source_event = Some("displayed-idr".to_string());
    settled.recovery_displayed_idr_at_ms = settled.clean_anchor_observed_at_ms;
    settled.recovery_fresh_anchor_recovered_at_ms = settled.clean_anchor_observed_at_ms;

    settled.recovery_surface_phase = RecoverySurfacePhase::Steady;
    let output = owner.evaluate(&settled);
    assert_eq!(output.state, VideoSchedulingOwnerState::StableServing);
    assert!(output.recovery_intent.is_none());
}

#[test]
fn surface_matrix_await_idr_rebuilding_emits_waiting_keyframe_intent() {
    let mut owner = VideoSchedulingOwner::new();
    let mut inp = input(
        ConnectionLifecycleStateFact::Connected,
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal::default(),
        Some("waiting-keyframe"),
        Some("frame-await-recovery-anchor"),
        Some("remoteTrackAttached"),
        Some(10_000),
        200.0,
        1,
    );
    inp.recovery_surface_phase = RecoverySurfacePhase::AwaitIdr;
    inp.derived_decoder_health = DerivedDecoderHealth::AwaitIdr;
    let output = owner.evaluate(&inp);
    assert_eq!(output.state, VideoSchedulingOwnerState::RebuildingSupply);
    let intent = output.recovery_intent.expect("await-idr intent");
    assert_eq!(intent.reason_label, "receiverWaitingKeyframe");
}

#[test]
fn surface_matrix_repairing_gap_in_flight_avoids_transport_await_intent() {
    let mut owner = VideoSchedulingOwner::new();
    let _ = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal::default(),
        None,
        Some("frame-await-recovery-anchor"),
        Some("remoteTrackAttached"),
        Some(10_000),
        300.0,
        1,
    ));
    let mut inp = input(
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
        Some("repairing"),
        Some("gap-repair-in-flight"),
        Some("remoteTrackAttached"),
        Some(48_000),
        400.0,
        1,
    );
    inp.recovery_surface_phase = RecoverySurfacePhase::Repairing;

    inp.recovery_displayed_idr_at_ms = Some(390.0);
    inp.recovery_fresh_anchor_recovered_at_ms = Some(390.0);
    inp.clean_anchor_epoch = Some(1);
    inp.clean_anchor_observed_at_ms = Some(385.0);
    inp.clean_anchor_source_event = Some("displayed-idr".to_string());
    let output = owner.evaluate(&inp);
    assert!(
        matches!(
            output.state,
            VideoSchedulingOwnerState::StableServing
                | VideoSchedulingOwnerState::DegradedServing
                | VideoSchedulingOwnerState::RebuildingSupply
        ),
        "unexpected state {:?}",
        output.state
    );
    assert!(
        output
            .recovery_intent
            .as_ref()
            .is_none_or(|intent| { intent.reason_label != "receiverWaitingKeyframe" }),
        "repairing surface must not emit transport-await: {:?}",
        output.recovery_intent
    );
}

#[test]
fn anchor_broken_enters_rebuilding_supply() {
    let mut owner = VideoSchedulingOwner::new();
    let output = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        Some("receiverWaitingKeyframe"),
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
    assert_eq!(intent.reason_label, "receiverWaitingKeyframe");
    assert!(intent.emit);
}

#[test]
fn h264_continuation_without_receiver_waiting_stays_supply_starved_not_anchor_rebuild() {
    let mut owner = VideoSchedulingOwner::new();
    let mut starved = input(
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
        Some("receiving"),
        Some("gap-repair-in-flight"),
        Some("remoteTrackAttached"),
        Some(64_000),
        200.0,
        1,
    );
    starved.receiver_state = Some("repairing".to_string());
    starved.latest_h264_bootstrap_ready = Some(false);
    starved.latest_h264_bootstrap_reject_reason = Some("bootstrapMissingIdr".to_string());
    starved.latest_h264_committed_sps_present = Some(true);
    starved.latest_h264_committed_pps_present = Some(true);
    starved.latest_h264_delta_continuation_ready = Some(true);

    let output = owner.evaluate(&starved);
    assert_eq!(output.state, VideoSchedulingOwnerState::SupplyStarved);
    let intent = output.recovery_intent.expect("supply intent");
    assert_eq!(intent.source, RecoveryIntentSource::Supply);
    assert_eq!(intent.reason_label, "displaySupplyCritical");
}

#[test]
fn stale_timeline_waiting_keyframe_with_receiver_repairing_stays_serving() {
    let mut owner = VideoSchedulingOwner::new();
    let _ = owner.evaluate(&input(
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
        Some("receiving"),
        Some("frame-complete-candidate"),
        Some("remoteTrackAttached"),
        Some(96_000),
        100.0,
        1,
    ));
    let _ = owner.evaluate(&input(
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
        Some("receiving"),
        Some("frame-complete-candidate"),
        Some("remoteTrackAttached"),
        Some(96_000),
        140.0,
        1,
    ));

    let mut during_repair = input(
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
        Some("waiting-keyframe"),
        Some("frame-await-recovery-anchor"),
        Some("remoteTrackAttached"),
        Some(96_000),
        200.0,
        1,
    );
    during_repair.receiver_state = Some("repairing".to_string());
    during_repair.latest_video_timeline_observation =
        Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 9,
            source_event: "frame-await-recovery-anchor".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "waiting-keyframe".to_string(),
                reason: Some("receiverWaitingKeyframe".to_string()),
                chain_break_evidence: None,
                observed_at_ms: 199.0,
            },
            observed_at_ms: 199.0,
        });

    let output = owner.evaluate(&during_repair);
    assert_ne!(output.state, VideoSchedulingOwnerState::RebuildingSupply);
    assert!(output
        .recovery_intent
        .as_ref()
        .is_none_or(|intent| intent.reason_label != "receiverWaitingKeyframe"));
}

#[test]
fn terminal_invalid_bootstrap_on_stable_without_receiver_waiting_enters_supply_starved() {
    let mut owner = VideoSchedulingOwner::new();
    let _ = owner.evaluate(&input(
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
        Some("receiving"),
        Some("frame-complete-candidate"),
        Some("remoteTrackAttached"),
        Some(96_000),
        100.0,
        2,
    ));
    let _ = owner.evaluate(&input(
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
        Some("receiving"),
        Some("frame-complete-candidate"),
        Some("remoteTrackAttached"),
        Some(96_000),
        140.0,
        2,
    ));

    let mut blocked = input(
        ConnectionLifecycleStateFact::Connected,
        None,
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("critical".to_string()),
            no_pending_streak: Some(168),
            present_age_ms: Some(900.0),
            decode_age_ms: Some(700.0),
            video_renderer_stalled: true,
            ..SchedulingDemandSignal::default()
        },
        Some("receiving"),
        Some("frame-complete-candidate"),
        Some("remoteTrackAttached"),
        Some(96_000),
        220.0,
        2,
    );
    blocked.receiver_state = Some("receiving".to_string());
    blocked.latest_h264_bootstrap_ready = Some(false);
    blocked.latest_h264_bootstrap_reject_reason = Some("NonIdrVcl".to_string());
    blocked.latest_h264_committed_sps_present = Some(true);
    blocked.latest_h264_committed_pps_present = Some(true);
    blocked.latest_h264_delta_continuation_ready = Some(true);
    blocked.latest_h264_observed_at_ms = Some(219.0);

    let output = owner.evaluate(&blocked);
    assert_eq!(output.state, VideoSchedulingOwnerState::SupplyStarved);
    let intent = output.recovery_intent.expect("supply intent");
    assert_eq!(intent.source, RecoveryIntentSource::Supply);
    assert_ne!(intent.reason_label, "receiverWaitingKeyframe");
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
        Some("receiving"),
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
        Some("receiving"),
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
        Some("receiving"),
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
        Some("receiving"),
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
        Some("receiving"),
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
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal::default(),
        None,
        Some("frame-await-recovery-anchor"),
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
        Some("receiving"),
        Some("frame-complete-candidate"),
        Some("remoteTrackAttached"),
        Some(20_000),
        520.0,
        1,
    );
    ready.clean_anchor_epoch = Some(1);
    ready.clean_anchor_observed_at_ms = Some(518.0);
    ready.clean_anchor_source_event = Some("displayed-idr".to_string());

    ready.recovery_displayed_idr_at_ms = ready.clean_anchor_observed_at_ms;

    ready.recovery_fresh_anchor_recovered_at_ms = ready.clean_anchor_observed_at_ms;
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
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal::default(),
        Some("recovering"),
        Some("frame-await-recovery-anchor"),
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
        Some("receiving"),
        Some("frame-complete-candidate"),
        Some("remoteTrackAttached"),
        Some(64_000),
        340.0,
        1,
    );
    codec_blocked.clean_anchor_epoch = Some(1);
    codec_blocked.clean_anchor_observed_at_ms = Some(338.0);
    codec_blocked.clean_anchor_source_event = Some("displayed-idr".to_string());

    codec_blocked.recovery_displayed_idr_at_ms = codec_blocked.clean_anchor_observed_at_ms;

    codec_blocked.recovery_fresh_anchor_recovered_at_ms = codec_blocked.clean_anchor_observed_at_ms;
    codec_blocked.latest_h264_bootstrap_ready = Some(false);
    codec_blocked.latest_h264_bootstrap_reject_reason = Some("NonIdrVcl".to_string());
    codec_blocked.latest_h264_observed_at_ms = Some(339.0);

    let blocked = owner.evaluate(&codec_blocked);
    assert_eq!(blocked.state, VideoSchedulingOwnerState::StableServing);
    assert_eq!(blocked.health, VideoHealthContract::Stable);
    assert!(blocked.recovery_intent.is_none());
}

#[test]
fn non_idr_with_committed_sets_and_delta_ready_does_not_block_recovery_exit() {
    let mut owner = VideoSchedulingOwner::new();
    let _ = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal::default(),
        Some("recovering"),
        Some("frame-await-recovery-anchor"),
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
        Some("receiving"),
        Some("frame-complete-candidate"),
        Some("remoteTrackAttached"),
        Some(64_000),
        340.0,
        1,
    );
    recoverable.clean_anchor_epoch = Some(1);
    recoverable.clean_anchor_observed_at_ms = Some(338.0);
    recoverable.clean_anchor_source_event = Some("displayed-idr".to_string());

    recoverable.recovery_displayed_idr_at_ms = recoverable.clean_anchor_observed_at_ms;

    recoverable.recovery_fresh_anchor_recovered_at_ms = recoverable.clean_anchor_observed_at_ms;
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
fn rebuilding_supply_with_clean_anchor_and_host_present_stall_switches_to_host_stall_supply_reason()
{
    let mut owner = VideoSchedulingOwner::new();
    let _ = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal {
            host_display_tick_epoch: Some(10),
            host_frame_present_epoch: Some(3),
            ..SchedulingDemandSignal::default()
        },
        Some("recovering"),
        Some("frame-await-recovery-anchor"),
        Some("remoteTrackAttached"),
        Some(10_000),
        300.0,
        1,
    ));

    let mut final_output = None;
    for step in 0..7 {
        let mut stalled = input(
            ConnectionLifecycleStateFact::Connected,
            None,
            SchedulingDemandSignal {
                no_pending_pressure_level: Some("normal".to_string()),
                no_pending_streak: Some(0),
                present_age_ms: Some(2_400.0),
                decode_age_ms: Some(18.0),
                video_renderer_stalled: false,
                host_display_tick_epoch: Some(11 + step),
                host_frame_present_epoch: Some(3),
                host_cadence_phase: Some("steady".to_string()),
                ..SchedulingDemandSignal::default()
            },
            Some("receiving"),
            Some("frame-complete-candidate"),
            Some("remoteTrackAttached"),
            Some(64_000),
            3_520.0 + step as f64,
            1,
        );
        stalled.clean_anchor_epoch = Some(1);
        stalled.clean_anchor_observed_at_ms = Some(600.0);
        stalled.clean_anchor_source_event = Some("displayed-idr".to_string());

        stalled.recovery_displayed_idr_at_ms = stalled.clean_anchor_observed_at_ms;

        stalled.recovery_fresh_anchor_recovered_at_ms = stalled.clean_anchor_observed_at_ms;

        let output = owner.evaluate(&stalled);
        if output.state == VideoSchedulingOwnerState::SupplyStarved {
            final_output = Some(output);
            break;
        }
    }
    let output = final_output.expect("owner should surface host present stall");
    assert_eq!(output.state, VideoSchedulingOwnerState::SupplyStarved);
    assert_eq!(output.health, VideoHealthContract::Starved);
    let intent = output.recovery_intent.expect("host stall intent");
    assert_eq!(intent.source, RecoveryIntentSource::Supply);
    assert_eq!(intent.reason_label, "hostPresentStalled");
}

#[test]
fn stable_serving_with_frozen_present_epoch_and_stale_decode_surfaces_host_present_stall() {
    let mut owner = VideoSchedulingOwner::new();
    let _ = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal::default(),
        Some("recovering"),
        Some("frame-await-recovery-anchor"),
        Some("remoteTrackAttached"),
        Some(10_000),
        300.0,
        1,
    ));

    let mut base = input(
        ConnectionLifecycleStateFact::Connected,
        None,
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("normal".to_string()),
            no_pending_streak: Some(1),
            present_age_ms: Some(12.0),
            decode_age_ms: Some(9.0),
            video_renderer_stalled: false,
            host_display_tick_epoch: Some(10),
            host_frame_present_epoch: Some(3),
            host_cadence_phase: Some("steady".to_string()),
            ..SchedulingDemandSignal::default()
        },
        Some("receiving"),
        Some("frame-complete-candidate"),
        Some("remoteTrackAttached"),
        Some(64_000),
        4_000.0,
        1,
    );
    base.clean_anchor_epoch = Some(1);
    base.clean_anchor_observed_at_ms = Some(3_900.0);
    base.clean_anchor_source_event = Some("displayed-idr".to_string());
    base.recovery_displayed_idr_at_ms = base.clean_anchor_observed_at_ms;
    base.recovery_fresh_anchor_recovered_at_ms = base.clean_anchor_observed_at_ms;

    base.latest_h264_bootstrap_ready = Some(false);
    base.latest_h264_bootstrap_reject_reason = Some("bootstrapMissingIdr".to_string());
    base.latest_h264_committed_sps_present = Some(true);
    base.latest_h264_committed_pps_present = Some(true);
    base.latest_h264_delta_continuation_ready = Some(true);
    base.latest_h264_observed_at_ms = Some(3_950.0);
    base.receiver_state = Some("waiting-keyframe".to_string());
    seed_ledger_display_recovery(&mut base);
    assert_eq!(
        owner.evaluate(&base).state,
        VideoSchedulingOwnerState::StableServing
    );

    let mut final_output = None;
    for step in 0..10 {
        let mut stalled = base.clone();
        stalled.demand = SchedulingDemandSignal {
            no_pending_pressure_level: Some("normal".to_string()),
            no_pending_streak: Some(1),
            // 高于 degraded_present_age_ms，避免 host_display_hold 清零 stall streak。
            present_age_ms: Some(250.0),
            decode_age_ms: Some(2_800.0),
            video_renderer_stalled: true,
            host_display_tick_epoch: Some(20 + step),
            host_frame_present_epoch: Some(3),
            host_cadence_phase: Some("steady".to_string()),
            ..SchedulingDemandSignal::default()
        };
        stalled.observed_at_ms = 4_100.0 + step as f64 * 100.0;
        let output = owner.evaluate(&stalled);
        if output
            .recovery_intent
            .as_ref()
            .is_some_and(|intent| intent.reason_label == "hostPresentStalled")
        {
            final_output = Some(output);
            break;
        }
    }
    let output = final_output.expect("stable serving should surface host present stall");
    let intent = output
        .recovery_intent
        .expect("host stall intent from stable serving");
    assert_eq!(intent.reason_label, "hostPresentStalled");
}

#[test]
fn chronic_low_present_throughput_suppresses_host_present_stall() {
    let mut owner = VideoSchedulingOwner::new();
    owner.state = VideoSchedulingOwnerState::RebuildingSupply;
    let mut stalled = input(
        ConnectionLifecycleStateFact::Connected,
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal {
            present_age_ms: Some(48.0),
            decode_age_ms: Some(24.0),
            smoothed_present_fps: Some(10.0),
            smoothed_decode_fps: Some(31.0),
            host_display_tick_epoch: Some(200),
            host_frame_present_epoch: Some(180),
            host_cadence_phase: Some("steady".to_string()),
            ..SchedulingDemandSignal::default()
        },
        Some("receiving"),
        Some("frame-complete-candidate"),
        Some("remoteTrackAttached"),
        Some(64_000),
        4_000.0,
        1,
    );
    for step in 0..8 {
        stalled.demand.host_display_tick_epoch = Some(201 + step);
        let output = owner.evaluate(&stalled);
        assert_ne!(
            output.state,
            VideoSchedulingOwnerState::SupplyStarved,
            "display throughput bottleneck must not surface hostPresentStalled starved at step {step}"
        );
        owner.state = output.state;
    }
}

#[test]
fn transient_anchor_noise_with_clean_anchor_and_delta_ready_does_not_stick_recovery() {
    let mut owner = VideoSchedulingOwner::new();
    let _ = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal::default(),
        Some("waiting-keyframe"),
        Some("frame-await-recovery-anchor"),
        Some("remoteTrackAttached"),
        Some(10_000),
        300.0,
        1,
    ));

    let mut recoverable = input(
        ConnectionLifecycleStateFact::Connected,
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("normal".to_string()),
            no_pending_streak: Some(0),
            present_age_ms: Some(12.0),
            decode_age_ms: Some(9.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("receiving"),
        Some("nack-observation"),
        Some("remoteTrackAttached"),
        Some(64_000),
        340.0,
        1,
    );
    recoverable.clean_anchor_epoch = Some(1);
    recoverable.clean_anchor_observed_at_ms = Some(338.0);
    recoverable.clean_anchor_source_event = Some("displayed-idr".to_string());

    recoverable.recovery_displayed_idr_at_ms = recoverable.clean_anchor_observed_at_ms;

    recoverable.recovery_fresh_anchor_recovered_at_ms = recoverable.clean_anchor_observed_at_ms;
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

    seed_ledger_display_recovery(&mut recoverable);
    let output = owner.evaluate(&recoverable);
    assert_eq!(output.state, VideoSchedulingOwnerState::StableServing);
    assert_eq!(output.health, VideoHealthContract::Stable);
}

#[test]
fn continuation_only_after_clean_anchor_grace_does_not_reenter_rebuilding_without_receiver_waiting()
{
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
        Some("receiving"),
        Some("frame-complete-candidate"),
        Some("remoteTrackAttached"),
        Some(96_000),
        340.0,
        1,
    );
    stable.clean_anchor_epoch = Some(1);
    stable.clean_anchor_observed_at_ms = Some(338.0);
    stable.clean_anchor_source_event = Some("displayed-idr".to_string());

    stable.recovery_displayed_idr_at_ms = stable.clean_anchor_observed_at_ms;

    stable.recovery_fresh_anchor_recovered_at_ms = stable.clean_anchor_observed_at_ms;
    let initial = owner.evaluate(&stable);
    assert_eq!(initial.state, VideoSchedulingOwnerState::Priming);
    let settled = owner.evaluate(&stable);
    assert_eq!(settled.state, VideoSchedulingOwnerState::StableServing);

    let mut degraded = stable.clone();
    degraded.observed_at_ms = 720.0;
    degraded.clean_anchor_observed_at_ms = Some(338.0);
    degraded.recovery_displayed_idr_at_ms = Some(338.0);
    degraded.recovery_fresh_anchor_recovered_at_ms = Some(338.0);
    degraded.latest_h264_bootstrap_ready = Some(false);
    degraded.latest_h264_bootstrap_reject_reason = Some("NonIdrVcl".to_string());
    degraded.latest_h264_committed_sps_present = Some(true);
    degraded.latest_h264_committed_pps_present = Some(true);
    degraded.latest_h264_delta_continuation_ready = Some(true);
    degraded.latest_h264_observed_at_ms = Some(719.0);

    let output = owner.evaluate(&degraded);
    assert_eq!(output.state, VideoSchedulingOwnerState::StableServing);
    assert_eq!(output.health, VideoHealthContract::Stable);
    assert!(output.recovery_intent.is_none());
}

#[test]
fn sustained_critical_pressure_without_clean_anchor_keeps_owner_in_rebuilding_supply() {
    let mut owner = VideoSchedulingOwner::new();
    let _ = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal::default(),
        Some("recovering"),
        Some("frame-await-recovery-anchor"),
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
        Some("receiving"),
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
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal::default(),
        Some("recovering"),
        Some("frame-await-recovery-anchor"),
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
        Some("receiving"),
        Some("frame-observed"),
        Some("remoteTrackAttached"),
        Some(42_000),
        580.0,
        1,
    );
    ready.clean_anchor_epoch = Some(1);
    ready.clean_anchor_observed_at_ms = Some(579.0);
    ready.clean_anchor_source_event = Some("displayed-idr".to_string());

    ready.recovery_displayed_idr_at_ms = ready.clean_anchor_observed_at_ms;

    ready.recovery_fresh_anchor_recovered_at_ms = ready.clean_anchor_observed_at_ms;

    seed_ledger_display_recovery(&mut ready);
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
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal::default(),
        Some("recovering"),
        Some("frame-await-recovery-anchor"),
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
            host_frame_present_epoch: Some(0),
            host_mailbox_enqueue_count_total: Some(0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("receiving"),
        Some("frame-complete-candidate"),
        Some("remoteTrackAttached"),
        Some(42_000),
        640.0,
        1,
    );
    recoverable.clean_anchor_epoch = Some(1);
    recoverable.clean_anchor_observed_at_ms = Some(639.0);
    recoverable.clean_anchor_source_event = Some("displayed-idr".to_string());

    recoverable.recovery_displayed_idr_at_ms = recoverable.clean_anchor_observed_at_ms;

    recoverable.recovery_fresh_anchor_recovered_at_ms = recoverable.clean_anchor_observed_at_ms;

    seed_ledger_display_recovery(&mut recoverable);
    let output = owner.evaluate(&recoverable);
    assert_eq!(output.state, VideoSchedulingOwnerState::StableServing);
    assert_eq!(output.health, VideoHealthContract::Stable);
}

#[test]
fn media_continuity_without_decode_or_present_feedback_can_exit_rebuilding_supply_as_degraded() {
    let mut owner = VideoSchedulingOwner::new();
    let _ = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal::default(),
        Some("recovering"),
        Some("frame-await-recovery-anchor"),
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
        Some("receiving"),
        Some("frame-complete-candidate"),
        Some("remoteTrackAttached"),
        Some(48_000),
        720.0,
        8,
    );
    recoverable.clean_anchor_epoch = Some(8);
    recoverable.clean_anchor_observed_at_ms = Some(719.0);
    recoverable.clean_anchor_source_event = Some("displayed-idr".to_string());

    recoverable.recovery_displayed_idr_at_ms = recoverable.clean_anchor_observed_at_ms;

    recoverable.recovery_fresh_anchor_recovered_at_ms = recoverable.clean_anchor_observed_at_ms;
    recoverable.latest_h264_bootstrap_ready = Some(false);
    recoverable.latest_h264_bootstrap_reject_reason = Some("NonIdrVcl".to_string());
    recoverable.latest_h264_committed_sps_present = Some(true);
    recoverable.latest_h264_committed_pps_present = Some(true);
    recoverable.latest_h264_delta_continuation_ready = Some(true);
    recoverable.latest_h264_observed_at_ms = Some(719.0);

    seed_ledger_display_recovery(&mut recoverable);
    let output = owner.evaluate(&recoverable);
    assert_eq!(output.state, VideoSchedulingOwnerState::StableServing);
    assert_eq!(output.health, VideoHealthContract::Stable);
    assert!(output.recovery_intent.is_none());
}

#[test]
fn missing_media_continuity_metadata_keeps_rebuilding_supply_without_decode_or_present_feedback() {
    let mut owner = VideoSchedulingOwner::new();
    let _ = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal::default(),
        Some("recovering"),
        Some("frame-await-recovery-anchor"),
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
        Some("receiving"),
        Some("frame-complete-candidate"),
        Some("remoteTrackAttached"),
        Some(48_000),
        780.0,
        9,
    );
    blocked.clean_anchor_epoch = Some(9);
    blocked.clean_anchor_observed_at_ms = Some(779.0);
    blocked.clean_anchor_source_event = Some("displayed-idr".to_string());

    blocked.recovery_displayed_idr_at_ms = blocked.clean_anchor_observed_at_ms;

    blocked.recovery_fresh_anchor_recovered_at_ms = blocked.clean_anchor_observed_at_ms;
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
        Some("receiving"),
        Some("frame-observed"),
        Some("remoteTrackAttached"),
        Some(42_000),
        1_000.0,
        1,
    );
    stable.clean_anchor_epoch = Some(1);
    stable.clean_anchor_observed_at_ms = Some(999.0);
    stable.clean_anchor_source_event = Some("displayed-idr".to_string());

    stable.recovery_displayed_idr_at_ms = stable.clean_anchor_observed_at_ms;

    stable.recovery_fresh_anchor_recovered_at_ms = stable.clean_anchor_observed_at_ms;
    assert_eq!(
        owner.evaluate(&stable).state,
        VideoSchedulingOwnerState::Priming
    );

    let mut stable_again = stable.clone();
    stable_again.observed_at_ms = 1_020.0;
    stable_again.clean_anchor_observed_at_ms = Some(1_019.0);
    stable_again.recovery_displayed_idr_at_ms = Some(1_019.0);
    stable_again.recovery_fresh_anchor_recovered_at_ms = Some(1_019.0);
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
        Some("receiving"),
        Some("frame-observed"),
        Some("remoteTrackAttached"),
        Some(60_000),
        1_100.0,
        1,
    );
    burst.clean_anchor_epoch = Some(1);
    burst.clean_anchor_observed_at_ms = Some(1_095.0);
    burst.clean_anchor_source_event = Some("displayed-idr".to_string());

    burst.recovery_displayed_idr_at_ms = burst.clean_anchor_observed_at_ms;

    burst.recovery_fresh_anchor_recovered_at_ms = burst.clean_anchor_observed_at_ms;
    let absorbed = owner.evaluate(&burst);
    assert_eq!(absorbed.state, VideoSchedulingOwnerState::DegradedServing);
    assert_eq!(absorbed.health, VideoHealthContract::Stable);
    assert!(absorbed.recovery_intent.is_none());

    let mut sustained = burst.clone();
    sustained.observed_at_ms = 1_360.0;
    sustained.clean_anchor_observed_at_ms = Some(1_355.0);
    sustained.recovery_displayed_idr_at_ms = Some(1_355.0);
    sustained.recovery_fresh_anchor_recovered_at_ms = Some(1_355.0);
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
        Some("receiving"),
        Some("frame-observed"),
        Some("remoteTrackAttached"),
        Some(48_000),
        2_000.0,
        4,
    );
    stable.clean_anchor_epoch = Some(4);
    stable.clean_anchor_observed_at_ms = Some(1_998.0);
    stable.clean_anchor_source_event = Some("displayed-idr".to_string());

    stable.recovery_displayed_idr_at_ms = stable.clean_anchor_observed_at_ms;

    stable.recovery_fresh_anchor_recovered_at_ms = stable.clean_anchor_observed_at_ms;
    assert_eq!(
        owner.evaluate(&stable).state,
        VideoSchedulingOwnerState::Priming
    );

    let mut settled = stable.clone();
    settled.observed_at_ms = 2_020.0;
    settled.clean_anchor_observed_at_ms = Some(2_018.0);
    settled.recovery_displayed_idr_at_ms = Some(2_018.0);
    settled.recovery_fresh_anchor_recovered_at_ms = Some(2_018.0);
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
        Some("receiving"),
        Some("frame-complete-candidate"),
        Some("remoteTrackAttached"),
        Some(72_000),
        2_120.0,
        4,
    );
    burst.clean_anchor_epoch = Some(4);
    burst.clean_anchor_observed_at_ms = Some(2_118.0);
    burst.clean_anchor_source_event = Some("displayed-idr".to_string());

    burst.recovery_displayed_idr_at_ms = burst.clean_anchor_observed_at_ms;

    burst.recovery_fresh_anchor_recovered_at_ms = burst.clean_anchor_observed_at_ms;
    let absorbed = owner.evaluate(&burst);
    assert_eq!(absorbed.state, VideoSchedulingOwnerState::DegradedServing);
    assert!(absorbed.recovery_intent.is_none());

    let mut feedback_gap = burst.clone();
    feedback_gap.observed_at_ms = 2_200.0;
    feedback_gap.clean_anchor_observed_at_ms = Some(2_198.0);
    feedback_gap.recovery_displayed_idr_at_ms = Some(2_198.0);
    feedback_gap.recovery_fresh_anchor_recovered_at_ms = Some(2_198.0);
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
    recovered.recovery_displayed_idr_at_ms = Some(2_212.0);
    recovered.recovery_fresh_anchor_recovered_at_ms = Some(2_212.0);
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
        Some("receiving"),
        Some("frame-complete-candidate"),
        Some("remoteTrackAttached"),
        Some(64_000),
        2_000.0,
        1,
    );
    stable.clean_anchor_epoch = Some(1);
    stable.clean_anchor_observed_at_ms = Some(1_999.0);
    stable.clean_anchor_source_event = Some("displayed-idr".to_string());

    stable.recovery_displayed_idr_at_ms = stable.clean_anchor_observed_at_ms;

    stable.recovery_fresh_anchor_recovered_at_ms = stable.clean_anchor_observed_at_ms;
    assert_eq!(
        owner.evaluate(&stable).state,
        VideoSchedulingOwnerState::Priming
    );

    let mut stable_again = stable.clone();
    stable_again.observed_at_ms = 2_020.0;
    stable_again.clean_anchor_observed_at_ms = Some(2_019.0);
    stable_again.recovery_displayed_idr_at_ms = Some(2_019.0);
    stable_again.recovery_fresh_anchor_recovered_at_ms = Some(2_019.0);
    assert_eq!(
        owner.evaluate(&stable_again).state,
        VideoSchedulingOwnerState::StableServing
    );

    let mut degraded_pressure = stable.clone();
    degraded_pressure.observed_at_ms = 2_100.0;
    degraded_pressure.clean_anchor_observed_at_ms = Some(2_099.0);
    degraded_pressure.recovery_displayed_idr_at_ms = Some(2_099.0);
    degraded_pressure.recovery_fresh_anchor_recovered_at_ms = Some(2_099.0);
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
    critical_pressure.recovery_displayed_idr_at_ms = Some(2_219.0);
    critical_pressure.recovery_fresh_anchor_recovered_at_ms = Some(2_219.0);
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
    after_switch_window.recovery_displayed_idr_at_ms = Some(2_359.0);
    after_switch_window.recovery_fresh_anchor_recovered_at_ms = Some(2_359.0);
    let still_held = owner.evaluate(&after_switch_window);
    assert_eq!(still_held.state, VideoSchedulingOwnerState::DegradedServing);
    assert!(still_held.recovery_intent.is_none());

    let mut confirmed = critical_pressure.clone();
    // 须超过 DISPLAY_SUPPLY_STARVED_CONFIRM_MS（与 owner 内常量对齐）
    confirmed.observed_at_ms = 2_520.0;
    confirmed.clean_anchor_observed_at_ms = Some(2_519.0);
    confirmed.recovery_displayed_idr_at_ms = Some(2_519.0);
    confirmed.recovery_fresh_anchor_recovered_at_ms = Some(2_519.0);
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
        Some("receiving"),
        Some("frame-complete-candidate"),
        Some("remoteTrackAttached"),
        Some(64_000),
        3_000.0,
        1,
    );
    stable.clean_anchor_epoch = Some(1);
    stable.clean_anchor_observed_at_ms = Some(2_999.0);
    stable.clean_anchor_source_event = Some("displayed-idr".to_string());

    stable.recovery_displayed_idr_at_ms = stable.clean_anchor_observed_at_ms;

    stable.recovery_fresh_anchor_recovered_at_ms = stable.clean_anchor_observed_at_ms;
    assert_eq!(
        owner.evaluate(&stable).state,
        VideoSchedulingOwnerState::Priming
    );

    let mut stable_again = stable.clone();
    stable_again.observed_at_ms = 3_020.0;
    stable_again.clean_anchor_observed_at_ms = Some(3_019.0);
    stable_again.recovery_displayed_idr_at_ms = Some(3_019.0);
    stable_again.recovery_fresh_anchor_recovered_at_ms = Some(3_019.0);
    assert_eq!(
        owner.evaluate(&stable_again).state,
        VideoSchedulingOwnerState::StableServing
    );

    let mut critical = stable.clone();
    critical.observed_at_ms = 3_100.0;
    critical.clean_anchor_observed_at_ms = Some(3_099.0);
    critical.recovery_displayed_idr_at_ms = Some(3_099.0);
    critical.recovery_fresh_anchor_recovered_at_ms = Some(3_099.0);
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
    recovered.recovery_displayed_idr_at_ms = Some(3_159.0);
    recovered.recovery_fresh_anchor_recovered_at_ms = Some(3_159.0);
    let back_to_stable = owner.evaluate(&recovered);
    assert_eq!(
        back_to_stable.state,
        VideoSchedulingOwnerState::StableServing
    );
    assert!(back_to_stable.recovery_intent.is_none());

    let mut critical_again = critical.clone();
    critical_again.observed_at_ms = 3_220.0;
    critical_again.clean_anchor_observed_at_ms = Some(3_219.0);
    critical_again.recovery_displayed_idr_at_ms = Some(3_219.0);
    critical_again.recovery_fresh_anchor_recovered_at_ms = Some(3_219.0);
    let restarted_hold = owner.evaluate(&critical_again);
    assert_eq!(
        restarted_hold.state,
        VideoSchedulingOwnerState::DegradedServing
    );
    assert!(restarted_hold.recovery_intent.is_none());

    let mut before_confirm = critical_again.clone();
    before_confirm.observed_at_ms = 3_360.0;
    before_confirm.clean_anchor_observed_at_ms = Some(3_359.0);
    before_confirm.recovery_displayed_idr_at_ms = Some(3_359.0);
    before_confirm.recovery_fresh_anchor_recovered_at_ms = Some(3_359.0);
    let still_held = owner.evaluate(&before_confirm);
    assert_eq!(still_held.state, VideoSchedulingOwnerState::DegradedServing);
    assert!(still_held.recovery_intent.is_none());

    let mut confirmed = critical_again.clone();
    confirmed.observed_at_ms = 3_520.0;
    confirmed.clean_anchor_observed_at_ms = Some(3_519.0);
    confirmed.recovery_displayed_idr_at_ms = Some(3_519.0);
    confirmed.recovery_fresh_anchor_recovered_at_ms = Some(3_519.0);
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
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal::default(),
        Some("recovering"),
        Some("frame-await-recovery-anchor"),
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
        Some("receiving"),
        Some("frame-await-recovery-anchor"),
        Some("remoteTrackAttached"),
        Some(36_000),
        540.0,
        1,
    );
    ready.clean_anchor_epoch = Some(1);
    ready.clean_anchor_observed_at_ms = Some(538.0);
    ready.clean_anchor_source_event = Some("displayed-idr".to_string());

    ready.recovery_displayed_idr_at_ms = ready.clean_anchor_observed_at_ms;

    ready.recovery_fresh_anchor_recovered_at_ms = ready.clean_anchor_observed_at_ms;
    ready.display_supply_thresholds = home_thresholds();
    seed_ledger_display_recovery(&mut ready);
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
        Some("receiving"),
        Some("frame-observed"),
        Some("remoteTrackAttached"),
        Some(10_000),
        620.0,
        1,
    ));
    assert_eq!(connected.state, VideoSchedulingOwnerState::Priming);
    let disconnected = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Closed,
        Some("receiverWaitingKeyframe"),
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
            host_frame_present_epoch: Some(0),
            host_cadence_phase: Some("priming".to_string()),
            host_mailbox_enqueue_count_total: Some(0),
            ..SchedulingDemandSignal::default()
        },
        Some("receiving"),
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
            host_frame_present_epoch: Some(0),
            host_cadence_phase: Some("priming".to_string()),
            host_mailbox_enqueue_count_total: Some(0),
            ..SchedulingDemandSignal::default()
        },
        Some("receiving"),
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
            host_frame_present_epoch: Some(0),
            host_cadence_phase: Some("priming".to_string()),
            host_mailbox_enqueue_count_total: Some(0),
            ..SchedulingDemandSignal::default()
        },
        Some("receiving"),
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
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("critical".to_string()),
            no_pending_streak: Some(180),
            present_age_ms: None,
            decode_age_ms: None,
            video_renderer_stalled: false,
            host_display_tick_epoch: Some(120),
            host_frame_present_epoch: Some(0),
            host_cadence_phase: Some("priming".to_string()),
            host_mailbox_enqueue_count_total: Some(0),
            ..SchedulingDemandSignal::default()
        },
        Some("recovering"),
        Some("frame-inspection-rejected-await-anchor"),
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
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("critical".to_string()),
            no_pending_streak: Some(180),
            present_age_ms: None,
            decode_age_ms: None,
            video_renderer_stalled: false,
            host_display_tick_epoch: Some(120),
            host_frame_present_epoch: Some(0),
            host_cadence_phase: Some("priming".to_string()),
            host_mailbox_enqueue_count_total: Some(0),
            ..SchedulingDemandSignal::default()
        },
        Some("recovering"),
        Some("frame-await-recovery-anchor"),
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
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("critical".to_string()),
            no_pending_streak: Some(180),
            present_age_ms: None,
            decode_age_ms: None,
            video_renderer_stalled: false,
            host_display_tick_epoch: Some(120),
            host_frame_present_epoch: Some(0),
            host_cadence_phase: Some("priming".to_string()),
            host_mailbox_enqueue_count_total: Some(0),
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
        Some("receiving"),
        Some("frame-complete-candidate"),
        Some("remoteTrackAttached"),
        Some(120_000),
        1_000.0,
        5,
    );
    priming.clean_anchor_epoch = Some(5);
    priming.clean_anchor_observed_at_ms = Some(999.0);
    priming.clean_anchor_source_event = Some("displayed-idr".to_string());

    priming.recovery_displayed_idr_at_ms = priming.clean_anchor_observed_at_ms;

    priming.recovery_fresh_anchor_recovered_at_ms = priming.clean_anchor_observed_at_ms;
    seed_ledger_display_recovery(&mut priming);
    assert_eq!(
        owner.evaluate(&priming).state,
        VideoSchedulingOwnerState::Priming
    );

    let stable = owner.evaluate(&priming);
    assert_eq!(stable.state, VideoSchedulingOwnerState::StableServing);

    let mut weak_probe = input(
        ConnectionLifecycleStateFact::Connected,
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("critical".to_string()),
            no_pending_streak: Some(220),
            present_age_ms: Some(18.0),
            decode_age_ms: Some(14.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("receiving"),
        Some("frame-await-recovery-anchor"),
        Some("remoteTrackAttached"),
        Some(128_000),
        1_060.0,
        5,
    );
    weak_probe.clean_anchor_epoch = Some(5);
    weak_probe.clean_anchor_observed_at_ms = Some(1_058.0);
    weak_probe.clean_anchor_source_event = Some("displayed-idr".to_string());

    weak_probe.recovery_displayed_idr_at_ms = weak_probe.clean_anchor_observed_at_ms;

    weak_probe.recovery_fresh_anchor_recovered_at_ms = weak_probe.clean_anchor_observed_at_ms;

    seed_ledger_display_recovery(&mut weak_probe);
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
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("critical".to_string()),
            no_pending_streak: Some(220),
            present_age_ms: Some(420.0),
            decode_age_ms: Some(380.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("recovering"),
        Some("frame-await-recovery-anchor"),
        Some("remoteTrackAttached"),
        Some(120_000),
        1_400.0,
        7,
    );
    hard_probe.latest_anchor_candidate_ledger = Some(crate::XbxEngineAnchorCandidateLedger {
        state: crate::XbxEngineAnchorCandidateState::Rejected,
        source_event: "frame-await-recovery-anchor".to_string(),
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
    assert_eq!(intent.reason_label, "receiverWaitingKeyframe");
}

#[test]
fn startup_bootstrap_reject_source_without_persisted_h264_state_stays_priming() {
    let mut owner = VideoSchedulingOwner::new();
    let bootstrap_pending = input(
        ConnectionLifecycleStateFact::Connected,
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("critical".to_string()),
            no_pending_streak: Some(180),
            present_age_ms: None,
            decode_age_ms: None,
            video_renderer_stalled: false,
            host_display_tick_epoch: Some(120),
            host_frame_present_epoch: Some(0),
            host_cadence_phase: Some("priming".to_string()),
            host_mailbox_enqueue_count_total: Some(0),
            ..SchedulingDemandSignal::default()
        },
        Some("recovering"),
        Some("frame-inspection-rejected-await-anchor"),
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
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("critical".to_string()),
            no_pending_streak: Some(180),
            present_age_ms: Some(420.0),
            decode_age_ms: Some(380.0),
            video_renderer_stalled: false,
            host_display_tick_epoch: Some(360),
            host_frame_present_epoch: Some(8),
            host_cadence_phase: Some("steady".to_string()),
            host_mailbox_enqueue_count_total: Some(64),
            ..SchedulingDemandSignal::default()
        },
        Some("recovering"),
        Some("frame-inspection-rejected-await-anchor"),
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
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal::default(),
        Some("recovering"),
        Some("frame-await-recovery-anchor"),
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
        Some("receiving"),
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
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal::default(),
        Some("recovering"),
        Some("frame-await-recovery-anchor"),
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
        Some("receiving"),
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
        Some("receiving"),
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
        Some("receiving"),
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
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal::default(),
        Some("recovering"),
        Some("frame-await-recovery-anchor"),
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
        Some("receiving"),
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
        Some("receiving"),
        Some("frame-observed"),
        Some("remoteTrackAttached"),
        Some(120_000),
        1230.0,
        1,
    );
    ready.clean_anchor_epoch = Some(1);
    ready.clean_anchor_observed_at_ms = Some(1225.0);
    ready.clean_anchor_source_event = Some("displayed-idr".to_string());

    ready.recovery_displayed_idr_at_ms = ready.clean_anchor_observed_at_ms;

    ready.recovery_fresh_anchor_recovered_at_ms = ready.clean_anchor_observed_at_ms;
    seed_ledger_display_recovery(&mut ready);
    let stable = owner.evaluate(&ready);
    assert_eq!(stable.state, VideoSchedulingOwnerState::StableServing);
    assert_eq!(stable.health, VideoHealthContract::Stable);
}

#[test]
fn rebuilding_supply_cannot_close_to_stable_without_explicit_healthy_chain() {
    let mut owner = VideoSchedulingOwner::new();
    let _ = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal::default(),
        Some("recovering"),
        Some("frame-await-recovery-anchor"),
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
    ready.clean_anchor_source_event = Some("displayed-idr".to_string());

    ready.recovery_displayed_idr_at_ms = ready.clean_anchor_observed_at_ms;

    ready.recovery_fresh_anchor_recovered_at_ms = ready.clean_anchor_observed_at_ms;
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
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal::default(),
        Some("recovering"),
        Some("frame-await-recovery-anchor"),
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
        Some("receiving"),
        Some("frame-observed"),
        Some("remoteTrackAttached"),
        Some(20_000),
        1310.0,
        3,
    );
    stale_anchor.clean_anchor_epoch = Some(2);
    stale_anchor.clean_anchor_observed_at_ms = Some(300.0);
    stale_anchor.clean_anchor_source_event = Some("displayed-idr".to_string());
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
        Some("receiving"),
        Some("frame-observed"),
        Some("remoteTrackAttached"),
        Some(90_000),
        3_000.0,
        5,
    );
    stable.clean_anchor_epoch = Some(5);
    stable.clean_anchor_observed_at_ms = Some(2_999.0);
    stable.clean_anchor_source_event = Some("displayed-idr".to_string());

    stable.recovery_displayed_idr_at_ms = stable.clean_anchor_observed_at_ms;

    stable.recovery_fresh_anchor_recovered_at_ms = stable.clean_anchor_observed_at_ms;
    seed_ledger_display_recovery(&mut stable);
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
        Some("receiving"),
        Some("frame-await-recovery-anchor"),
        Some("remoteTrackAttached"),
        Some(120_000),
        3_020.0,
        5,
    );
    noisy.clean_anchor_epoch = Some(5);
    noisy.clean_anchor_observed_at_ms = Some(3_018.0);
    noisy.clean_anchor_source_event = Some("displayed-idr".to_string());

    noisy.recovery_displayed_idr_at_ms = noisy.clean_anchor_observed_at_ms;

    noisy.recovery_fresh_anchor_recovered_at_ms = noisy.clean_anchor_observed_at_ms;

    seed_ledger_display_recovery(&mut noisy);
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
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal::default(),
        Some("recovering"),
        Some("frame-await-recovery-anchor"),
        Some("remoteTrackAttached"),
        Some(64_000),
        1_000.0,
        7,
    ));

    let mut pending = input(
        ConnectionLifecycleStateFact::Connected,
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("critical".to_string()),
            no_pending_streak: Some(180),
            present_age_ms: Some(420.0),
            decode_age_ms: Some(380.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("recovering"),
        Some("frame-await-recovery-anchor"),
        Some("remoteTrackAttached"),
        Some(120_000),
        1_280.0,
        7,
    );
    pending.clean_anchor_epoch = Some(7);
    pending.clean_anchor_observed_at_ms = Some(1_250.0);
    pending.clean_anchor_source_event = Some("displayed-idr".to_string());

    pending.recovery_displayed_idr_at_ms = pending.clean_anchor_observed_at_ms;

    pending.recovery_fresh_anchor_recovered_at_ms = pending.clean_anchor_observed_at_ms;
    pending.latest_anchor_candidate_ledger = Some(crate::XbxEngineAnchorCandidateLedger {
        recovery_epoch: 7,
        frame_rtp_timestamp: Some(7_001),
        state: crate::XbxEngineAnchorCandidateState::SubmittedCleanAnchor,
        source_event: "displayed-idr".to_string(),
        failure_reason: None,
        observed_at_ms: 1_255.0,
    });

    let output = owner.evaluate(&pending);
    assert_eq!(output.state, VideoSchedulingOwnerState::RebuildingSupply);
    let intent = output.recovery_intent.expect("bootstrap in flight intent");
    assert_eq!(intent.reason_label, "receiverWaitingKeyframe");
}

#[test]
fn submitted_clean_anchor_within_sustaining_phase_keeps_bootstrap_in_flight_reason() {
    let mut owner = VideoSchedulingOwner::new();
    let _ = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal::default(),
        Some("recovering"),
        Some("frame-await-recovery-anchor"),
        Some("remoteTrackAttached"),
        Some(64_000),
        1_000.0,
        7,
    ));

    let mut pending = input(
        ConnectionLifecycleStateFact::Connected,
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("critical".to_string()),
            no_pending_streak: Some(180),
            present_age_ms: Some(420.0),
            decode_age_ms: Some(380.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("recovering"),
        Some("frame-await-recovery-anchor"),
        Some("remoteTrackAttached"),
        Some(120_000),
        1_650.0,
        7,
    );
    pending.clean_anchor_epoch = Some(7);
    pending.clean_anchor_observed_at_ms = Some(1_250.0);
    pending.clean_anchor_source_event = Some("displayed-idr".to_string());

    pending.recovery_displayed_idr_at_ms = pending.clean_anchor_observed_at_ms;

    pending.recovery_fresh_anchor_recovered_at_ms = pending.clean_anchor_observed_at_ms;
    pending.latest_anchor_candidate_ledger = Some(crate::XbxEngineAnchorCandidateLedger {
        recovery_epoch: 7,
        frame_rtp_timestamp: Some(7_001),
        state: crate::XbxEngineAnchorCandidateState::SubmittedCleanAnchor,
        source_event: "displayed-idr".to_string(),
        failure_reason: None,
        observed_at_ms: 1_255.0,
    });

    let output = owner.evaluate(&pending);
    assert_eq!(output.state, VideoSchedulingOwnerState::RebuildingSupply);
    let intent = output.recovery_intent.expect("transport await intent");
    assert_eq!(intent.reason_label, "receiverWaitingKeyframe");
}

#[test]
fn sustaining_recovery_with_serviceable_output_exits_rebuilding_supply_as_degraded() {
    let mut owner = VideoSchedulingOwner::new();
    let _ = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal::default(),
        Some("recovering"),
        Some("frame-await-recovery-anchor"),
        Some("remoteTrackAttached"),
        Some(64_000),
        1_000.0,
        17,
    ));

    let mut sustaining = input(
        ConnectionLifecycleStateFact::Connected,
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("critical".to_string()),
            no_pending_streak: Some(180),
            present_age_ms: Some(420.0),
            decode_age_ms: Some(260.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("receiving"),
        Some("frame-observed"),
        Some("remoteTrackAttached"),
        Some(120_000),
        1_180.0,
        17,
    );
    sustaining.clean_anchor_epoch = Some(17);
    sustaining.clean_anchor_observed_at_ms = Some(1_120.0);
    sustaining.clean_anchor_source_event = Some("displayed-idr".to_string());

    sustaining.recovery_displayed_idr_at_ms = sustaining.clean_anchor_observed_at_ms;

    sustaining.recovery_fresh_anchor_recovered_at_ms = sustaining.clean_anchor_observed_at_ms;
    sustaining.latest_anchor_candidate_ledger = Some(crate::XbxEngineAnchorCandidateLedger {
        recovery_epoch: 17,
        frame_rtp_timestamp: Some(17_001),
        state: crate::XbxEngineAnchorCandidateState::SubmittedCleanAnchor,
        source_event: "displayed-idr".to_string(),
        failure_reason: None,
        observed_at_ms: 1_120.0,
    });

    seed_ledger_display_recovery(&mut sustaining);
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
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal::default(),
        Some("recovering"),
        Some("frame-await-recovery-anchor"),
        Some("remoteTrackAttached"),
        Some(64_000),
        1_000.0,
        7,
    ));

    let mut pending = input(
        ConnectionLifecycleStateFact::Connected,
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("critical".to_string()),
            no_pending_streak: Some(180),
            present_age_ms: Some(420.0),
            decode_age_ms: Some(380.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("waiting-keyframe"),
        Some("frame-await-recovery-anchor"),
        Some("remoteTrackAttached"),
        Some(120_000),
        1_650.0,
        7,
    );
    pending.clean_anchor_epoch = Some(7);
    pending.clean_anchor_observed_at_ms = Some(1_250.0);
    pending.clean_anchor_source_event = Some("displayed-idr".to_string());

    pending.recovery_displayed_idr_at_ms = pending.clean_anchor_observed_at_ms;

    pending.recovery_fresh_anchor_recovered_at_ms = pending.clean_anchor_observed_at_ms;
    pending.latest_anchor_candidate_ledger = Some(crate::XbxEngineAnchorCandidateLedger {
        recovery_epoch: 7,
        frame_rtp_timestamp: Some(7_001),
        state: crate::XbxEngineAnchorCandidateState::Rejected,
        source_event: "frame-await-recovery-anchor".to_string(),
        failure_reason: Some(crate::XbxEngineAnchorCandidateFailureReason::GapExpiredDeadline),
        observed_at_ms: 1_640.0,
    });

    let output = owner.evaluate(&pending);
    assert_eq!(output.state, VideoSchedulingOwnerState::RebuildingSupply);
    let intent = output.recovery_intent.expect("transport await intent");
    assert_eq!(intent.reason_label, "receiverWaitingKeyframe");
}

#[test]
fn sustaining_phase_stops_when_transport_await_reappears_after_clean_anchor() {
    let mut owner = VideoSchedulingOwner::new();
    let _ = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal::default(),
        Some("recovering"),
        Some("frame-await-recovery-anchor"),
        Some("remoteTrackAttached"),
        Some(64_000),
        1_000.0,
        7,
    ));

    let mut pending = input(
        ConnectionLifecycleStateFact::Connected,
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("critical".to_string()),
            no_pending_streak: Some(180),
            present_age_ms: Some(420.0),
            decode_age_ms: Some(380.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("recovering"),
        Some("frame-await-recovery-anchor"),
        Some("remoteTrackAttached"),
        Some(120_000),
        1_650.0,
        7,
    );
    pending.clean_anchor_epoch = Some(7);
    pending.clean_anchor_observed_at_ms = Some(1_250.0);
    pending.clean_anchor_source_event = Some("displayed-idr".to_string());

    pending.recovery_displayed_idr_at_ms = pending.clean_anchor_observed_at_ms;

    pending.recovery_fresh_anchor_recovered_at_ms = pending.clean_anchor_observed_at_ms;
    pending.latest_anchor_candidate_ledger = Some(crate::XbxEngineAnchorCandidateLedger {
        recovery_epoch: 7,
        frame_rtp_timestamp: Some(7_001),
        state: crate::XbxEngineAnchorCandidateState::SubmittedCleanAnchor,
        source_event: "displayed-idr".to_string(),
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
            frame_importance: Some("anchor".to_string()),
            budget_importance: Some("supply".to_string()),
            evidence_importance: Some("anchor".to_string()),
            gap_dependency_confidence: Some("bound".to_string()),
            observed_at_ms: 1_640.0,
        }),
        frame: None,
        chain: crate::XbxEngineVideoTimelineChainSnapshot {
            state: "waiting-keyframe".to_string(),
            reason: Some("receiverWaitingKeyframe".to_string()),
            chain_break_evidence: None,
            observed_at_ms: 1_640.0,
        },
        observed_at_ms: 1_640.0,
    });
    pending.receiver_state = Some("waiting-keyframe".to_string());

    let output = owner.evaluate(&pending);
    assert_eq!(output.state, VideoSchedulingOwnerState::RebuildingSupply);
    let intent = output.recovery_intent.expect("transport await intent");
    assert_eq!(intent.reason_label, "receiverWaitingKeyframe");
}

#[test]
fn clean_anchor_recovery_sustaining_uses_sustaining_label_without_h264_anchor_override() {
    let mut owner = VideoSchedulingOwner::new();
    let _ = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal::default(),
        Some("recovering"),
        Some("frame-await-recovery-anchor"),
        Some("remoteTrackAttached"),
        Some(64_000),
        1_000.0,
        7,
    ));

    let mut pending = input(
        ConnectionLifecycleStateFact::Connected,
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("critical".to_string()),
            no_pending_streak: Some(180),
            present_age_ms: Some(420.0),
            decode_age_ms: Some(380.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("recovering"),
        Some("gap-repair-in-flight"),
        Some("remoteTrackAttached"),
        Some(120_000),
        1_280.0,
        7,
    );
    pending.clean_anchor_epoch = Some(7);
    pending.clean_anchor_observed_at_ms = Some(1_250.0);
    pending.clean_anchor_source_event = Some("displayed-idr".to_string());

    pending.recovery_displayed_idr_at_ms = pending.clean_anchor_observed_at_ms;

    pending.recovery_fresh_anchor_recovered_at_ms = pending.clean_anchor_observed_at_ms;
    pending.latest_anchor_candidate_ledger = Some(crate::XbxEngineAnchorCandidateLedger {
        recovery_epoch: 7,
        frame_rtp_timestamp: Some(7_001),
        state: crate::XbxEngineAnchorCandidateState::SubmittedCleanAnchor,
        source_event: "displayed-idr".to_string(),
        failure_reason: None,
        observed_at_ms: 1_255.0,
    });
    pending.latest_h264_bootstrap_ready = Some(false);
    pending.latest_h264_bootstrap_reject_reason = Some("bootstrapMissingIdr".to_string());
    pending.latest_h264_committed_sps_present = Some(true);
    pending.latest_h264_committed_pps_present = Some(true);
    pending.latest_h264_delta_continuation_ready = Some(true);
    pending.latest_h264_observed_at_ms = Some(1_279.0);

    let output = owner.evaluate(&pending);
    assert_eq!(output.state, VideoSchedulingOwnerState::RebuildingSupply);
    let intent = output.recovery_intent.expect("transport await intent");
    assert_eq!(intent.reason_label, "receiverWaitingKeyframe");
}

#[test]
fn rebuilding_supply_cannot_close_by_clean_anchor_candidate_without_explicit_healthy_chain() {
    let mut owner = VideoSchedulingOwner::new();
    let _ = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal::default(),
        Some("recovering"),
        Some("frame-await-recovery-anchor"),
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
        Some("waiting-keyframe"),
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
        source_event: "displayed-idr".to_string(),
        failure_reason: None,
        observed_at_ms: 1309.0,
    });

    let not_ready = owner.evaluate(&ready);
    assert_eq!(not_ready.state, VideoSchedulingOwnerState::RebuildingSupply);
    assert_eq!(not_ready.health, VideoHealthContract::Recovering);
}

#[test]
fn rebuilding_supply_keeps_waiting_on_submitted_clean_anchor_candidate() {
    let mut owner = VideoSchedulingOwner::new();
    let _ = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal::default(),
        Some("recovering"),
        Some("frame-await-recovery-anchor"),
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
        Some("receiving"),
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
        source_event: "displayed-idr".to_string(),
        failure_reason: None,
        observed_at_ms: 2_000.0,
    });

    let waiting = owner.evaluate(&ready);
    assert_eq!(waiting.state, VideoSchedulingOwnerState::RebuildingSupply);
    assert_eq!(waiting.health, VideoHealthContract::Recovering);
}

#[test]
fn rebuilding_supply_allows_displayed_idr_fact() {
    let mut owner = VideoSchedulingOwner::new();
    let _ = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal::default(),
        Some("recovering"),
        Some("frame-await-recovery-anchor"),
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
        Some("receiving"),
        Some("frame-observed"),
        Some("remoteTrackAttached"),
        Some(150_000),
        2_700.0,
        10,
    );
    ready.clean_anchor_epoch = Some(10);
    ready.clean_anchor_observed_at_ms = Some(2_680.0);
    ready.clean_anchor_source_event = Some("displayed-idr".to_string());

    ready.recovery_displayed_idr_at_ms = ready.clean_anchor_observed_at_ms;

    ready.recovery_fresh_anchor_recovered_at_ms = ready.clean_anchor_observed_at_ms;
    ready.recovery_displayed_idr_at_ms = Some(2_680.0);
    ready.recovery_fresh_anchor_recovered_at_ms = Some(2_680.0);

    seed_ledger_display_recovery(&mut ready);
    let stable = owner.evaluate(&ready);
    assert_eq!(stable.state, VideoSchedulingOwnerState::StableServing);
    assert_eq!(stable.health, VideoHealthContract::Stable);
}

#[test]
fn displayed_idr_with_repairing_chain_and_stale_anchor_label_exits_rebuilding_supply() {
    let mut owner = VideoSchedulingOwner::new();
    let _ = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal::default(),
        Some("repairing"),
        Some("gap-repair-in-flight"),
        Some("remoteTrackAttached"),
        Some(40_000),
        2_000.0,
        10,
    ));

    let mut ready = input(
        ConnectionLifecycleStateFact::Connected,
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("normal".to_string()),
            no_pending_streak: Some(0),
            present_age_ms: Some(65.0),
            decode_age_ms: Some(24.0),
            host_cadence_phase: Some("steady".to_string()),
            host_frame_present_epoch: Some(3_424),
            host_display_tick_epoch: Some(8_000),
            ..SchedulingDemandSignal::default()
        },
        Some("repairing"),
        Some("frame-observed"),
        Some("remoteTrackAttached"),
        Some(150_000),
        2_700.0,
        10,
    );
    ready.recovery_displayed_idr_at_ms = Some(2_680.0);
    ready.recovery_fresh_anchor_recovered_at_ms = Some(2_680.0);

    seed_ledger_display_recovery(&mut ready);

    let stable = owner.evaluate(&ready);
    assert_eq!(stable.state, VideoSchedulingOwnerState::StableServing);
    assert_eq!(stable.health, VideoHealthContract::Stable);
    assert!(stable.recovery_intent.is_none());
}

#[test]
fn displayed_idr_with_waiting_keyframe_chain_exits_rebuilding_supply() {
    // 以下用例要求 decoder 参考链已同步，才允许 displayed-idr release transport-await。
    let mut owner = VideoSchedulingOwner::new();
    let _ = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal::default(),
        Some("waiting-keyframe"),
        Some("receiverWaitingKeyframe"),
        Some("remoteTrackAttached"),
        Some(40_000),
        2_000.0,
        10,
    ));

    let mut ready = input(
        ConnectionLifecycleStateFact::Connected,
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("normal".to_string()),
            no_pending_streak: Some(0),
            present_age_ms: Some(12.0),
            decode_age_ms: Some(24.0),
            host_cadence_phase: Some("steady".to_string()),
            host_frame_present_epoch: Some(3_424),
            host_display_tick_epoch: Some(8_000),
            ..SchedulingDemandSignal::default()
        },
        Some("waiting-keyframe"),
        Some("frame-inspection-rejected-await-anchor"),
        Some("remoteTrackAttached"),
        Some(150_000),
        2_700.0,
        10,
    );
    ready.recovery_displayed_idr_at_ms = Some(2_680.0);
    ready.recovery_fresh_anchor_recovered_at_ms = Some(2_680.0);

    seed_ledger_display_recovery(&mut ready);

    let stable = owner.evaluate(&ready);
    assert_eq!(stable.state, VideoSchedulingOwnerState::StableServing);
    assert_eq!(stable.health, VideoHealthContract::Stable);
    assert!(stable.recovery_intent.is_none());
}

#[test]
fn serving_wide_without_decoder_sync_does_not_release_to_stable_serving() {
    let mut owner = VideoSchedulingOwner::new();
    let _ = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal::default(),
        Some("waiting-keyframe"),
        Some("receiverWaitingKeyframe"),
        Some("remoteTrackAttached"),
        Some(40_000),
        2_000.0,
        10,
    ));

    let mut ready = input(
        ConnectionLifecycleStateFact::Connected,
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("normal".to_string()),
            no_pending_streak: Some(0),
            present_age_ms: Some(12.0),
            decode_age_ms: Some(24.0),
            host_cadence_phase: Some("steady".to_string()),
            host_frame_present_epoch: Some(3_424),
            host_display_tick_epoch: Some(8_000),
            ..SchedulingDemandSignal::default()
        },
        Some("waiting-keyframe"),
        Some("frame-inspection-rejected-await-anchor"),
        Some("remoteTrackAttached"),
        Some(150_000),
        2_700.0,
        10,
    );
    ready.recovery_displayed_idr_at_ms = Some(2_680.0);
    ready.recovery_fresh_anchor_recovered_at_ms = Some(2_680.0);

    let output = owner.evaluate(&ready);
    assert_ne!(output.state, VideoSchedulingOwnerState::StableServing);
}

#[test]
fn clean_anchor_recovery_can_close_on_transient_present_feedback_gap() {
    let mut owner = VideoSchedulingOwner::new();
    let _ = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal::default(),
        Some("recovering"),
        Some("frame-await-recovery-anchor"),
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
        Some("receiving"),
        Some("frame-observed"),
        Some("remoteTrackAttached"),
        Some(88_000),
        1_520.0,
        7,
    );
    missing_present.clean_anchor_epoch = Some(7);
    missing_present.clean_anchor_observed_at_ms = Some(1_518.0);
    missing_present.clean_anchor_source_event = Some("displayed-idr".to_string());

    missing_present.recovery_displayed_idr_at_ms = missing_present.clean_anchor_observed_at_ms;

    missing_present.recovery_fresh_anchor_recovered_at_ms =
        missing_present.clean_anchor_observed_at_ms;
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
        Some("receiving"),
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
            host_mailbox_enqueue_count_total: Some(10),
            host_mailbox_drop_count_total: Some(2),
            ..SchedulingDemandSignal::default()
        },
        Some("receiving"),
        Some("frame-observed"),
        Some("remoteTrackAttached"),
        Some(96_000),
        1_540.0,
        8,
    );
    recovering.clean_anchor_epoch = Some(8);
    recovering.clean_anchor_observed_at_ms = Some(1_538.0);
    recovering.clean_anchor_source_event = Some("displayed-idr".to_string());

    recovering.recovery_displayed_idr_at_ms = recovering.clean_anchor_observed_at_ms;

    recovering.recovery_fresh_anchor_recovered_at_ms = recovering.clean_anchor_observed_at_ms;

    seed_ledger_display_recovery(&mut recovering);
    let output = owner.evaluate(&recovering);
    assert_eq!(output.state, VideoSchedulingOwnerState::DegradedServing);
    assert_eq!(output.health, VideoHealthContract::Stable);
    assert!(output.recovery_intent.is_none());
}

#[test]
fn clean_anchor_recovery_ignores_shadow_renderer_stall_when_host_present_is_fresh() {
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
        Some("receiving"),
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
            video_renderer_stalled: true,
            host_mailbox_enqueue_count_total: Some(10),
            host_mailbox_drop_count_total: Some(0),
            host_display_tick_epoch: Some(24),
            host_frame_present_epoch: Some(18),
            ..SchedulingDemandSignal::default()
        },
        Some("receiving"),
        Some("frame-observed"),
        Some("remoteTrackAttached"),
        Some(96_000),
        1_540.0,
        8,
    );
    recovering.clean_anchor_epoch = Some(8);
    recovering.clean_anchor_observed_at_ms = Some(1_538.0);
    recovering.clean_anchor_source_event = Some("displayed-idr".to_string());

    recovering.recovery_displayed_idr_at_ms = recovering.clean_anchor_observed_at_ms;

    recovering.recovery_fresh_anchor_recovered_at_ms = recovering.clean_anchor_observed_at_ms;

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
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal::default(),
        Some("recovering"),
        Some("frame-await-recovery-anchor"),
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
        Some("receiving"),
        Some("frame-observed"),
        Some("remoteTrackAttached"),
        Some(88_000),
        1_520.0,
        7,
    );
    missing_present.clean_anchor_epoch = Some(7);
    missing_present.clean_anchor_observed_at_ms = Some(1_518.0);
    missing_present.clean_anchor_source_event = Some("displayed-idr".to_string());

    missing_present.recovery_displayed_idr_at_ms = missing_present.clean_anchor_observed_at_ms;

    missing_present.recovery_fresh_anchor_recovered_at_ms =
        missing_present.clean_anchor_observed_at_ms;
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
        Some("receiving"),
        Some("frame-observed"),
        Some("remoteTrackAttached"),
        Some(90_000),
        1_540.0,
        7,
    );
    fresh_present.clean_anchor_epoch = Some(7);
    fresh_present.clean_anchor_observed_at_ms = Some(1_538.0);
    fresh_present.clean_anchor_source_event = Some("displayed-idr".to_string());

    fresh_present.recovery_displayed_idr_at_ms = fresh_present.clean_anchor_observed_at_ms;

    fresh_present.recovery_fresh_anchor_recovered_at_ms = fresh_present.clean_anchor_observed_at_ms;
    seed_ledger_display_recovery(&mut fresh_present);
    let healed = owner.evaluate(&fresh_present);
    assert_eq!(healed.state, VideoSchedulingOwnerState::StableServing);
    assert_eq!(healed.health, VideoHealthContract::Stable);
}

#[test]
fn clean_anchor_with_terminal_invalid_bootstrap_releases_rebuilding_supply() {
    let mut owner = VideoSchedulingOwner::new();
    let first = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        Some("receiverWaitingKeyframe"),
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
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("normal".to_string()),
            no_pending_streak: Some(0),
            present_age_ms: Some(24.0),
            decode_age_ms: Some(18.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("waiting-keyframe"),
        Some("gap-reorder-pending"),
        Some("remoteTrackAttached"),
        Some(84_000),
        140.0,
        7,
    );
    recovered.clean_anchor_epoch = Some(7);
    recovered.clean_anchor_observed_at_ms = Some(132.0);
    recovered.clean_anchor_source_event = Some("displayed-idr".to_string());

    recovered.recovery_displayed_idr_at_ms = recovered.clean_anchor_observed_at_ms;

    recovered.recovery_fresh_anchor_recovered_at_ms = recovered.clean_anchor_observed_at_ms;
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
            state: "waiting-keyframe".to_string(),
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
    assert_eq!(second.state, VideoSchedulingOwnerState::RebuildingSupply);
    assert!(second.recovery_intent.is_some());
}

#[test]
fn terminal_invalid_bootstrap_without_clean_anchor_releases_rebuilding_supply_when_output_serviceable(
) {
    let mut owner = VideoSchedulingOwner::new();
    let first = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        Some("receiverWaitingKeyframe"),
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
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("normal".to_string()),
            no_pending_streak: Some(0),
            present_age_ms: Some(26.0),
            decode_age_ms: Some(18.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("waiting-keyframe"),
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
            state: "waiting-keyframe".to_string(),
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
    assert_eq!(second.state, VideoSchedulingOwnerState::RebuildingSupply);
    assert!(second.recovery_intent.is_some());
}

#[test]
fn non_idr_grace_window_keeps_rebuilding_supply_until_clean_anchor_reaches_stable_serving() {
    let mut owner = VideoSchedulingOwner::new();
    let first = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal::default(),
        None,
        None,
        None,
        None,
        100.0,
        10,
    ));
    assert_eq!(first.state, VideoSchedulingOwnerState::RebuildingSupply);

    let mut grace_blocked = input(
        ConnectionLifecycleStateFact::Connected,
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("normal".to_string()),
            no_pending_streak: Some(0),
            present_age_ms: Some(26.0),
            decode_age_ms: Some(18.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("waiting-keyframe"),
        Some("gap-reorder-pending"),
        Some("remoteTrackAttached"),
        Some(88_000),
        140.0,
        10,
    );
    grace_blocked.latest_video_timeline_observation =
        Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 31,
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
                state: "waiting-keyframe".to_string(),
                reason: None,
                chain_break_evidence: None,
                observed_at_ms: 139.0,
            },
            observed_at_ms: 139.0,
        });
    grace_blocked.latest_h264_bootstrap_ready = Some(false);
    grace_blocked.latest_h264_bootstrap_reject_reason = Some("NonIdrVcl".to_string());
    grace_blocked.latest_h264_committed_sps_present = Some(true);
    grace_blocked.latest_h264_committed_pps_present = Some(true);
    grace_blocked.latest_h264_delta_continuation_ready = Some(true);
    grace_blocked.latest_h264_observed_at_ms = Some(139.0);

    let blocked = owner.evaluate(&grace_blocked);
    assert_eq!(blocked.state, VideoSchedulingOwnerState::RebuildingSupply);
    assert!(
        blocked.recovery_intent.is_some(),
        "grace window 内先到的 Non-IDR 继续保持恢复链，不释放为 serviceable"
    );

    let mut healed = input(
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
        Some("receiving"),
        Some("frame-observed"),
        Some("remoteTrackAttached"),
        Some(90_000),
        160.0,
        10,
    );
    healed.clean_anchor_epoch = Some(10);
    healed.clean_anchor_observed_at_ms = Some(158.0);
    healed.clean_anchor_source_event = Some("displayed-idr".to_string());

    healed.recovery_displayed_idr_at_ms = healed.clean_anchor_observed_at_ms;

    healed.recovery_fresh_anchor_recovered_at_ms = healed.clean_anchor_observed_at_ms;

    seed_ledger_display_recovery(&mut healed);
    let stable = owner.evaluate(&healed);
    assert_eq!(stable.state, VideoSchedulingOwnerState::StableServing);
    assert_eq!(stable.health, VideoHealthContract::Stable);
    assert!(stable.recovery_intent.is_none());
}

#[test]
fn terminal_invalid_bootstrap_without_clean_anchor_and_without_serviceable_output_stays_rebuilding_supply(
) {
    let mut owner = VideoSchedulingOwner::new();
    let first = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        Some("receiverWaitingKeyframe"),
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
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("normal".to_string()),
            no_pending_streak: Some(0),
            present_age_ms: Some(420.0),
            decode_age_ms: Some(380.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("waiting-keyframe"),
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
            state: "waiting-keyframe".to_string(),
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
fn fresh_terminal_invalid_bootstrap_after_serving_stays_rebuilding_supply() {
    let mut owner = VideoSchedulingOwner::new();
    let priming = owner.evaluate(&input(
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
        Some("receiving"),
        Some("frame-complete-candidate"),
        Some("remoteTrackAttached"),
        Some(96_000),
        100.0,
        12,
    ));
    assert_eq!(priming.state, VideoSchedulingOwnerState::Priming);

    let stable = owner.evaluate(&input(
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
        Some("receiving"),
        Some("frame-complete-candidate"),
        Some("remoteTrackAttached"),
        Some(96_000),
        140.0,
        12,
    ));
    assert_eq!(stable.state, VideoSchedulingOwnerState::StableServing);

    let mut blocked = input(
        ConnectionLifecycleStateFact::Connected,
        None,
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("critical".to_string()),
            no_pending_streak: Some(168),
            present_age_ms: Some(2_691.0),
            decode_age_ms: Some(2_708.0),
            video_renderer_stalled: true,
            ..SchedulingDemandSignal::default()
        },
        Some("receiving"),
        Some("frame-complete-candidate"),
        Some("remoteTrackAttached"),
        Some(96_000),
        220.0,
        12,
    );
    blocked.latest_h264_bootstrap_ready = Some(false);
    blocked.latest_h264_bootstrap_reject_reason = Some("NonIdrVcl".to_string());
    blocked.latest_h264_committed_sps_present = Some(true);
    blocked.latest_h264_committed_pps_present = Some(true);
    blocked.latest_h264_delta_continuation_ready = Some(true);
    blocked.latest_h264_observed_at_ms = Some(210.0);

    let output = owner.evaluate(&blocked);
    assert_eq!(output.state, VideoSchedulingOwnerState::SupplyStarved);
    assert!(output.recovery_intent.is_some());
}

#[test]
fn degraded_supply_still_releases_terminal_invalid_bootstrap_waiting() {
    let mut owner = VideoSchedulingOwner::new();
    let first = owner.evaluate(&input(
        ConnectionLifecycleStateFact::Connected,
        Some("receiverWaitingKeyframe"),
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
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("normal".to_string()),
            no_pending_streak: Some(0),
            present_age_ms: Some(210.0),
            decode_age_ms: Some(150.0),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("waiting-keyframe"),
        Some("gap-reorder-pending"),
        Some("remoteTrackAttached"),
        Some(96_000),
        140.0,
        8,
    );
    recovered.clean_anchor_epoch = Some(8);
    recovered.clean_anchor_observed_at_ms = Some(132.0);
    recovered.clean_anchor_source_event = Some("displayed-idr".to_string());

    recovered.recovery_displayed_idr_at_ms = recovered.clean_anchor_observed_at_ms;

    recovered.recovery_fresh_anchor_recovered_at_ms = recovered.clean_anchor_observed_at_ms;
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
            state: "waiting-keyframe".to_string(),
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
    assert_eq!(second.state, VideoSchedulingOwnerState::RebuildingSupply);
    assert!(second.recovery_intent.is_some());
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
        Some("receiving"),
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
        Some("receiving"),
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
        Some("receiving"),
        Some("frame-await-recovery-anchor"),
        Some("remoteTrackAttached"),
        Some(96_000),
        700.0,
        9,
    );
    probed.clean_anchor_epoch = Some(9);
    probed.clean_anchor_observed_at_ms = Some(698.0);
    probed.clean_anchor_source_event = Some("displayed-idr".to_string());

    probed.recovery_displayed_idr_at_ms = probed.clean_anchor_observed_at_ms;

    probed.recovery_fresh_anchor_recovered_at_ms = probed.clean_anchor_observed_at_ms;

    seed_ledger_display_recovery(&mut probed);
    let output = owner.evaluate(&probed);
    assert_ne!(output.state, VideoSchedulingOwnerState::RebuildingSupply);
    assert!(output.recovery_intent.is_none());
}

#[test]
fn supply_starved_with_pipeline_stress_and_waiting_keyframe_emits_transport_await_intent() {
    let mut owner = VideoSchedulingOwner::new();
    owner.state = VideoSchedulingOwnerState::SupplyStarved;
    let stressed = input(
        ConnectionLifecycleStateFact::Connected,
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal {
            present_age_ms: Some(48.0),
            decode_age_ms: Some(24.0),
            smoothed_present_fps: Some(10.0),
            smoothed_decode_fps: Some(31.0),
            host_display_tick_epoch: Some(120),
            host_frame_present_epoch: Some(118),
            ..SchedulingDemandSignal::default()
        },
        Some("waiting-keyframe"),
        Some("nack-observation"),
        Some("remoteTrackAttached"),
        Some(96_000),
        1_200.0,
        9,
    );
    let output = owner.evaluate(&stressed);
    let intent = output
        .recovery_intent
        .expect("waiting-keyframe under pipeline stress should request transport anchor");
    assert_eq!(intent.reason_label, "receiverWaitingKeyframe");
    assert_eq!(intent.source, RecoveryIntentSource::Anchor);
}

#[test]
fn supply_starved_pipeline_stress_absorbs_to_degraded_serving() {
    let mut owner = VideoSchedulingOwner::new();
    owner.state = VideoSchedulingOwnerState::SupplyStarved;
    let stressed = input(
        ConnectionLifecycleStateFact::Connected,
        None,
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("critical".to_string()),
            no_pending_streak: Some(120),
            present_age_ms: Some(26.0),
            decode_age_ms: Some(20.0),
            smoothed_present_fps: Some(12.0),
            smoothed_decode_fps: Some(30.0),
            host_display_tick_epoch: Some(200),
            host_frame_present_epoch: Some(198),
            host_cadence_phase: Some("steady".to_string()),
            ..SchedulingDemandSignal::default()
        },
        Some("receiving"),
        Some("frame-complete-candidate"),
        Some("remoteTrackAttached"),
        Some(96_000),
        1_500.0,
        9,
    );
    let output = owner.evaluate(&stressed);
    assert_ne!(output.state, VideoSchedulingOwnerState::SupplyStarved);
    assert_ne!(output.health, VideoHealthContract::Starved);
}

#[test]
fn supply_starved_repairing_chain_with_fresh_output_exits_to_degraded_serving() {
    let mut owner = VideoSchedulingOwner::new();
    owner.state = VideoSchedulingOwnerState::SupplyStarved;
    let mut repairing = input(
        ConnectionLifecycleStateFact::Connected,
        None,
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("normal".to_string()),
            no_pending_streak: Some(0),
            present_age_ms: Some(44.0),
            decode_age_ms: Some(23.0),
            submit_age_ms: Some(20.0),
            smoothed_present_fps: Some(17.4),
            smoothed_decode_fps: Some(22.5),
            host_display_tick_epoch: Some(14_166),
            host_frame_present_epoch: Some(1_541),
            host_cadence_phase: Some("steady".to_string()),
            video_renderer_stalled: false,
            ..SchedulingDemandSignal::default()
        },
        Some("repairing"),
        Some("insert-gate-supply-break"),
        Some("remoteTrackAttached"),
        Some(225_308_692),
        1_785_100.0,
        9,
    );
    repairing.receive_keyframe_required = Some(false);
    repairing.receive_keyframe_response_state = Some("non-idr-only".to_string());
    repairing.receive_display_state = Some("none".to_string());

    let output = owner.evaluate(&repairing);

    assert_eq!(output.state, VideoSchedulingOwnerState::DegradedServing);
    assert_eq!(output.health, VideoHealthContract::Stable);
    assert!(output.recovery_intent.is_none());
}

#[test]
fn display_pipeline_stress_exits_rebuilding_supply_to_degraded_serving() {
    let mut owner = VideoSchedulingOwner::new();
    owner.state = VideoSchedulingOwnerState::RebuildingSupply;
    let mut stressed = input(
        ConnectionLifecycleStateFact::Connected,
        Some("receiverWaitingKeyframe"),
        SchedulingDemandSignal {
            no_pending_pressure_level: Some("normal".to_string()),
            no_pending_streak: Some(2),
            present_age_ms: Some(48.0),
            decode_age_ms: Some(24.0),
            smoothed_present_fps: Some(10.0),
            smoothed_decode_fps: Some(31.0),
            host_display_tick_epoch: Some(120),
            host_frame_present_epoch: Some(118),
            ..SchedulingDemandSignal::default()
        },
        Some("receiving"),
        Some("frame-complete-candidate"),
        Some("remoteTrackAttached"),
        Some(96_000),
        1_200.0,
        9,
    );
    stressed.anchor_reason_label = Some("receiverWaitingKeyframe".to_string());
    let output = owner.evaluate(&stressed);
    assert_ne!(output.state, VideoSchedulingOwnerState::RebuildingSupply);
}

#[test]
fn owner_contract_state_maps_internal_states_to_four_state_contract() {
    assert_eq!(
        VideoSchedulingOwnerState::StableServing.contract_state(false, false),
        VideoSchedulingOwnerContractState::Playing
    );
    assert_eq!(
        VideoSchedulingOwnerState::RebuildingSupply.contract_state(false, false),
        VideoSchedulingOwnerContractState::WaitingKeyframe
    );
    assert_eq!(
        VideoSchedulingOwnerState::SupplyStarved.contract_state(false, false),
        VideoSchedulingOwnerContractState::DisplayStalled
    );
    assert_eq!(
        VideoSchedulingOwnerState::Priming.contract_state(false, false),
        VideoSchedulingOwnerContractState::Starting
    );
    assert_eq!(
        VideoSchedulingOwnerState::StableServing.contract_state(true, false),
        VideoSchedulingOwnerContractState::WaitingKeyframe
    );
    assert_eq!(
        VideoSchedulingOwnerState::StableServing.contract_state(false, true),
        VideoSchedulingOwnerContractState::DisplayStalled
    );
}
