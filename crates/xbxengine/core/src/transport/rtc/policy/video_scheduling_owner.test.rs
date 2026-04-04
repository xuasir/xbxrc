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
            anchor_reason_label: anchor_reason_label.map(str::to_string),
            demand,
            clean_anchor_epoch: None,
            clean_anchor_observed_at_ms: None,
            clean_anchor_source_event: None,
            latest_timeline_chain_state: timeline_chain_state.map(str::to_string),
            latest_timeline_source_event: timeline_source_event.map(str::to_string),
            latest_track_state: track_state.map(str::to_string),
            latest_track_video_bytes_total: track_video_bytes_total,
            display_supply_thresholds: thresholds(),
            observed_at_ms,
            latest_anchor_candidate_ledger: None,
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
        assert!(not_ready.temporary_diagnostic_summary.is_some());
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
    fn critical_wait_keyframe_noise_prefers_rebuilding_over_supply_starved_even_with_clean_anchor()
    {
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
        assert_eq!(output.state, VideoSchedulingOwnerState::RebuildingSupply);
        assert_eq!(output.health, VideoHealthContract::Recovering);
        let intent = output.recovery_intent.expect("anchor intent");
        assert_eq!(intent.source, RecoveryIntentSource::Anchor);
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
        assert!(stable.temporary_diagnostic_summary.is_none());
    }

    #[test]
    fn clean_anchor_recovery_requires_present_freshness_before_stable_serving() {
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
        missing_present.clean_anchor_source_event =
            Some("chain-clean-keyframe-submitted".to_string());
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
        fresh_present.clean_anchor_source_event =
            Some("chain-clean-keyframe-submitted".to_string());
        let healed = owner.evaluate(&fresh_present);
        assert_eq!(healed.state, VideoSchedulingOwnerState::StableServing);
        assert_eq!(healed.health, VideoHealthContract::Stable);
    }
