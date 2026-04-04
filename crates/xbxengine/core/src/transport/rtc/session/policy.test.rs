    use super::RtcSessionPolicy;
    use crate::api::backend::{XbxEngineMediaRuntimeStats, XbxEngineVideoTwccObservation};
    use crate::api::runtime::XbxEngineRuntimeConfig;
    use crate::transport::rtc::facts::{ConnectionLifecycleStateFact, TransportCommand};
    use crate::transport::rtc::policy::display_supply::SchedulingDemandSignal;
    use crate::transport::rtc::projection::{
        BweProjection, ConnectionProjection, DiagnosticsProjection, MediaProjection,
        RecoveryProjection, TransportSnapshot,
    };
    use crate::transport::rtc::session::actor::SessionPolicyHook;
    use std::sync::{Arc, Mutex};

    fn build_demand_for_stats(
        stats: &XbxEngineMediaRuntimeStats,
        now_ms: f64,
    ) -> SchedulingDemandSignal {
        SchedulingDemandSignal {
            no_pending_pressure_level: stats.host_no_pending_pressure_level.clone(),
            no_pending_streak: Some(stats.host_no_pending_streak),
            present_age_ms: stats
                .latest_video_host_present_time_ms
                .map(|ts| (now_ms - ts).max(0.0)),
            decode_age_ms: stats
                .latest_video_decode_ok_time_ms
                .map(|ts| (now_ms - ts).max(0.0)),
            video_renderer_stalled: stats.video_renderer_stalled.unwrap_or(false),
            present_submit_count_total: Some(stats.video_present_submit_count_total),
            present_drop_count_total: Some(stats.video_present_drop_count_total),
            present_overwrite_count_total: Some(stats.video_present_overwrite_count_total),
            pacer_submit_count_total: Some(stats.video_pacer_submit_count_total),
            pacer_drop_count_total: Some(stats.video_pacer_drop_count_total),
            renderer_submit_count_total: Some(stats.video_renderer_submit_count_total),
            renderer_drop_count_total: Some(stats.video_renderer_drop_count_total),
        }
    }

    fn classify_supply_state_with_profile(
        stats: &XbxEngineMediaRuntimeStats,
    ) -> crate::transport::rtc::policy::display_supply::DisplaySupplyState {
        let profile =
            crate::transport::rtc::recovery::runtime_state::resolve_runtime_recovery_profile(stats);
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        let demand = build_demand_for_stats(stats, now_ms);
        demand.classify_display_supply_state(&profile.display_supply_thresholds)
    }

    #[test]
    fn reconnect_command_is_throttled_and_re_emitted_during_continuous_recovering() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
        let mut connection = ConnectionProjection::default();
        let mut recovery = RecoveryProjection::default();
        recovery.latest_diagnosis_label = Some("rtcPeerConnectionFailed".to_string());
        connection.lifecycle_state = ConnectionLifecycleStateFact::Recovering;
        let media = MediaProjection {
            frame_count: 1,
            ..MediaProjection::default()
        };

        let first = TransportSnapshot::new(
            1,
            100.0,
            connection.clone(),
            media.clone(),
            recovery.clone(),
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let first_commands = policy.on_snapshot(&first);
        assert!(first_commands.iter().any(|command| {
            matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));

        let second = TransportSnapshot::new(
            2,
            1_200.0,
            connection.clone(),
            media.clone(),
            recovery.clone(),
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let second_commands = policy.on_snapshot(&second);
        assert!(second_commands.iter().all(|command| {
            !matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));

        let third = TransportSnapshot::new(
            3,
            2_701.0,
            connection,
            media,
            recovery,
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let third_commands = policy.on_snapshot(&third);
        assert!(third_commands.iter().any(|command| {
            matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));
    }

    #[test]
    fn cloud_lifecycle_reconnect_interval_is_more_relaxed_than_non_cloud() {
        fn run_for_target(
            session_target_type: Option<xbxengine_protocol::XbxEngineTargetTypeDto>,
        ) -> Vec<Vec<TransportCommand>> {
            let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
            let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
            if let Ok(mut stats) = runtime_stats.lock() {
                stats.session_target_type = session_target_type;
            }
            let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
            let mut connection = ConnectionProjection::default();
            connection.lifecycle_state = ConnectionLifecycleStateFact::Recovering;
            let recovery = RecoveryProjection {
                latest_diagnosis_label: Some("rtcPeerConnectionFailed".to_string()),
                pending_action: false,
                successful_action_count: 0,
                failed_action_count: 0,
                last_observed_at_ms: Some(100.0),
            };
            let media = MediaProjection {
                frame_count: 1,
                ..MediaProjection::default()
            };
            let timestamps = [100.0, 2_000.0];
            timestamps
                .into_iter()
                .enumerate()
                .map(|(idx, ts)| {
                    let snapshot = TransportSnapshot::new(
                        (idx as u64) + 1,
                        ts,
                        connection.clone(),
                        media.clone(),
                        RecoveryProjection {
                            last_observed_at_ms: Some(ts),
                            ..recovery.clone()
                        },
                        BweProjection::default(),
                        DiagnosticsProjection::default(),
                    );
                    policy.on_snapshot(&snapshot)
                })
                .collect()
        }

        let home_commands = run_for_target(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Home));
        let cloud_commands =
            run_for_target(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

        assert!(home_commands[0].iter().any(|command| {
            matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));
        assert!(cloud_commands[0].iter().any(|command| {
            matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));
        assert!(home_commands[1].iter().any(|command| {
            matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));
        assert!(cloud_commands[1].iter().all(|command| {
            !matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));
    }

    #[test]
    fn disconnected_surface_emits_lifecycle_reconnect_without_waiting_no_progress_timeout() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::Disconnected;
        let recovery = RecoveryProjection {
            latest_diagnosis_label: Some("rtcControlChannelClosed".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(100.0),
        };
        let first = TransportSnapshot::new(
            1,
            100.0,
            connection,
            MediaProjection::default(),
            recovery,
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let first_commands = policy.on_snapshot(&first);
        assert!(first_commands.iter().any(|command| {
            matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));
        let stats = runtime_stats.lock().expect("runtime stats lock");
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(
            ledger.input_signal,
            "rtcConnectionRecovering:rtcConnectionDisconnected"
        );
    }

    #[test]
    fn fallback_transport_await_recovery_keyframe_is_not_blocked_before_coordinator() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
        let snapshot = build_snapshot(
            ConnectionLifecycleStateFact::Connected,
            "transportAwaitRecoveryKeyframe",
            100.0,
        );
        let commands = policy.on_snapshot(&snapshot);
        assert!(commands
            .iter()
            .any(|command| matches!(command, TransportCommand::RequestKeyframe { .. })));
    }

    #[test]
    fn connecting_startup_without_progress_triggers_lifecycle_reconnect() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        if let Ok(mut stats) = runtime_stats.lock() {
            stats.session_phase = Some("startup".to_string());
        }
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::Connecting;
        let recovery = RecoveryProjection {
            latest_diagnosis_label: Some("none".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(100.0),
        };
        let first = TransportSnapshot::new(
            1,
            100.0,
            connection.clone(),
            MediaProjection::default(),
            recovery.clone(),
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let first_commands = policy.on_snapshot(&first);
        assert!(first_commands.iter().all(|command| {
            !matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));

        let second = TransportSnapshot::new(
            2,
            4_200.0,
            connection.clone(),
            MediaProjection::default(),
            RecoveryProjection {
                last_observed_at_ms: Some(4_200.0),
                ..recovery.clone()
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let second_commands = policy.on_snapshot(&second);
        assert!(second_commands.iter().all(|command| {
            !matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));

        let third = TransportSnapshot::new(
            3,
            15_600.0,
            connection.clone(),
            MediaProjection::default(),
            RecoveryProjection {
                last_observed_at_ms: Some(15_600.0),
                ..recovery.clone()
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let third_commands = policy.on_snapshot(&third);
        assert!(third_commands.iter().any(|command| {
            matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));

        let fourth = TransportSnapshot::new(
            4,
            16_200.0,
            connection.clone(),
            MediaProjection::default(),
            RecoveryProjection {
                last_observed_at_ms: Some(16_200.0),
                ..recovery.clone()
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let fourth_commands = policy.on_snapshot(&fourth);
        assert!(fourth_commands.iter().all(|command| {
            !matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));

        let fifth = TransportSnapshot::new(
            5,
            20_200.0,
            connection,
            MediaProjection::default(),
            RecoveryProjection {
                last_observed_at_ms: Some(20_200.0),
                ..recovery
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let fifth_commands = policy.on_snapshot(&fifth);
        assert!(fifth_commands.iter().any(|command| {
            matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));
    }

    #[test]
    fn connecting_seeking_anchor_without_progress_triggers_lifecycle_reconnect() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        if let Ok(mut stats) = runtime_stats.lock() {
            stats.video_owner_state = Some("seeking-anchor".to_string());
        }
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::Connecting;
        let recovery = RecoveryProjection {
            latest_diagnosis_label: Some("none".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(100.0),
        };
        let first = TransportSnapshot::new(
            1,
            100.0,
            connection.clone(),
            MediaProjection::default(),
            recovery.clone(),
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let first_commands = policy.on_snapshot(&first);
        assert!(first_commands.iter().all(|command| {
            !matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));

        let second = TransportSnapshot::new(
            2,
            15_600.0,
            connection,
            MediaProjection::default(),
            RecoveryProjection {
                last_observed_at_ms: Some(15_600.0),
                ..recovery
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let second_commands = policy.on_snapshot(&second);
        assert!(second_commands.iter().any(|command| {
            matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));
    }

    #[test]
    fn connecting_without_semantic_hints_still_triggers_liveness_reconnect() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::Connecting;
        let recovery = RecoveryProjection {
            latest_diagnosis_label: Some("none".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(100.0),
        };
        let first = TransportSnapshot::new(
            1,
            100.0,
            connection.clone(),
            MediaProjection::default(),
            recovery.clone(),
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let first_commands = policy.on_snapshot(&first);
        assert!(first_commands.iter().all(|command| {
            !matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));

        let second = TransportSnapshot::new(
            2,
            4_220.0,
            connection.clone(),
            MediaProjection::default(),
            RecoveryProjection {
                last_observed_at_ms: Some(4_220.0),
                ..recovery.clone()
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let second_commands = policy.on_snapshot(&second);
        assert!(second_commands.iter().all(|command| {
            !matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));

        let third = TransportSnapshot::new(
            3,
            15_600.0,
            connection,
            MediaProjection::default(),
            RecoveryProjection {
                last_observed_at_ms: Some(15_600.0),
                ..recovery
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let third_commands = policy.on_snapshot(&third);
        assert!(third_commands.iter().any(|command| {
            matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));
    }

    #[test]
    fn new_state_does_not_emit_liveness_reconnect_before_connecting() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::New;
        let recovery = RecoveryProjection {
            latest_diagnosis_label: Some("none".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(100.0),
        };

        let first = TransportSnapshot::new(
            1,
            100.0,
            connection.clone(),
            MediaProjection::default(),
            recovery.clone(),
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let _ = policy.on_snapshot(&first);

        let second = TransportSnapshot::new(
            2,
            10_000.0,
            connection,
            MediaProjection::default(),
            RecoveryProjection {
                last_observed_at_ms: Some(10_000.0),
                ..recovery
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let second_commands = policy.on_snapshot(&second);
        assert!(second_commands.iter().all(|command| {
            !matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));
    }

    #[test]
    fn lifecycle_reconnect_attempt_limit_enters_failed_terminal() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::Recovering;
        let recovery = RecoveryProjection {
            latest_diagnosis_label: Some("rtcPeerConnectionFailed".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(100.0),
        };
        let media = MediaProjection {
            frame_count: 1,
            ..MediaProjection::default()
        };

        let first = TransportSnapshot::new(
            1,
            100.0,
            connection.clone(),
            media.clone(),
            recovery.clone(),
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let first_commands = policy.on_snapshot(&first);
        assert!(first_commands.iter().any(|command| {
            matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));

        let second = TransportSnapshot::new(
            2,
            2_000.0,
            connection.clone(),
            media.clone(),
            RecoveryProjection {
                last_observed_at_ms: Some(2_000.0),
                ..recovery.clone()
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let second_commands = policy.on_snapshot(&second);
        assert!(second_commands.iter().any(|command| {
            matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));

        let third = TransportSnapshot::new(
            3,
            3_800.0,
            connection.clone(),
            media.clone(),
            RecoveryProjection {
                last_observed_at_ms: Some(3_800.0),
                ..recovery.clone()
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let third_commands = policy.on_snapshot(&third);
        assert!(third_commands.iter().any(|command| {
            matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));

        let fourth = TransportSnapshot::new(
            4,
            5_600.0,
            connection.clone(),
            media.clone(),
            RecoveryProjection {
                last_observed_at_ms: Some(5_600.0),
                ..recovery.clone()
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let fourth_commands = policy.on_snapshot(&fourth);
        assert!(
            fourth_commands.is_empty(),
            "attempts exhausted should enter failed-terminal without emitting more commands"
        );

        let stats = runtime_stats.lock().expect("runtime stats lock");
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(ledger.state_after, "failed-terminal");
        assert_eq!(
            ledger.gate_result,
            "terminal:livenessReconnectAttemptLimitExceeded"
        );
        assert_eq!(ledger.action_selected, "failed-terminal");
        drop(stats);

        let fifth = TransportSnapshot::new(
            5,
            7_300.0,
            connection,
            media,
            RecoveryProjection {
                last_observed_at_ms: Some(7_300.0),
                ..recovery
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let fifth_commands = policy.on_snapshot(&fifth);
        assert!(fifth_commands.is_empty());
    }

    #[test]
    fn failed_terminal_clears_after_successful_progress_and_rearms_reconnect() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::Recovering;
        let base_recovery = RecoveryProjection {
            latest_diagnosis_label: Some("rtcPeerConnectionFailed".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(100.0),
        };
        let media = MediaProjection {
            frame_count: 1,
            ..MediaProjection::default()
        };
        let timeline = [100.0, 2_000.0, 3_800.0, 5_600.0];
        for (idx, ts) in timeline.into_iter().enumerate() {
            let snapshot = TransportSnapshot::new(
                (idx as u64) + 1,
                ts,
                connection.clone(),
                media.clone(),
                RecoveryProjection {
                    last_observed_at_ms: Some(ts),
                    ..base_recovery.clone()
                },
                BweProjection::default(),
                DiagnosticsProjection::default(),
            );
            let _ = policy.on_snapshot(&snapshot);
        }
        {
            let stats = runtime_stats.lock().expect("runtime stats lock");
            let ledger = stats
                .latest_recovery_decision_ledger
                .as_ref()
                .expect("recovery decision ledger");
            assert_eq!(ledger.state_after, "failed-terminal");
        }

        let resumed = TransportSnapshot::new(
            5,
            7_800.0,
            connection,
            MediaProjection {
                frame_count: 2,
                ..media
            },
            RecoveryProjection {
                last_observed_at_ms: Some(7_800.0),
                ..base_recovery
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let resumed_commands = policy.on_snapshot(&resumed);
        assert!(resumed_commands.iter().any(|command| {
            matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));
        let stats = runtime_stats.lock().expect("runtime stats lock");
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(ledger.state_after, "reconnecting");
    }

    #[test]
    fn no_progress_upper_bound_applies_to_connecting_and_recovering_surfaces() {
        let cases = [
            (
                ConnectionLifecycleStateFact::Connecting,
                Some("none".to_string()),
            ),
            (
                ConnectionLifecycleStateFact::Recovering,
                Some("rtcPeerConnectionFailed".to_string()),
            ),
        ];
        for (idx, (lifecycle_state, diagnosis)) in cases.into_iter().enumerate() {
            let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
            let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
            let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
            let mut connection = ConnectionProjection::default();
            connection.lifecycle_state = lifecycle_state;
            let recovery = RecoveryProjection {
                latest_diagnosis_label: diagnosis,
                pending_action: false,
                successful_action_count: 0,
                failed_action_count: 0,
                last_observed_at_ms: Some(100.0),
            };
            let media = MediaProjection {
                frame_count: if lifecycle_state == ConnectionLifecycleStateFact::Recovering {
                    1
                } else {
                    0
                },
                ..MediaProjection::default()
            };
            let first = TransportSnapshot::new(
                ((idx as u64) * 10) + 1,
                100.0,
                connection.clone(),
                media.clone(),
                recovery.clone(),
                BweProjection::default(),
                DiagnosticsProjection::default(),
            );
            let _ = policy.on_snapshot(&first);
            let second_ts = if lifecycle_state == ConnectionLifecycleStateFact::Connecting {
                15_600.0
            } else {
                4_300.0
            };
            let second = TransportSnapshot::new(
                ((idx as u64) * 10) + 2,
                second_ts,
                connection,
                media,
                RecoveryProjection {
                    last_observed_at_ms: Some(second_ts),
                    ..recovery
                },
                BweProjection::default(),
                DiagnosticsProjection::default(),
            );
            let second_commands = policy.on_snapshot(&second);
            assert!(
                second_commands.iter().any(|command| {
                    matches!(command, TransportCommand::RequestReconnectCandidate { .. })
                }),
                "case idx={} should emit reconnect under no-progress upper bound",
                idx
            );
        }
    }

    #[test]
    fn pre_first_frame_transport_progress_uses_relaxed_liveness_timeout() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::Connecting;
        connection.latest_transport_path = Some("Direct".to_string());
        connection.latest_rtt_ms = Some(9.0);
        let media = MediaProjection {
            frame_count: 0,
            ..MediaProjection::default()
        };
        let recovery = RecoveryProjection {
            latest_diagnosis_label: Some("none".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(100.0),
        };

        let first = TransportSnapshot::new(
            1,
            100.0,
            connection.clone(),
            media.clone(),
            recovery.clone(),
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let _ = policy.on_snapshot(&first);

        let second = TransportSnapshot::new(
            2,
            4_300.0,
            connection.clone(),
            media.clone(),
            RecoveryProjection {
                last_observed_at_ms: Some(4_300.0),
                ..recovery.clone()
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let second_commands = policy.on_snapshot(&second);
        assert!(
            second_commands.iter().all(|command| {
                !matches!(command, TransportCommand::RequestReconnectCandidate { .. })
            }),
            "transport 已有进展但尚未首帧时，不应在 4s 上界内过早重连"
        );

        let third = TransportSnapshot::new(
            3,
            15_600.0,
            connection,
            media,
            RecoveryProjection {
                last_observed_at_ms: Some(15_600.0),
                ..recovery
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let third_commands = policy.on_snapshot(&third);
        assert!(third_commands.iter().any(|command| {
            matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));
    }

    #[test]
    fn recovering_pre_first_frame_without_transport_progress_uses_relaxed_liveness_timeout() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::Recovering;
        let media = MediaProjection {
            frame_count: 0,
            ..MediaProjection::default()
        };
        let recovery = RecoveryProjection {
            latest_diagnosis_label: Some("none".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(100.0),
        };

        let first = TransportSnapshot::new(
            1,
            100.0,
            connection.clone(),
            media.clone(),
            recovery.clone(),
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let _ = policy.on_snapshot(&first);

        let second = TransportSnapshot::new(
            2,
            4_300.0,
            connection.clone(),
            media.clone(),
            RecoveryProjection {
                last_observed_at_ms: Some(4_300.0),
                ..recovery.clone()
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let second_commands = policy.on_snapshot(&second);
        assert!(
            second_commands.iter().all(|command| {
                !matches!(command, TransportCommand::RequestReconnectCandidate { .. })
            }),
            "首帧前即便尚无 transport 进展，也不应在 4s 内过早重连"
        );

        let third = TransportSnapshot::new(
            3,
            15_600.0,
            connection,
            media,
            RecoveryProjection {
                last_observed_at_ms: Some(15_600.0),
                ..recovery
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let third_commands = policy.on_snapshot(&third);
        assert!(third_commands.iter().any(|command| {
            matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));
    }

    #[test]
    fn cloud_early_connecting_without_builder_waits_for_long_terminal_window() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        if let Ok(mut stats) = runtime_stats.lock() {
            stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud);
        }
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::Connecting;
        let recovery = RecoveryProjection {
            latest_diagnosis_label: Some("none".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(100.0),
        };
        let media = MediaProjection::default();

        let first = TransportSnapshot::new(
            1,
            100.0,
            connection.clone(),
            media.clone(),
            recovery.clone(),
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let first_commands = policy.on_snapshot(&first);
        assert!(first_commands.iter().all(|command| {
            !matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));

        let second = TransportSnapshot::new(
            2,
            15_600.0,
            connection.clone(),
            media.clone(),
            RecoveryProjection {
                last_observed_at_ms: Some(15_600.0),
                ..recovery.clone()
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let second_commands = policy.on_snapshot(&second);
        assert!(second_commands.iter().all(|command| {
            !matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));
        let stats = runtime_stats.lock().expect("runtime stats lock");
        assert!(
            stats.latest_rtc_builder_observation.is_none(),
            "early connecting soft hold 应在 builder 尚未出现时就生效"
        );
        drop(stats);

        let third = TransportSnapshot::new(
            3,
            35_600.0,
            connection.clone(),
            media.clone(),
            RecoveryProjection {
                last_observed_at_ms: Some(35_600.0),
                ..recovery.clone()
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let third_commands = policy.on_snapshot(&third);
        assert!(third_commands.iter().any(|command| {
            matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));

        let fourth = TransportSnapshot::new(
            4,
            38_200.0,
            connection.clone(),
            media.clone(),
            RecoveryProjection {
                last_observed_at_ms: Some(38_200.0),
                ..recovery.clone()
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let fourth_commands = policy.on_snapshot(&fourth);
        assert!(
            fourth_commands.iter().all(|command| {
                !matches!(command, TransportCommand::RequestReconnectCandidate { .. })
            }),
            "connecting + 首帧前应按更长间隔节流 reconnect"
        );

        let reconnect_ticks = [40_200.0, 44_800.0, 49_400.0, 53_900.0, 58_400.0];
        for (idx, ts) in reconnect_ticks.into_iter().enumerate() {
            let snapshot = TransportSnapshot::new(
                (idx as u64) + 5,
                ts,
                connection.clone(),
                media.clone(),
                RecoveryProjection {
                    last_observed_at_ms: Some(ts),
                    ..recovery.clone()
                },
                BweProjection::default(),
                DiagnosticsProjection::default(),
            );
            let commands = policy.on_snapshot(&snapshot);
            assert!(
                commands.iter().any(|command| {
                    matches!(command, TransportCommand::RequestReconnectCandidate { .. })
                }),
                "cloud 在长窗口内应继续允许第 {} 次无进展 reconnect 尝试",
                idx + 2
            );
        }

        let terminal = TransportSnapshot::new(
            10,
            90_200.0,
            connection,
            media,
            RecoveryProjection {
                last_observed_at_ms: Some(90_200.0),
                ..recovery
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let terminal_commands = policy.on_snapshot(&terminal);
        assert!(
            terminal_commands.is_empty(),
            "cloud 只有超过长窗口后才允许进入 failed-terminal"
        );
        let stats = runtime_stats.lock().expect("runtime stats lock");
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(
            ledger.gate_result,
            "terminal:livenessReconnectAttemptLimitExceeded"
        );
        assert_eq!(ledger.state_after, "failed-terminal");
    }

    #[test]
    fn cloud_early_new_without_builder_waits_for_long_terminal_window() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        if let Ok(mut stats) = runtime_stats.lock() {
            stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud);
        }
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::New;
        let recovery = RecoveryProjection {
            latest_diagnosis_label: Some("none".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(100.0),
        };
        let media = MediaProjection::default();

        let first = TransportSnapshot::new(
            1,
            100.0,
            connection.clone(),
            media.clone(),
            recovery.clone(),
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let first_commands = policy.on_snapshot(&first);
        assert!(first_commands.iter().all(|command| {
            !matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));

        for (idx, ts) in [35_600.0, 40_200.0, 44_800.0, 49_400.0, 53_900.0, 58_400.0]
            .into_iter()
            .enumerate()
        {
            let snapshot = TransportSnapshot::new(
                (idx as u64) + 2,
                ts,
                connection.clone(),
                media.clone(),
                RecoveryProjection {
                    last_observed_at_ms: Some(ts),
                    ..recovery.clone()
                },
                BweProjection::default(),
                DiagnosticsProjection::default(),
            );
            let commands = policy.on_snapshot(&snapshot);
            assert!(
                commands.iter().any(|command| {
                    matches!(command, TransportCommand::RequestReconnectCandidate { .. })
                }),
                "cloud new 首窗在长窗口内应继续允许第 {} 次无进展 reconnect 尝试",
                idx + 1
            );
        }

        let pre_terminal = TransportSnapshot::new(
            8,
            58_500.0,
            connection.clone(),
            media.clone(),
            RecoveryProjection {
                last_observed_at_ms: Some(58_500.0),
                ..recovery.clone()
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let pre_terminal_commands = policy.on_snapshot(&pre_terminal);
        assert!(
            pre_terminal_commands.iter().all(|command| {
                !matches!(command, TransportCommand::RequestReconnectCandidate { .. })
            }),
            "proposal interval 仍应保持 4.5s 节流"
        );
        let stats = runtime_stats.lock().expect("runtime stats lock");
        assert!(
            stats.latest_rtc_builder_observation.is_none(),
            "cloud new 首窗 soft hold 应在 builder 尚未出现时就生效"
        );
        drop(stats);

        let terminal = TransportSnapshot::new(
            9,
            90_200.0,
            connection,
            media,
            RecoveryProjection {
                last_observed_at_ms: Some(90_200.0),
                ..recovery
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let terminal_commands = policy.on_snapshot(&terminal);
        assert!(
            terminal_commands.is_empty(),
            "cloud new 首窗只有超过长窗口后才允许进入 failed-terminal"
        );
        let stats = runtime_stats.lock().expect("runtime stats lock");
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(
            ledger.gate_result,
            "terminal:livenessReconnectAttemptLimitExceeded"
        );
        assert_eq!(ledger.state_after, "failed-terminal");
    }

    #[test]
    fn cloud_early_recovering_without_builder_waits_for_long_terminal_window() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        if let Ok(mut stats) = runtime_stats.lock() {
            stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud);
        }
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::Recovering;
        let recovery = RecoveryProjection {
            latest_diagnosis_label: Some("rtcPeerConnectionFailed".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(100.0),
        };
        let media = MediaProjection::default();

        let first = TransportSnapshot::new(
            1,
            100.0,
            connection.clone(),
            media.clone(),
            recovery.clone(),
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let _ = policy.on_snapshot(&first);

        let second = TransportSnapshot::new(
            2,
            15_600.0,
            connection.clone(),
            media.clone(),
            RecoveryProjection {
                last_observed_at_ms: Some(15_600.0),
                ..recovery.clone()
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let second_commands = policy.on_snapshot(&second);
        assert!(
            second_commands.iter().all(|command| {
                !matches!(command, TransportCommand::RequestReconnectCandidate { .. })
            }),
            "cloud recovering 首窗在长窗口前不应进入 reconnect"
        );

        let third = TransportSnapshot::new(
            3,
            35_600.0,
            connection.clone(),
            media.clone(),
            RecoveryProjection {
                last_observed_at_ms: Some(35_600.0),
                ..recovery.clone()
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let third_commands = policy.on_snapshot(&third);
        assert!(third_commands.iter().any(|command| {
            matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));

        for (idx, ts) in [38_200.0, 40_800.0, 43_400.0, 46_000.0, 48_600.0]
            .into_iter()
            .enumerate()
        {
            let snapshot = TransportSnapshot::new(
                (idx as u64) + 4,
                ts,
                connection.clone(),
                media.clone(),
                RecoveryProjection {
                    last_observed_at_ms: Some(ts),
                    ..recovery.clone()
                },
                BweProjection::default(),
                DiagnosticsProjection::default(),
            );
            let commands = policy.on_snapshot(&snapshot);
            assert!(
                commands.iter().any(|command| {
                    matches!(command, TransportCommand::RequestReconnectCandidate { .. })
                }),
                "cloud recovering 首窗长窗口内应继续允许无进展 reconnect 尝试，idx={}",
                idx
            );
        }

        let terminal = TransportSnapshot::new(
            9,
            90_200.0,
            connection,
            media,
            RecoveryProjection {
                last_observed_at_ms: Some(90_200.0),
                ..recovery
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let terminal_commands = policy.on_snapshot(&terminal);
        assert!(
            terminal_commands.is_empty(),
            "cloud recovering 首窗超过长窗口后应进入 failed-terminal"
        );
        let stats = runtime_stats.lock().expect("runtime stats lock");
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(
            ledger.gate_result,
            "terminal:livenessReconnectAttemptLimitExceeded"
        );
        assert_eq!(ledger.state_after, "failed-terminal");
    }

    #[test]
    fn connecting_without_target_type_keeps_reconnecting_before_long_terminal_window() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::Connecting;
        let recovery = RecoveryProjection {
            latest_diagnosis_label: Some("none".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(100.0),
        };
        let media = MediaProjection::default();

        for (idx, ts) in [100.0, 15_600.0, 20_200.0, 24_800.0]
            .into_iter()
            .enumerate()
        {
            let snapshot = TransportSnapshot::new(
                (idx as u64) + 1,
                ts,
                connection.clone(),
                media.clone(),
                RecoveryProjection {
                    last_observed_at_ms: Some(ts),
                    ..recovery.clone()
                },
                BweProjection::default(),
                DiagnosticsProjection::default(),
            );
            let commands = policy.on_snapshot(&snapshot);
            if idx == 0 {
                assert!(commands.iter().all(|command| {
                    !matches!(command, TransportCommand::RequestReconnectCandidate { .. })
                }));
            } else {
                assert!(commands.iter().any(|command| {
                    matches!(command, TransportCommand::RequestReconnectCandidate { .. })
                }));
            }
        }

        let stats = runtime_stats.lock().expect("runtime stats lock");
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_ne!(ledger.state_after, "failed-terminal");
        assert_ne!(
            ledger.gate_result,
            "terminal:livenessReconnectAttemptLimitExceeded"
        );
        drop(stats);

        // target_type 未决的首窗也要遵循长窗口，超过阈值后仍应进入 terminal，避免无限重试。
        let terminal = TransportSnapshot::new(
            5,
            90_200.0,
            connection,
            media,
            RecoveryProjection {
                last_observed_at_ms: Some(90_200.0),
                ..recovery
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let terminal_commands = policy.on_snapshot(&terminal);
        assert!(
            terminal_commands.is_empty(),
            "target_type 缺失场景超过长窗口后应进入 failed-terminal"
        );
        let stats = runtime_stats.lock().expect("runtime stats lock");
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(
            ledger.gate_result,
            "terminal:livenessReconnectAttemptLimitExceeded"
        );
        assert_eq!(ledger.state_after, "failed-terminal");
    }

    #[test]
    fn recovering_without_first_frame_does_not_emit_periodic_reconnect() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::Recovering;
        let media = MediaProjection {
            frame_count: 0,
            ..MediaProjection::default()
        };
        let recovery = RecoveryProjection {
            latest_diagnosis_label: Some("rtcPeerConnectionFailed".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(100.0),
        };

        let first = TransportSnapshot::new(
            1,
            100.0,
            connection.clone(),
            media.clone(),
            recovery.clone(),
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let _ = policy.on_snapshot(&first);

        let second = TransportSnapshot::new(
            2,
            2_000.0,
            connection.clone(),
            media.clone(),
            RecoveryProjection {
                last_observed_at_ms: Some(2_000.0),
                ..recovery.clone()
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let second_commands = policy.on_snapshot(&second);
        assert!(
            second_commands.iter().all(|command| {
                !matches!(command, TransportCommand::RequestReconnectCandidate { .. })
            }),
            "Recovering + 首帧前不应按 1.5s 节流周期反复触发 reconnect"
        );
    }

    #[test]
    fn liveness_uses_snapshot_now_when_last_observed_stalls() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::Connecting;
        let recovery = RecoveryProjection {
            latest_diagnosis_label: Some("none".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(100.0),
        };
        let media = MediaProjection {
            frame_count: 0,
            ..MediaProjection::default()
        };

        let first = TransportSnapshot::new(
            1,
            100.0,
            connection.clone(),
            media.clone(),
            recovery.clone(),
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let _ = policy.on_snapshot(&first);

        // 模拟 recovery.last_observed_at_ms 卡住不变，但 snapshot.now_ms 持续推进。
        let second = TransportSnapshot::new(
            2,
            15_600.0,
            connection,
            media,
            recovery,
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let second_commands = policy.on_snapshot(&second);
        assert!(second_commands.iter().any(|command| {
            matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));
    }

    #[test]
    fn command_success_without_frames_does_not_reset_liveness_budget() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        if let Ok(mut stats) = runtime_stats.lock() {
            stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Home);
        }
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::Connecting;
        let base_recovery = RecoveryProjection {
            latest_diagnosis_label: Some("none".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(100.0),
        };
        let media = MediaProjection {
            frame_count: 0,
            ..MediaProjection::default()
        };

        let first = TransportSnapshot::new(
            1,
            100.0,
            connection.clone(),
            media.clone(),
            base_recovery.clone(),
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let first_commands = policy.on_snapshot(&first);
        assert!(first_commands.iter().all(|command| {
            !matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));

        let second = TransportSnapshot::new(
            2,
            15_600.0,
            connection.clone(),
            media.clone(),
            RecoveryProjection {
                successful_action_count: 1,
                last_observed_at_ms: Some(15_600.0),
                ..base_recovery.clone()
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let second_commands = policy.on_snapshot(&second);
        assert!(second_commands.iter().any(|command| {
            matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));

        let third = TransportSnapshot::new(
            3,
            20_200.0,
            connection.clone(),
            media.clone(),
            RecoveryProjection {
                successful_action_count: 2,
                last_observed_at_ms: Some(20_200.0),
                ..base_recovery.clone()
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let third_commands = policy.on_snapshot(&third);
        assert!(third_commands.iter().any(|command| {
            matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));

        let fourth = TransportSnapshot::new(
            4,
            24_800.0,
            connection.clone(),
            media,
            RecoveryProjection {
                successful_action_count: 3,
                last_observed_at_ms: Some(24_800.0),
                ..base_recovery
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let fourth_commands = policy.on_snapshot(&fourth);
        assert!(
            fourth_commands.iter().any(|command| {
                matches!(command, TransportCommand::RequestReconnectCandidate { .. })
            }),
            "third no-progress reconnect is still allowed before terminal closes the loop"
        );

        let fifth = TransportSnapshot::new(
            5,
            90_200.0,
            connection,
            MediaProjection::default(),
            RecoveryProjection {
                successful_action_count: 4,
                last_observed_at_ms: Some(90_200.0),
                ..RecoveryProjection::default()
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let fifth_commands = policy.on_snapshot(&fifth);
        assert!(
            fifth_commands.is_empty(),
            "no media progress should still exhaust liveness attempts and stop reconnect loop"
        );
        let stats = runtime_stats.lock().expect("runtime stats lock");
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(ledger.state_after, "failed-terminal");
        assert_eq!(
            ledger.gate_result,
            "terminal:livenessReconnectAttemptLimitExceeded"
        );
    }

    #[test]
    fn connected_ingress_progress_without_present_progress_does_not_force_reconnect() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        if let Ok(mut stats) = runtime_stats.lock() {
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 260;
            // 这里使用与 snapshot 同一时间轴，避免“墙钟时间”与“策略时间”混用。
            stats.latest_video_host_present_time_ms = Some(0.0);
            stats.latest_video_decode_ok_time_ms = Some(0.0);
            stats.inbound_primary_video_bytes_total = 1_000;
        }
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
        connection.control_channel_open = true;
        connection.latest_transport_path = Some("relay/udp".to_string());
        connection.latest_rtt_ms = Some(48.0);
        connection.last_observed_at_ms = Some(100.0);
        let recovery = RecoveryProjection {
            latest_diagnosis_label: Some("none".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(100.0),
        };

        let first = TransportSnapshot::new(
            1,
            100.0,
            connection.clone(),
            MediaProjection {
                frame_count: 10,
                ..MediaProjection::default()
            },
            recovery.clone(),
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let first_commands = policy.on_snapshot(&first);
        assert!(first_commands.iter().all(|command| {
            !matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));

        if let Ok(mut stats) = runtime_stats.lock() {
            stats.inbound_primary_video_bytes_total = 2_000;
        }
        let second = TransportSnapshot::new(
            2,
            5_000.0,
            connection.clone(),
            MediaProjection {
                frame_count: 11,
                ..MediaProjection::default()
            },
            RecoveryProjection {
                last_observed_at_ms: Some(5_000.0),
                ..recovery.clone()
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let second_commands = policy.on_snapshot(&second);
        assert!(second_commands.iter().all(|command| {
            !matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));

        if let Ok(mut stats) = runtime_stats.lock() {
            stats.inbound_primary_video_bytes_total = 3_000;
        }
        let third = TransportSnapshot::new(
            3,
            10_400.0,
            connection,
            MediaProjection {
                frame_count: 12,
                ..MediaProjection::default()
            },
            RecoveryProjection {
                last_observed_at_ms: Some(10_400.0),
                ..recovery
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let third_commands = policy.on_snapshot(&third);
        assert!(third_commands.iter().all(|command| {
            !matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));
        let stats = runtime_stats.lock().expect("runtime stats lock");
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_ne!(ledger.action_selected, "requestReconnectCandidate");
    }

    #[test]
    fn recovery_decision_ledger_is_written_with_budget_snapshot() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
        let snapshot = build_snapshot(
            ConnectionLifecycleStateFact::Connected,
            "transportAwaitRecoveryKeyframe",
            320.0,
        );
        let commands = policy.on_snapshot(&snapshot);
        assert!(commands
            .iter()
            .any(|command| matches!(command, TransportCommand::RequestKeyframe { .. })));

        let stats = runtime_stats.lock().expect("runtime stats lock");
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(
            ledger.input_signal,
            "transportAwaitRecoveryKeyframe:transportAwaitRecoveryKeyframe"
        );
        assert_eq!(ledger.action_selected, "requestKeyframe");
        assert_eq!(ledger.gate_result, "pass");
        assert!(ledger.budget_before.is_some());
        assert!(ledger.budget_after.is_some());
        assert_eq!(ledger.command_result, None);
    }

    #[test]
    fn recovery_decision_ledger_updates_when_proposal_is_none_even_if_previous_is_pending() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());

        let first = build_snapshot(
            ConnectionLifecycleStateFact::Connected,
            "transportAwaitRecoveryKeyframe",
            320.0,
        );
        let first_commands = policy.on_snapshot(&first);
        assert!(first_commands
            .iter()
            .any(|command| matches!(command, TransportCommand::RequestKeyframe { .. })));
        let first_decision_id = runtime_stats
            .lock()
            .expect("runtime stats lock")
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger")
            .decision_id;

        // 下一 tick 明确无恢复信号时，也必须写入新的 ledger，保证观测连续完整。
        let second = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 340.0);
        let _ = policy.on_snapshot(&second);
        let stats = runtime_stats.lock().expect("runtime stats lock");
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_ne!(ledger.decision_id, first_decision_id);
        assert_eq!(ledger.input_signal, "none");
        assert_eq!(ledger.gate_result, "no-signal");
        assert_eq!(ledger.action_selected, "none");
    }

    #[test]
    fn high_no_pending_but_fresh_present_does_not_force_keyframe() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        if let Ok(mut stats) = runtime_stats.lock() {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.host_no_pending_pressure_level = Some("high".to_string());
            stats.host_no_pending_streak = 88;
            stats.latest_video_host_present_time_ms = Some(now_ms - 14.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 10.0);
            stats.video_renderer_stalled = Some(false);
        }
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
        let snapshot = build_snapshot(
            ConnectionLifecycleStateFact::Connected,
            "waitKeyframeEntered",
            220.0,
        );
        let commands = policy.on_snapshot(&snapshot);
        assert!(commands
            .iter()
            .all(|command| !matches!(command, TransportCommand::RequestKeyframe { .. })));
    }

    #[test]
    fn critical_display_supply_uses_recovery_controller_budget() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        if let Ok(mut stats) = runtime_stats.lock() {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 220;
            stats.latest_video_host_present_time_ms = Some(now_ms - 980.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 520.0);
            stats.video_renderer_stalled = Some(true);
        }
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
        let mut snapshot = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 360.0);
        let first = policy.on_snapshot(&snapshot);
        assert!(first
            .iter()
            .any(|command| matches!(command, TransportCommand::RequestKeyframe { .. })));

        snapshot.version = 2;
        snapshot.now_ms = 361.0;
        snapshot.recovery.last_observed_at_ms = Some(361.0);
        let second = policy.on_snapshot(&snapshot);
        assert!(
            second
                .iter()
                .all(|command| !matches!(command, TransportCommand::RequestKeyframe { .. })),
            "second snapshot should be suppressed by escalation cooldown budget"
        );
    }

    #[test]
    fn owner_contract_is_persisted_to_runtime_stats() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        if let Ok(mut stats) = runtime_stats.lock() {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 240;
            stats.latest_video_host_present_time_ms = Some(now_ms - 1000.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 540.0);
            stats.video_renderer_stalled = Some(true);
        }
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
        let snapshot = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 450.0);
        let _ = policy.on_snapshot(&snapshot);
        let stats = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(stats.video_owner_state.as_deref(), Some("supply-starved"));
        assert_eq!(stats.video_owner_source.as_deref(), Some("supply"));
        assert_eq!(
            stats.video_owner_reason.as_deref(),
            Some("displaySupplyCritical")
        );
        assert_eq!(stats.video_owner_observed_at_ms, Some(450.0));
        assert_eq!(
            stats.baseline_remote_profile.as_deref(),
            Some("homeLanGaming")
        );
        assert_eq!(
            stats.recovery_policy_profile.as_deref(),
            Some("homeLanGaming")
        );
        assert_eq!(
            stats.dynamic_remote_subprofile.as_deref(),
            Some("displayConstrained")
        );
        assert_eq!(
            stats.effective_remote_profile_label.as_deref(),
            Some("homeLanGaming+displayConstrained")
        );
    }

    #[test]
    fn recovery_intent_is_suppressed_within_same_epoch_via_coordinator_chain() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        if let Ok(mut stats) = runtime_stats.lock() {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.transport_recovery_epoch = 2;
            stats.transport_recovery_epoch_at_last_escalation = 2;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 220;
            stats.latest_video_host_present_time_ms = Some(now_ms - 1200.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 600.0);
            stats.video_renderer_stalled = Some(true);
        }
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
        let mut first = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 500.0);
        first.version = 1;
        let first_cmds = policy.on_snapshot(&first);
        assert!(first_cmds
            .iter()
            .any(|command| matches!(command, TransportCommand::RequestKeyframe { .. })));

        let mut second = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 501.0);
        second.version = 2;
        let second_cmds = policy.on_snapshot(&second);
        assert!(second_cmds
            .iter()
            .all(|command| !matches!(command, TransportCommand::RequestKeyframe { .. })));
    }

    #[test]
    fn new_recovery_epoch_does_not_bypass_existing_recovery_suppression_chain() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        if let Ok(mut stats) = runtime_stats.lock() {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.transport_recovery_epoch = 3;
            stats.transport_recovery_epoch_at_last_escalation = 3;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 240;
            stats.latest_video_host_present_time_ms = Some(now_ms - 1300.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 700.0);
            stats.video_renderer_stalled = Some(true);
        }
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
        let mut first = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 700.0);
        first.version = 1;
        let first_cmds = policy.on_snapshot(&first);
        assert!(first_cmds
            .iter()
            .any(|command| matches!(command, TransportCommand::RequestKeyframe { .. })));

        let mut second = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 701.0);
        second.version = 2;
        let second_cmds = policy.on_snapshot(&second);
        assert!(second_cmds
            .iter()
            .all(|command| !matches!(command, TransportCommand::RequestKeyframe { .. })));

        if let Ok(mut stats) = runtime_stats.lock() {
            stats.transport_recovery_epoch = 4;
            stats.transport_recovery_epoch_at_last_escalation = 3;
        }
        let mut third = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 703.0);
        third.version = 3;
        let third_cmds = policy.on_snapshot(&third);
        assert!(third_cmds
            .iter()
            .all(|command| !matches!(command, TransportCommand::RequestKeyframe { .. })));
    }

    #[test]
    fn owner_contract_drives_display_supply_recovery_reason() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        if let Ok(mut stats) = runtime_stats.lock() {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 260;
            stats.latest_video_host_present_time_ms = Some(now_ms - 1200.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 600.0);
            stats.video_renderer_stalled = Some(true);
        }
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
        let snapshot = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 400.0);
        let commands = policy.on_snapshot(&snapshot);
        let reason = commands.into_iter().find_map(|cmd| match cmd {
            TransportCommand::RequestKeyframe { reason, .. } => Some(reason),
            _ => None,
        });
        assert_eq!(reason.as_deref(), Some("displaySupplyCritical"));
    }

    #[test]
    fn owner_does_not_enter_stable_serving_when_audio_only_and_no_pending_critical() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        if let Ok(mut stats) = runtime_stats.lock() {
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 980;
            stats.latest_video_host_present_time_ms = None;
            stats.latest_video_decode_ok_time_ms = None;
            stats.video_renderer_stalled = Some(false);
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "audioOnly".to_string(),
                video_width: None,
                video_height: None,
                mime_type: None,
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 0,
                video_packet_count_total: 0,
                audio_bytes_total: 128,
                observed_at_ms: 700.0,
            });
        }
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
        let snapshot = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 700.0);
        let _ = policy.on_snapshot(&snapshot);
        let stats = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(stats.video_owner_state.as_deref(), Some("supply-starved"));
    }

    #[test]
    fn owner_keeps_rebuilding_supply_when_timeline_keeps_awaiting_recovery_keyframe() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        if let Ok(mut stats) = runtime_stats.lock() {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 24;
            stats.latest_video_host_present_time_ms = Some(now_ms - 220.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 180.0);
            stats.video_renderer_stalled = Some(false);
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: None,
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 120_000,
                video_packet_count_total: 900,
                audio_bytes_total: 32_000,
                observed_at_ms: 810.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-await-recovery-keyframe".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        observed_at_ms: 810.0,
                    },
                    observed_at_ms: 810.0,
                });
        }
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
        let first = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 810.0);
        let _ = policy.on_snapshot(&first);

        let second = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 820.0);
        let _ = policy.on_snapshot(&second);
        let stats = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(
            stats.video_owner_state.as_deref(),
            Some("rebuilding-supply")
        );
        assert_eq!(
            stats.video_owner_reason.as_deref(),
            Some("transportAwaitRecoveryKeyframe")
        );
    }

    #[test]
    fn owner_anchor_reason_is_derived_from_timeline_chain_reason_not_recovery_diagnosis() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        if let Ok(mut stats) = runtime_stats.lock() {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 8;
            stats.latest_video_host_present_time_ms = Some(now_ms - 240.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 180.0);
            stats.video_renderer_stalled = Some(false);
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: None,
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 120_000,
                video_packet_count_total: 900,
                audio_bytes_total: 32_000,
                observed_at_ms: 910.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 11,
                    source_event: "frame-await-recovery-keyframe".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("awaitRecoveryKeyframe".to_string()),
                        observed_at_ms: 910.0,
                    },
                    observed_at_ms: 910.0,
                });
        }
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
        let snapshot = build_snapshot(
            ConnectionLifecycleStateFact::Connected,
            "decoderBackendFailure",
            920.0,
        );
        let _ = policy.on_snapshot(&snapshot);
        let stats = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(
            stats.video_owner_state.as_deref(),
            Some("rebuilding-supply")
        );
        assert_eq!(
            stats.video_owner_reason.as_deref(),
            Some("transportAwaitRecoveryKeyframe")
        );
    }

    #[test]
    fn owner_exits_recovering_after_recovery_completion_evidence() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());

        if let Ok(mut stats) = runtime_stats.lock() {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 30;
            stats.latest_video_host_present_time_ms = Some(now_ms - 240.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 170.0);
            stats.video_renderer_stalled = Some(false);
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: None,
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 140_000,
                video_packet_count_total: 1000,
                audio_bytes_total: 36_000,
                observed_at_ms: 900.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-complete-candidate".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        observed_at_ms: 900.0,
                    },
                    observed_at_ms: 900.0,
                });
        }
        let recovering = build_snapshot(
            ConnectionLifecycleStateFact::Connected,
            "transportAwaitRecoveryKeyframe",
            900.0,
        );
        let _ = policy.on_snapshot(&recovering);

        if let Ok(mut stats) = runtime_stats.lock() {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(now_ms - 18.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 12.0);
            stats.video_anchor_clean_epoch = Some(stats.transport_recovery_epoch);
            stats.video_anchor_clean_observed_at_ms = Some(now_ms - 15.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-keyframe-submitted".to_string());
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.source_event = "frame-observed".to_string();
                timeline.chain.state = "healthy".to_string();
            }
        }
        let healed = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 930.0);
        let _ = policy.on_snapshot(&healed);
        let stats = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(stats.video_owner_state.as_deref(), Some("stable-serving"));
        assert_eq!(stats.video_owner_reason.as_deref(), Some("steady"));
    }

    #[test]
    fn frame_observed_without_clean_anchor_fact_cannot_exit_recovering() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());

        if let Ok(mut stats) = runtime_stats.lock() {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 30;
            stats.latest_video_host_present_time_ms = Some(now_ms - 240.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 170.0);
            stats.video_renderer_stalled = Some(false);
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: None,
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 140_000,
                video_packet_count_total: 1000,
                audio_bytes_total: 36_000,
                observed_at_ms: 900.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-await-recovery-keyframe".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                        observed_at_ms: 900.0,
                    },
                    observed_at_ms: 900.0,
                });
        }
        let recovering = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 900.0);
        let _ = policy.on_snapshot(&recovering);

        if let Ok(mut stats) = runtime_stats.lock() {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(now_ms - 18.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 12.0);
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.source_event = "frame-observed".to_string();
                timeline.chain.state = "healthy".to_string();
            }
        }
        let healed = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 930.0);
        let _ = policy.on_snapshot(&healed);
        let stats = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(
            stats.video_owner_state.as_deref(),
            Some("rebuilding-supply")
        );
    }

    #[test]
    fn clean_anchor_healthy_chain_waits_for_host_present_freshness_before_closing_recovery() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());

        if let Ok(mut stats) = runtime_stats.lock() {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 24;
            stats.latest_video_host_present_time_ms = Some(now_ms - 260.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 180.0);
            stats.video_renderer_stalled = Some(false);
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: None,
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 155_000,
                video_packet_count_total: 1200,
                audio_bytes_total: 42_000,
                observed_at_ms: 900.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-await-recovery-keyframe".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                        observed_at_ms: 900.0,
                    },
                    observed_at_ms: 900.0,
                });
        }
        let recovering = build_snapshot(
            ConnectionLifecycleStateFact::Connected,
            "transportAwaitRecoveryKeyframe",
            900.0,
        );
        let _ = policy.on_snapshot(&recovering);

        if let Ok(mut stats) = runtime_stats.lock() {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = None;
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 9.0);
            stats.video_anchor_clean_epoch = Some(stats.transport_recovery_epoch);
            stats.video_anchor_clean_observed_at_ms = Some(now_ms - 10.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-keyframe-submitted".to_string());
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.source_event = "frame-observed".to_string();
                timeline.chain.state = "healthy".to_string();
                timeline.chain.reason = None;
            }
        }
        let waiting_present =
            build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 930.0);
        let _ = policy.on_snapshot(&waiting_present);
        {
            let stats = runtime_stats.lock().expect("runtime stats lock");
            assert_eq!(
                stats.video_owner_state.as_deref(),
                Some("rebuilding-supply")
            );
        }

        if let Ok(mut stats) = runtime_stats.lock() {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.latest_video_host_present_time_ms = Some(now_ms - 12.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 8.0);
        }
        let healed = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 960.0);
        let _ = policy.on_snapshot(&healed);
        let stats = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(stats.video_owner_state.as_deref(), Some("stable-serving"));
        assert_eq!(stats.video_owner_reason.as_deref(), Some("steady"));
    }

    #[test]
    fn frame_complete_candidate_without_clean_anchor_fact_can_exit_recovering() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());

        if let Ok(mut stats) = runtime_stats.lock() {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 30;
            stats.latest_video_host_present_time_ms = Some(now_ms - 240.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 170.0);
            stats.video_renderer_stalled = Some(false);
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: None,
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 140_000,
                video_packet_count_total: 1000,
                audio_bytes_total: 36_000,
                observed_at_ms: 900.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-await-recovery-keyframe".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                        observed_at_ms: 900.0,
                    },
                    observed_at_ms: 900.0,
                });
        }
        let recovering = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 900.0);
        let _ = policy.on_snapshot(&recovering);

        if let Ok(mut stats) = runtime_stats.lock() {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(now_ms - 18.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 12.0);
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.source_event = "frame-complete-candidate".to_string();
                timeline.chain.state = "healthy".to_string();
            }
        }
        let healed = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 930.0);
        let _ = policy.on_snapshot(&healed);
        let stats = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(stats.video_owner_state.as_deref(), Some("stable-serving"));
        assert_eq!(stats.video_owner_reason.as_deref(), Some("steady"));
    }

    #[test]
    fn lifecycle_recovering_clears_stale_clean_anchor_fact() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        if let Ok(mut stats) = runtime_stats.lock() {
            stats.video_anchor_clean_epoch = Some(5);
            stats.video_anchor_clean_observed_at_ms = Some(1000.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-keyframe-submitted".to_string());
        }
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::Recovering;
        let snapshot = TransportSnapshot::new(
            1,
            1100.0,
            connection,
            MediaProjection {
                frame_count: 1,
                ..MediaProjection::default()
            },
            RecoveryProjection {
                latest_diagnosis_label: Some("rtcConnectionRecovering".to_string()),
                pending_action: false,
                successful_action_count: 0,
                failed_action_count: 0,
                last_observed_at_ms: Some(1100.0),
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let _ = policy.on_snapshot(&snapshot);
        let stats = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(stats.video_anchor_clean_epoch, None);
        assert_eq!(stats.video_anchor_clean_observed_at_ms, None);
        assert_eq!(stats.video_anchor_clean_source_event, None);
    }

    #[test]
    fn lifecycle_recovering_preserves_current_clean_anchor_fact() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        if let Ok(mut stats) = runtime_stats.lock() {
            stats.transport_recovery_epoch = 5;
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 5,
                    source_event: "frame-await-recovery-keyframe".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                        observed_at_ms: 1095.0,
                    },
                    observed_at_ms: 1095.0,
                });
        }
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::Recovering;
        let first_snapshot = TransportSnapshot::new(
            1,
            1100.0,
            connection,
            MediaProjection {
                frame_count: 1,
                ..MediaProjection::default()
            },
            RecoveryProjection {
                latest_diagnosis_label: Some("rtcConnectionRecovering".to_string()),
                pending_action: false,
                successful_action_count: 0,
                failed_action_count: 0,
                last_observed_at_ms: Some(1100.0),
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let _ = policy.on_snapshot(&first_snapshot);
        let current_epoch = {
            let stats = runtime_stats.lock().expect("runtime stats lock");
            assert_eq!(
                stats.video_owner_state.as_deref(),
                Some("rebuilding-supply")
            );
            stats.transport_recovery_epoch
        };
        if let Ok(mut stats) = runtime_stats.lock() {
            stats.video_anchor_clean_epoch = Some(current_epoch);
            stats.video_anchor_clean_observed_at_ms = Some(1200.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-keyframe-submitted".to_string());
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(1290.0);
            stats.latest_video_decode_ok_time_ms = Some(1292.0);
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 42_000,
                video_packet_count_total: 420,
                audio_bytes_total: 0,
                observed_at_ms: 1294.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 6,
                    source_event: "frame-observed".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        observed_at_ms: 1295.0,
                    },
                    observed_at_ms: 1295.0,
                });
        }
        let second_snapshot = TransportSnapshot::new(
            1,
            1300.0,
            ConnectionProjection {
                lifecycle_state: ConnectionLifecycleStateFact::Recovering,
                ..ConnectionProjection::default()
            },
            MediaProjection {
                frame_count: 1,
                ..MediaProjection::default()
            },
            RecoveryProjection {
                latest_diagnosis_label: Some("rtcConnectionRecovering".to_string()),
                pending_action: false,
                successful_action_count: 0,
                failed_action_count: 0,
                last_observed_at_ms: Some(1300.0),
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let _ = policy.on_snapshot(&second_snapshot);
        let stats = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(stats.transport_recovery_epoch, current_epoch);
        assert_eq!(stats.video_anchor_clean_epoch, Some(current_epoch));
        assert_eq!(stats.video_anchor_clean_observed_at_ms, Some(1200.0));
        assert_eq!(
            stats.video_anchor_clean_source_event.as_deref(),
            Some("chain-clean-keyframe-submitted")
        );
    }

    #[test]
    fn display_supply_thresholds_differ_between_home_and_cloud_profiles() {
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        let base = XbxEngineMediaRuntimeStats {
            host_no_pending_pressure_level: Some("critical".to_string()),
            host_no_pending_streak: 100,
            latest_video_host_present_time_ms: Some(now_ms - 630.0),
            latest_video_decode_ok_time_ms: Some(now_ms - 340.0),
            video_renderer_stalled: Some(false),
            ..XbxEngineMediaRuntimeStats::default()
        };
        let cloud_stats = XbxEngineMediaRuntimeStats {
            session_target_type: Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud),
            ..base.clone()
        };
        let home_stats = XbxEngineMediaRuntimeStats {
            session_target_type: Some(xbxengine_protocol::XbxEngineTargetTypeDto::Home),
            transport_path: Some("direct".to_string()),
            ..base
        };

        assert_eq!(
            classify_supply_state_with_profile(&cloud_stats),
            crate::transport::rtc::policy::display_supply::DisplaySupplyState::Degraded
        );
        assert_eq!(
            classify_supply_state_with_profile(&home_stats),
            crate::transport::rtc::policy::display_supply::DisplaySupplyState::Degraded
        );
    }

    #[test]
    fn decoder_backend_failure_can_emit_decoder_reset_command() {
        let mut policy = RtcSessionPolicy::default();
        let snapshot = build_snapshot(
            ConnectionLifecycleStateFact::Connected,
            "decoderBackendFailure",
            180.0,
        );
        let commands = policy.on_snapshot(&snapshot);
        assert!(commands
            .iter()
            .any(|command| matches!(command, TransportCommand::RequestDecoderReset { .. })));
    }

    #[test]
    fn bwe_tick_emits_target_remb_update_when_metrics_are_healthy() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        if let Ok(mut config) = runtime_config.lock() {
            config.webrtc.bwe_mode = "observed-remb".to_string();
        }
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
        connection.latest_loss_ratio_1s = Some(0.01);
        connection.latest_rtt_ms = Some(40.0);
        connection.latest_transport_path = Some("udp-direct".to_string());
        let bwe = BweProjection {
            latest_rtt_ms: Some(40.0),
            latest_loss_ratio_1s: Some(0.01),
            latest_actual_video_bitrate_kbps: Some(16_000.0),
            latest_observed_remb_kbps: Some(20_000),
            latest_transport_path: Some("udp-direct".to_string()),
            latest_sample_tick_ms: Some(300.0),
            target_remb_kbps: Some(16_000),
            last_observed_at_ms: Some(300.0),
        };
        let snapshot = TransportSnapshot::new(
            1,
            300.0,
            connection,
            MediaProjection::default(),
            RecoveryProjection::default(),
            bwe,
            DiagnosticsProjection::default(),
        );
        let commands = policy.on_snapshot(&snapshot);
        let command = commands
            .into_iter()
            .find_map(|command| {
                if let TransportCommand::SetTargetRembKbps { target_kbps, .. } = command {
                    Some(target_kbps)
                } else {
                    None
                }
            })
            .unwrap_or(0);
        assert!(command > 16_000);
    }

    #[test]
    fn runtime_config_floor_is_respected() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        if let Ok(mut config) = runtime_config.lock() {
            config.webrtc.bwe_mode = "observed-remb".to_string();
            config.webrtc.remb_floor_kbps = 25_000;
            config.webrtc.remb_ceiling_kbps = 150_000;
        }
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
        connection.latest_loss_ratio_1s = Some(0.0);
        connection.latest_rtt_ms = Some(35.0);
        connection.latest_transport_path = Some("Direct".to_string());
        let bwe = BweProjection {
            latest_rtt_ms: Some(35.0),
            latest_loss_ratio_1s: Some(0.0),
            latest_actual_video_bitrate_kbps: Some(14_000.0),
            latest_observed_remb_kbps: Some(16_000),
            latest_transport_path: Some("Direct".to_string()),
            latest_sample_tick_ms: Some(400.0),
            target_remb_kbps: Some(12_000),
            last_observed_at_ms: Some(400.0),
        };
        let snapshot = TransportSnapshot::new(
            2,
            400.0,
            connection,
            MediaProjection::default(),
            RecoveryProjection::default(),
            bwe,
            DiagnosticsProjection::default(),
        );
        let target = policy
            .on_snapshot(&snapshot)
            .into_iter()
            .find_map(|command| {
                if let TransportCommand::SetTargetRembKbps { target_kbps, .. } = command {
                    Some(target_kbps)
                } else {
                    None
                }
            })
            .unwrap_or(0);
        assert_eq!(target, 25_000);
    }

    #[test]
    fn session_target_type_and_twcc_input_flow_into_new_bwe_policy() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        if let Ok(mut config) = runtime_config.lock() {
            config.webrtc.bwe_mode = "twcc-gcc".to_string();
            config.webrtc.remb_floor_kbps = 8_000;
            config.webrtc.remb_ceiling_kbps = 150_000;
        }
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        if let Ok(mut stats) = runtime_stats.lock() {
            stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud);
            stats.latest_video_twcc_observation = Some(XbxEngineVideoTwccObservation {
                observation_id: 1,
                source: "local-feedback".to_string(),
                feedback_packet_count: 3,
                covered_sequence_start: 100,
                covered_sequence_end: 120,
                covered_sequence_span: 20,
                observed_packet_count: 20,
                observed_byte_count: 30_000,
                coverage_ratio: None,
                ledger_hit_ratio: None,
                feedback_interval_ms: Some(80.0),
                arrival_span_ms: Some(70.0),
                receive_bitrate_kbps: Some(28_000.0),
                twcc_sample_valid: true,

                twcc_invalid_reason: None,

                quality: crate::XbxEngineTwccObservationQuality::Stable,
                delivery_ratio: 0.995,
                packet_loss_ratio: 0.0,
                observed_at_ms: 10.0,
            });
            stats.session_phase = Some("steady".to_string());
        }
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
        connection.latest_loss_ratio_1s = Some(0.0);
        connection.latest_rtt_ms = Some(40.0);
        connection.latest_transport_path = Some("Direct".to_string());
        let bwe = BweProjection {
            latest_rtt_ms: Some(40.0),
            latest_loss_ratio_1s: Some(0.0),
            latest_actual_video_bitrate_kbps: Some(18_000.0),
            latest_observed_remb_kbps: Some(28_000),
            latest_transport_path: Some("Direct".to_string()),
            latest_sample_tick_ms: Some(1.0),
            target_remb_kbps: Some(18_000),
            last_observed_at_ms: Some(1.0),
        };
        let snapshot = TransportSnapshot::new(
            1,
            1.0,
            connection,
            MediaProjection::default(),
            RecoveryProjection::default(),
            bwe,
            DiagnosticsProjection::default(),
        );
        let reason = policy
            .on_snapshot(&snapshot)
            .into_iter()
            .find_map(|command| {
                if let TransportCommand::SetTargetRembKbps { reason, .. } = command {
                    Some(reason)
                } else {
                    None
                }
            });
        assert!(reason.is_some_and(|value| value.starts_with("twcc-gcc-cloud-")));
    }

    #[test]
    fn cloud_builder_configured_warmup_blocks_bwe_update() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        if let Ok(mut config) = runtime_config.lock() {
            config.webrtc.bwe_mode = "twcc-gcc".to_string();
            config.webrtc.remb_floor_kbps = 8_000;
            config.webrtc.remb_ceiling_kbps = 150_000;
        }
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        if let Ok(mut stats) = runtime_stats.lock() {
            stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud);
            stats.session_phase = Some("recovering".to_string());
            stats.latest_rtc_builder_observation = Some(crate::XbxEngineRtcBuilderObservation {
                observation_id: 1,
                controlled_twcc_registry: true,
                feedback_interval_ms: 100.0,
                registered_header_extensions: vec!["video:transport-cc".to_string()],
                registered_rtcp_feedback: vec!["video:transport-cc".to_string()],
                observed_at_ms: 10.0,
            });
        }
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
        connection.latest_loss_ratio_1s = Some(0.0);
        connection.latest_rtt_ms = Some(40.0);
        connection.latest_transport_path = Some("Direct".to_string());
        let bwe = BweProjection {
            latest_rtt_ms: Some(40.0),
            latest_loss_ratio_1s: Some(0.0),
            latest_actual_video_bitrate_kbps: Some(18_000.0),
            latest_observed_remb_kbps: Some(28_000),
            latest_transport_path: Some("Direct".to_string()),
            latest_sample_tick_ms: Some(1.0),
            target_remb_kbps: Some(18_000),
            last_observed_at_ms: Some(1.0),
        };
        let snapshot = TransportSnapshot::new(
            1,
            1.0,
            connection,
            MediaProjection::default(),
            RecoveryProjection::default(),
            bwe,
            DiagnosticsProjection::default(),
        );

        let commands = policy.on_snapshot(&snapshot);
        assert!(!commands
            .iter()
            .any(|command| matches!(command, TransportCommand::SetTargetRembKbps { .. })));
    }

    #[test]
    fn cloud_valid_local_feedback_restores_bwe_after_warmup() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        if let Ok(mut config) = runtime_config.lock() {
            config.webrtc.bwe_mode = "twcc-gcc".to_string();
            config.webrtc.remb_floor_kbps = 8_000;
            config.webrtc.remb_ceiling_kbps = 150_000;
        }
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        if let Ok(mut stats) = runtime_stats.lock() {
            stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud);
            stats.session_phase = Some("steady".to_string());
            stats.latest_rtc_builder_observation = Some(crate::XbxEngineRtcBuilderObservation {
                observation_id: 1,
                controlled_twcc_registry: true,
                feedback_interval_ms: 100.0,
                registered_header_extensions: vec!["video:transport-cc".to_string()],
                registered_rtcp_feedback: vec!["video:transport-cc".to_string()],
                observed_at_ms: 10.0,
            });
            stats.latest_video_twcc_observation = Some(XbxEngineVideoTwccObservation {
                observation_id: 2,
                source: "local-feedback".to_string(),
                feedback_packet_count: 3,
                covered_sequence_start: 100,
                covered_sequence_end: 120,
                covered_sequence_span: 20,
                observed_packet_count: 20,
                observed_byte_count: 30_000,
                coverage_ratio: None,
                ledger_hit_ratio: None,
                feedback_interval_ms: Some(80.0),
                arrival_span_ms: Some(70.0),
                receive_bitrate_kbps: Some(28_000.0),
                twcc_sample_valid: true,
                twcc_invalid_reason: None,
                quality: crate::XbxEngineTwccObservationQuality::Stable,
                delivery_ratio: 0.995,
                packet_loss_ratio: 0.0,
                observed_at_ms: 10.0,
            });
        }
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
        connection.latest_loss_ratio_1s = Some(0.0);
        connection.latest_rtt_ms = Some(40.0);
        connection.latest_transport_path = Some("Direct".to_string());
        let bwe = BweProjection {
            latest_rtt_ms: Some(40.0),
            latest_loss_ratio_1s: Some(0.0),
            latest_actual_video_bitrate_kbps: Some(18_000.0),
            latest_observed_remb_kbps: Some(28_000),
            latest_transport_path: Some("Direct".to_string()),
            latest_sample_tick_ms: Some(1.0),
            target_remb_kbps: Some(18_000),
            last_observed_at_ms: Some(1.0),
        };
        let snapshot = TransportSnapshot::new(
            1,
            1.0,
            connection,
            MediaProjection::default(),
            RecoveryProjection::default(),
            bwe,
            DiagnosticsProjection::default(),
        );

        let commands = policy.on_snapshot(&snapshot);
        assert!(commands
            .iter()
            .any(|command| matches!(command, TransportCommand::SetTargetRembKbps { .. })));
    }

    #[test]
    fn cloud_builder_configured_warmup_holds_media_reconnect_candidate() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        if let Ok(mut stats) = runtime_stats.lock() {
            stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud);
            stats.latest_rtc_builder_observation = Some(crate::XbxEngineRtcBuilderObservation {
                observation_id: 1,
                controlled_twcc_registry: true,
                feedback_interval_ms: 100.0,
                registered_header_extensions: vec!["video:transport-cc".to_string()],
                registered_rtcp_feedback: vec!["video:transport-cc".to_string()],
                observed_at_ms: 1_000.0,
            });
            stats.video_renderer_stalled = Some(true);
            stats.latest_video_host_present_time_ms = Some(0.0);
            stats.latest_video_decoder_reset_time_ms = Some(2_000.0);
        }
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
        let first = build_snapshot(
            ConnectionLifecycleStateFact::Connected,
            "transportAwaitRecoveryKeyframe",
            1_000.0,
        );
        let _ = policy.on_snapshot(&first);

        let second = build_snapshot(
            ConnectionLifecycleStateFact::Connected,
            "transportAwaitRecoveryKeyframe",
            8_000.0,
        );
        let commands = policy.on_snapshot(&second);
        assert!(commands.iter().all(|command| {
            !matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));
        let stats = runtime_stats.lock().expect("runtime stats lock");
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(ledger.action_selected, "cooldownSuppressed");
    }

    #[test]
    fn cloud_builder_configured_warmup_does_not_block_lifecycle_reconnect() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        if let Ok(mut stats) = runtime_stats.lock() {
            stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud);
            stats.latest_rtc_builder_observation = Some(crate::XbxEngineRtcBuilderObservation {
                observation_id: 1,
                controlled_twcc_registry: true,
                feedback_interval_ms: 100.0,
                registered_header_extensions: vec!["video:transport-cc".to_string()],
                registered_rtcp_feedback: vec!["video:transport-cc".to_string()],
                observed_at_ms: 100.0,
            });
        }
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::Connecting;
        let recovery = RecoveryProjection {
            latest_diagnosis_label: Some("none".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(100.0),
        };
        let first = TransportSnapshot::new(
            1,
            100.0,
            connection.clone(),
            MediaProjection::default(),
            recovery.clone(),
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let _ = policy.on_snapshot(&first);
        let second = TransportSnapshot::new(
            2,
            35_600.0,
            connection,
            MediaProjection::default(),
            RecoveryProjection {
                last_observed_at_ms: Some(35_600.0),
                ..recovery
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let commands = policy.on_snapshot(&second);
        assert!(commands.iter().any(|command| {
            matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));
    }

    #[test]
    fn cloud_builder_configured_uses_more_relaxed_lifecycle_reconnect_interval_than_missing_feedback(
    ) {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        if let Ok(mut stats) = runtime_stats.lock() {
            stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud);
            stats.latest_rtc_builder_observation = Some(crate::XbxEngineRtcBuilderObservation {
                observation_id: 1,
                controlled_twcc_registry: true,
                feedback_interval_ms: 100.0,
                registered_header_extensions: vec!["video:transport-cc".to_string()],
                registered_rtcp_feedback: vec!["video:transport-cc".to_string()],
                observed_at_ms: 100.0,
            });
        }
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::Recovering;
        let media = MediaProjection {
            frame_count: 1,
            ..MediaProjection::default()
        };
        let recovery = RecoveryProjection {
            latest_diagnosis_label: Some("rtcPeerConnectionFailed".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(100.0),
        };

        let first = TransportSnapshot::new(
            1,
            100.0,
            connection.clone(),
            media.clone(),
            recovery.clone(),
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let first_commands = policy.on_snapshot(&first);
        assert!(first_commands.iter().any(|command| {
            matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));

        if let Ok(mut stats) = runtime_stats.lock() {
            stats.latest_twcc_remote_stream_observation =
                Some(crate::XbxEngineTwccRemoteStreamObservation {
                    observation_id: 2,
                    ssrc: 42,
                    mime_type: "video/H264".to_string(),
                    twcc_ext_id: Some(7),
                    header_extensions: vec!["transport-cc#7".to_string()],
                    rtcp_feedback: vec!["transport-cc:".to_string()],
                    observed_at_ms: 200.0,
                });
        }
        let second = TransportSnapshot::new(
            2,
            3_200.0,
            connection.clone(),
            media.clone(),
            RecoveryProjection {
                last_observed_at_ms: Some(3_200.0),
                ..recovery.clone()
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let second_commands = policy.on_snapshot(&second);
        assert!(second_commands.iter().all(|command| {
            !matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));

        let third = TransportSnapshot::new(
            3,
            3_800.0,
            connection,
            media,
            RecoveryProjection {
                last_observed_at_ms: Some(3_800.0),
                ..recovery
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let third_commands = policy.on_snapshot(&third);
        assert!(third_commands.iter().any(|command| {
            matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));
    }

    #[test]
    fn cloud_local_feedback_ready_restores_default_cloud_reconnect_interval() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        if let Ok(mut stats) = runtime_stats.lock() {
            stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud);
            stats.latest_rtc_builder_observation = Some(crate::XbxEngineRtcBuilderObservation {
                observation_id: 1,
                controlled_twcc_registry: true,
                feedback_interval_ms: 100.0,
                registered_header_extensions: vec!["video:transport-cc".to_string()],
                registered_rtcp_feedback: vec!["video:transport-cc".to_string()],
                observed_at_ms: 100.0,
            });
            stats.latest_video_twcc_observation = Some(XbxEngineVideoTwccObservation {
                observation_id: 2,
                source: "local-feedback".to_string(),
                feedback_packet_count: 3,
                covered_sequence_start: 100,
                covered_sequence_end: 120,
                covered_sequence_span: 20,
                observed_packet_count: 20,
                observed_byte_count: 30_000,
                coverage_ratio: None,
                ledger_hit_ratio: None,
                feedback_interval_ms: Some(80.0),
                arrival_span_ms: Some(70.0),
                receive_bitrate_kbps: Some(28_000.0),
                twcc_sample_valid: true,
                twcc_invalid_reason: None,
                quality: crate::XbxEngineTwccObservationQuality::Stable,
                delivery_ratio: 0.995,
                packet_loss_ratio: 0.0,
                observed_at_ms: 100.0,
            });
        }
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::Recovering;
        let media = MediaProjection {
            frame_count: 1,
            ..MediaProjection::default()
        };
        let recovery = RecoveryProjection {
            latest_diagnosis_label: Some("rtcPeerConnectionFailed".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(100.0),
        };

        let first = TransportSnapshot::new(
            1,
            100.0,
            connection.clone(),
            media.clone(),
            recovery.clone(),
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let first_commands = policy.on_snapshot(&first);
        assert!(first_commands.iter().any(|command| {
            matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));

        let second = TransportSnapshot::new(
            2,
            2_000.0,
            connection.clone(),
            media.clone(),
            RecoveryProjection {
                last_observed_at_ms: Some(2_000.0),
                ..recovery.clone()
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let second_commands = policy.on_snapshot(&second);
        assert!(second_commands.iter().all(|command| {
            !matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));

        let third = TransportSnapshot::new(
            3,
            2_700.0,
            connection,
            media,
            RecoveryProjection {
                last_observed_at_ms: Some(2_700.0),
                ..recovery
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let third_commands = policy.on_snapshot(&third);
        assert!(third_commands.iter().any(|command| {
            matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));
    }

    #[test]
    fn bwe_emits_reason_update_even_when_target_is_unchanged() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        if let Ok(mut config) = runtime_config.lock() {
            config.webrtc.bwe_mode = "twcc-gcc".to_string();
            config.webrtc.remb_floor_kbps = 8_000;
            config.webrtc.remb_ceiling_kbps = 50_000;
            config.webrtc.video_pipeline.feedback_interval_ms = 1_000;
        }
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        if let Ok(mut stats) = runtime_stats.lock() {
            stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud);
            stats.session_phase = Some("steady".to_string());
        }
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
        policy.last_sent_remb_kbps = 25_000;
        policy.last_bwe_reason = Some("twcc-gcc-cloud-await-feedback".to_string());

        if let Ok(mut stats) = runtime_stats.lock() {
            stats.latest_video_twcc_observation = Some(XbxEngineVideoTwccObservation {
                observation_id: 1,
                source: "local-feedback".to_string(),
                feedback_packet_count: 3,
                covered_sequence_start: 100,
                covered_sequence_end: 220,
                covered_sequence_span: 120,
                observed_packet_count: 120,
                observed_byte_count: 180_000,
                coverage_ratio: None,
                ledger_hit_ratio: None,
                feedback_interval_ms: Some(1_000.0),
                arrival_span_ms: Some(1_000.0),
                receive_bitrate_kbps: Some(24_500.0),
                twcc_sample_valid: true,

                twcc_invalid_reason: None,

                quality: crate::XbxEngineTwccObservationQuality::Stable,
                delivery_ratio: 1.0,
                packet_loss_ratio: 0.0,
                observed_at_ms: 10.0,
            });
        }

        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
        connection.latest_loss_ratio_1s = Some(0.0);
        connection.latest_rtt_ms = Some(40.0);
        connection.latest_transport_path = Some("Direct".to_string());
        let bwe = BweProjection {
            latest_rtt_ms: Some(40.0),
            latest_loss_ratio_1s: Some(0.0),
            latest_actual_video_bitrate_kbps: Some(18_000.0),
            latest_observed_remb_kbps: Some(25_000),
            latest_transport_path: Some("Direct".to_string()),
            latest_sample_tick_ms: Some(1.0),
            target_remb_kbps: Some(25_000),
            last_observed_at_ms: Some(1.0),
        };
        let snapshot = TransportSnapshot::new(
            1,
            1.0,
            connection,
            MediaProjection::default(),
            RecoveryProjection::default(),
            bwe,
            DiagnosticsProjection::default(),
        );

        let reason = policy
            .on_snapshot(&snapshot)
            .into_iter()
            .find_map(|command| {
                if let TransportCommand::SetTargetRembKbps { reason, .. } = command {
                    Some(reason)
                } else {
                    None
                }
            });

        assert!(reason.is_some());
        assert_ne!(reason.as_deref(), Some("twcc-gcc-cloud-await-feedback"));
    }

    #[test]
    fn reconnect_keeps_priority_over_recovery_and_bwe() {
        let mut policy = RtcSessionPolicy::default();
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::Recovering;
        connection.latest_loss_ratio_1s = Some(0.01);
        connection.latest_rtt_ms = Some(40.0);
        let recovery = RecoveryProjection {
            latest_diagnosis_label: Some("transportAwaitRecoveryKeyframe".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(100.0),
        };
        let bwe = BweProjection {
            latest_rtt_ms: Some(40.0),
            latest_loss_ratio_1s: Some(0.01),
            latest_actual_video_bitrate_kbps: Some(12_000.0),
            latest_observed_remb_kbps: Some(18_000),
            latest_transport_path: Some("udp-direct".to_string()),
            latest_sample_tick_ms: Some(100.0),
            target_remb_kbps: Some(12_000),
            last_observed_at_ms: Some(100.0),
        };
        let snapshot = TransportSnapshot::new(
            1,
            100.0,
            connection,
            MediaProjection {
                frame_count: 1,
                ..MediaProjection::default()
            },
            recovery,
            bwe,
            DiagnosticsProjection::default(),
        );
        let commands = policy.on_snapshot(&snapshot);
        assert_eq!(commands.len(), 1);
        assert!(matches!(
            commands[0],
            TransportCommand::RequestReconnectCandidate { .. }
        ));
    }

    #[test]
    fn stale_adapter_idle_timeout_does_not_replay_during_steady_progress() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        if let Ok(mut stats) = runtime_stats.lock() {
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 7;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(988.0);
            stats.latest_video_decode_ok_time_ms = Some(992.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = Some(7);
            stats.video_anchor_clean_observed_at_ms = Some(994.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-keyframe-submitted".to_string());
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 128_000,
                video_packet_count_total: 1_600,
                audio_bytes_total: 36_000,
                observed_at_ms: 995.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-observed".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        observed_at_ms: 995.0,
                    },
                    observed_at_ms: 995.0,
                });
        }
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
        connection.control_channel_open = true;
        connection.latest_transport_path = Some("Direct".to_string());
        connection.latest_rtt_ms = Some(42.0);
        connection.last_observed_at_ms = Some(1_000.0);
        let healthy_snapshot = TransportSnapshot::new(
            1,
            1_000.0,
            connection.clone(),
            MediaProjection {
                frame_count: 31,
                ..MediaProjection::default()
            },
            RecoveryProjection {
                latest_diagnosis_label: Some("none".to_string()),
                pending_action: false,
                successful_action_count: 0,
                failed_action_count: 0,
                last_observed_at_ms: Some(999.0),
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let _ = policy.on_snapshot(&healthy_snapshot);

        let snapshot = TransportSnapshot::new(
            2,
            1_008.0,
            connection,
            MediaProjection {
                frame_count: 32,
                ..MediaProjection::default()
            },
            RecoveryProjection {
                latest_diagnosis_label: Some("adapterIdleTimeout".to_string()),
                pending_action: false,
                successful_action_count: 0,
                failed_action_count: 0,
                last_observed_at_ms: Some(900.0),
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );

        let commands = policy.on_snapshot(&snapshot);
        assert!(commands.is_empty(), "unexpected commands: {commands:?}");
        let stats = runtime_stats.lock().expect("runtime stats lock");
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(ledger.gate_result, "no-signal");
        assert_eq!(ledger.action_selected, "none");
    }

    #[test]
    fn active_adapter_idle_timeout_is_suppressed_when_render_output_is_still_fresh() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        if let Ok(mut stats) = runtime_stats.lock() {
            stats.session_phase = Some("steady".to_string());
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.transport_recovery_epoch = 4;
            stats.latest_video_host_present_time_ms = Some(930.0);
            stats.latest_video_decode_ok_time_ms = Some(948.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = Some(4);
            stats.video_anchor_clean_observed_at_ms = Some(940.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-keyframe-submitted".to_string());
        }
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
        connection.last_observed_at_ms = Some(1_000.0);
        let snapshot = TransportSnapshot::new(
            1,
            1_000.0,
            connection,
            MediaProjection {
                frame_count: 24,
                ..MediaProjection::default()
            },
            RecoveryProjection {
                latest_diagnosis_label: Some("adapterIdleTimeout".to_string()),
                pending_action: false,
                successful_action_count: 0,
                failed_action_count: 0,
                last_observed_at_ms: Some(1_000.0),
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );

        let commands = policy.on_snapshot(&snapshot);
        assert!(commands.is_empty(), "unexpected commands: {commands:?}");
        let stats = runtime_stats.lock().expect("runtime stats lock");
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(ledger.input_signal, "none");
        assert_eq!(ledger.gate_result, "no-signal");
        assert_eq!(ledger.action_selected, "none");
    }

    #[test]
    fn active_adapter_idle_timeout_still_reaches_recovery_path() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        if let Ok(mut stats) = runtime_stats.lock() {
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
        }
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
        connection.last_observed_at_ms = Some(1_000.0);
        let snapshot = TransportSnapshot::new(
            1,
            1_000.0,
            connection,
            MediaProjection::default(),
            RecoveryProjection {
                latest_diagnosis_label: Some("adapterIdleTimeout".to_string()),
                pending_action: false,
                successful_action_count: 0,
                failed_action_count: 0,
                last_observed_at_ms: Some(1_000.0),
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );

        let commands = policy.on_snapshot(&snapshot);
        assert!(commands.iter().any(|command| {
            matches!(command, TransportCommand::RequestDecoderReset { .. })
        }));
        let stats = runtime_stats.lock().expect("runtime stats lock");
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(ledger.input_signal, "adapterIdleTimeout:adapterIdleTimeout");
        assert_eq!(ledger.gate_result, "pass");
    }

    #[test]
    fn realtime_adapter_idle_timeout_is_absorbed_when_render_output_is_fresh() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        if let Ok(mut stats) = runtime_stats.lock() {
            stats.session_phase = Some("steady".to_string());
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.transport_recovery_epoch = 7;
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(996.0);
            stats.latest_video_decode_ok_time_ms = Some(997.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = Some(7);
            stats.video_anchor_clean_observed_at_ms = Some(995.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-keyframe-submitted".to_string());
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 256_000,
                video_packet_count_total: 3_200,
                audio_bytes_total: 64_000,
                observed_at_ms: 998.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-observed".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        observed_at_ms: 998.0,
                    },
                    observed_at_ms: 998.0,
                });
        }
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
        connection.control_channel_open = true;
        connection.latest_transport_path = Some("Direct".to_string());
        connection.latest_rtt_ms = Some(18.0);
        connection.last_observed_at_ms = Some(1_000.0);

        let healthy_snapshot = TransportSnapshot::new(
            1,
            1_000.0,
            connection.clone(),
            MediaProjection {
                frame_count: 48,
                ..MediaProjection::default()
            },
            RecoveryProjection {
                latest_diagnosis_label: Some("none".to_string()),
                pending_action: false,
                successful_action_count: 0,
                failed_action_count: 0,
                last_observed_at_ms: Some(999.0),
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let _ = policy.on_snapshot(&healthy_snapshot);

        let snapshot = TransportSnapshot::new(
            2,
            1_000.0,
            connection,
            MediaProjection {
                frame_count: 49,
                ..MediaProjection::default()
            },
            RecoveryProjection {
                latest_diagnosis_label: Some("adapterIdleTimeout".to_string()),
                pending_action: false,
                successful_action_count: 0,
                failed_action_count: 0,
                last_observed_at_ms: Some(1_000.0),
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );

        let commands = policy.on_snapshot(&snapshot);
        assert!(commands.is_empty(), "unexpected commands: {commands:?}");
        let stats = runtime_stats.lock().expect("runtime stats lock");
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(ledger.gate_result, "no-signal");
        assert_eq!(ledger.action_selected, "none");
    }

    #[test]
    fn connected_track_attached_without_host_feedback_does_not_escalate_adapter_idle_timeout_during_priming_window(
    ) {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        if let Ok(mut stats) = runtime_stats.lock() {
            stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud);
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 64_000,
                video_packet_count_total: 800,
                audio_bytes_total: 16_000,
                observed_at_ms: 1_000.0,
            });
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
        }
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
        connection.control_channel_open = true;
        connection.latest_transport_path = Some("Direct".to_string());
        connection.latest_rtt_ms = Some(24.0);
        connection.last_observed_at_ms = Some(5_000.0);
        let snapshot = TransportSnapshot::new(
            1,
            5_000.0,
            connection,
            MediaProjection::default(),
            RecoveryProjection {
                latest_diagnosis_label: Some("adapterIdleTimeout".to_string()),
                pending_action: false,
                successful_action_count: 0,
                failed_action_count: 0,
                last_observed_at_ms: Some(5_000.0),
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );

        let commands = policy.on_snapshot(&snapshot);
        assert!(commands.is_empty(), "unexpected commands: {commands:?}");
        let stats = runtime_stats.lock().expect("runtime stats lock");
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(ledger.gate_result, "no-signal");
        assert_eq!(ledger.action_selected, "none");
    }

    #[test]
    fn connected_track_attached_without_host_feedback_eventually_escalates_after_priming_window_expires(
    ) {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        if let Ok(mut stats) = runtime_stats.lock() {
            stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud);
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 96_000,
                video_packet_count_total: 1_200,
                audio_bytes_total: 24_000,
                observed_at_ms: 1_000.0,
            });
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
        }
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
        connection.control_channel_open = true;
        connection.latest_transport_path = Some("Direct".to_string());
        connection.latest_rtt_ms = Some(24.0);
        connection.last_observed_at_ms = Some(37_000.0);
        let snapshot = TransportSnapshot::new(
            1,
            37_000.0,
            connection,
            MediaProjection::default(),
            RecoveryProjection {
                latest_diagnosis_label: Some("adapterIdleTimeout".to_string()),
                pending_action: false,
                successful_action_count: 0,
                failed_action_count: 0,
                last_observed_at_ms: Some(37_000.0),
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );

        let commands = policy.on_snapshot(&snapshot);
        assert!(
            !commands.is_empty(),
            "priming bad window expired should enter recovery path"
        );
        let stats = runtime_stats.lock().expect("runtime stats lock");
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(ledger.input_signal, "adapterIdleTimeout:adapterIdleTimeout");
        assert_eq!(ledger.gate_result, "pass");
        assert_ne!(ledger.action_selected, "none");
    }

    #[test]
    fn unstable_hold_requires_consecutive_confirmation_before_emit() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        if let Ok(mut config) = runtime_config.lock() {
            config.webrtc.bwe_mode = "twcc-gcc".to_string();
            config.webrtc.video_pipeline.feedback_interval_ms = 100;
        }
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        if let Ok(mut stats) = runtime_stats.lock() {
            stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud);
            stats.session_phase = Some("steady".to_string());
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(now_ms - 12.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 10.0);
            stats.video_anchor_clean_epoch = Some(0);
            stats.video_anchor_clean_observed_at_ms = Some(now_ms - 8.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-keyframe-submitted".to_string());
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: None,
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 64_000,
                video_packet_count_total: 1_200,
                audio_bytes_total: 32_000,
                observed_at_ms: now_ms,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-observed".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        observed_at_ms: now_ms - 6.0,
                    },
                    observed_at_ms: now_ms - 6.0,
                });
            stats.latest_video_twcc_observation = Some(XbxEngineVideoTwccObservation {
                observation_id: 1,
                source: "local-feedback".to_string(),
                feedback_packet_count: 1,
                covered_sequence_start: 1,
                covered_sequence_end: 2,
                covered_sequence_span: 2,
                observed_packet_count: 1,
                observed_byte_count: 1200,
                coverage_ratio: None,
                ledger_hit_ratio: None,
                feedback_interval_ms: None,
                arrival_span_ms: None,
                receive_bitrate_kbps: Some(0.0),
                twcc_sample_valid: true,

                twcc_invalid_reason: None,

                quality: crate::XbxEngineTwccObservationQuality::Stable,
                delivery_ratio: 1.0,
                packet_loss_ratio: 0.0,
                observed_at_ms: 1.0,
            });
        }
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
        policy.last_sent_remb_kbps = 25_000;

        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
        connection.latest_transport_path = Some("Direct".to_string());
        let snapshot_first = TransportSnapshot::new(
            1,
            1.0,
            connection.clone(),
            MediaProjection::default(),
            RecoveryProjection::default(),
            BweProjection {
                latest_rtt_ms: Some(180.0),
                latest_loss_ratio_1s: Some(0.0),
                latest_actual_video_bitrate_kbps: Some(1_000.0),
                latest_observed_remb_kbps: Some(25_000),
                latest_transport_path: Some("Direct".to_string()),
                latest_sample_tick_ms: Some(1.0),
                target_remb_kbps: Some(25_000),
                last_observed_at_ms: Some(1.0),
            },
            DiagnosticsProjection::default(),
        );
        let first_commands = policy.on_snapshot(&snapshot_first);
        assert!(first_commands
            .iter()
            .all(|command| !matches!(command, TransportCommand::SetTargetRembKbps { .. })));

        let snapshot_second = TransportSnapshot::new(
            2,
            2.0,
            connection,
            MediaProjection::default(),
            RecoveryProjection::default(),
            BweProjection {
                latest_rtt_ms: Some(180.0),
                latest_loss_ratio_1s: Some(0.0),
                latest_actual_video_bitrate_kbps: Some(1_000.0),
                latest_observed_remb_kbps: Some(25_000),
                latest_transport_path: Some("Direct".to_string()),
                latest_sample_tick_ms: Some(2.0),
                target_remb_kbps: Some(25_000),
                last_observed_at_ms: Some(2.0),
            },
            DiagnosticsProjection::default(),
        );
        let second_commands = policy.on_snapshot(&snapshot_second);
        assert!(second_commands.iter().any(|command| {
            matches!(
                command,
                TransportCommand::SetTargetRembKbps { reason, .. }
                    if reason.contains("unstable-hold")
            )
        }));
    }

    fn build_snapshot(
        lifecycle_state: ConnectionLifecycleStateFact,
        diagnosis: &str,
        observed_at_ms: f64,
    ) -> TransportSnapshot {
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = lifecycle_state;
        let recovery = RecoveryProjection {
            latest_diagnosis_label: Some(diagnosis.to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(observed_at_ms),
        };
        TransportSnapshot::new(
            1,
            observed_at_ms,
            connection,
            MediaProjection::default(),
            recovery,
            BweProjection::default(),
            DiagnosticsProjection::default(),
        )
    }
