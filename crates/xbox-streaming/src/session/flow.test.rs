    use super::*;
    use crate::session::monitor::{SessionRuntimeBinding, SessionRuntimeSnapshot};
    use crate::session::store::SessionRuntimeRecord;

    #[derive(Clone)]
    struct Snapshot {
        session_id: String,
        session_path: String,
        target_id: String,
        target_type: String,
        runtime: SessionRuntimeSnapshot,
    }

    impl SessionRuntimeBinding for Snapshot {
        fn runtime_snapshot(&self) -> SessionRuntimeSnapshot {
            self.runtime.clone()
        }

        fn replace_runtime_snapshot(&mut self, runtime: SessionRuntimeSnapshot) {
            self.runtime = runtime;
        }
    }

    impl SessionFlowSnapshot for Snapshot {
        fn new_pending(
            session_id: String,
            session_path: String,
            target_id: String,
            target_type: String,
        ) -> Self {
            Self {
                session_id,
                session_path,
                target_id,
                target_type,
                runtime: SessionRuntimeSnapshot {
                    stream_state: None,
                    player_state: "pending".to_string(),
                    queue: None,
                    error_details: None,
                },
            }
        }

        fn session_id(&self) -> &str {
            &self.session_id
        }

        fn session_path(&self) -> &str {
            &self.session_path
        }

        fn target_id(&self) -> &str {
            &self.target_id
        }

        fn target_type(&self) -> &str {
            &self.target_type
        }
    }

    fn snapshot_with_runtime(runtime: SessionRuntimeSnapshot) -> Snapshot {
        Snapshot {
            session_id: "session-1".to_string(),
            session_path: "/v5/sessions/cloud/session-1".to_string(),
            target_id: "target-1".to_string(),
            target_type: "cloud".to_string(),
            runtime,
        }
    }

    #[test]
    fn started_player_state_maps_to_session_ready_phase() {
        let phase = resolve_session_phase(&SessionRuntimeSnapshot {
            stream_state: Some("Provisioned".to_string()),
            player_state: "started".to_string(),
            queue: None,
            error_details: None,
        });

        assert_eq!(phase, SessionPhase::SessionReady);
        assert_eq!(
            default_status_text_key(phase),
            "streamPage.status.startingPlayer"
        );
    }

    #[test]
    fn build_session_progress_snapshot_uses_session_ready_for_started_session() {
        let record = SessionRuntimeRecord::new(
            snapshot_with_runtime(SessionRuntimeSnapshot {
                stream_state: Some("Provisioned".to_string()),
                player_state: "started".to_string(),
                queue: None,
                error_details: None,
            }),
            crate::policy::Plan::default(),
            0,
        );

        let progress = build_session_progress_snapshot(record);

        assert_eq!(progress.phase, SessionPhase::SessionReady);
        assert_eq!(progress.status_text_key, "streamPage.status.startingPlayer");
    }

    #[test]
    fn restart_offer_filter_ignores_previous_answer_snapshot() {
        let answer = AnswerPayload {
            sdp: "answer-1".to_string(),
            message_type: Some("answer".to_string()),
        };

        assert_eq!(
            filter_stale_offer_response(Some(answer.clone()), Some(&answer), true),
            None
        );
        assert_eq!(
            filter_stale_offer_response(Some(answer.clone()), Some(&answer), false),
            Some(answer)
        );
    }

    #[test]
    fn restart_ice_filter_ignores_previous_candidate_snapshot() {
        let candidates = vec![IceCandidate {
            candidate: "a=candidate:1 1 UDP 1 10.0.0.1 9000 typ host".to_string(),
            ..Default::default()
        }];

        assert_eq!(
            filter_stale_ice_response(Some(candidates.clone()), Some(&candidates), true),
            None
        );
        assert_eq!(
            filter_stale_ice_response(Some(candidates.clone()), Some(&candidates), false),
            Some(candidates)
        );
    }

    #[test]
    fn waiting_for_server_registration_http_error_is_retryable_for_home() {
        let mut plan = crate::policy::Plan::default();
        plan.session.target = crate::policy::types::Target::Home;
        plan.session.schedule.ready_timeout_ms = 10_000;

        let error = xbox_webapi::WebApiError::http(
            503,
            "Streaming error: Xccs : ErrorCallingWNS : Send command failed : State WaitingForServerToRegister",
        );

        assert!(should_retry_home_server_registration(
            &plan,
            &error,
            0,
            plan.session.schedule.ready_timeout_ms,
        ));
        assert!(!should_retry_home_server_registration(
            &plan,
            &error,
            plan.session.schedule.ready_timeout_ms,
            plan.session.schedule.ready_timeout_ms,
        ));
    }

    #[test]
    fn waiting_for_server_registration_retry_does_not_apply_to_cloud() {
        let mut plan = crate::policy::Plan::default();
        plan.session.target = crate::policy::types::Target::Cloud;
        plan.session.schedule.ready_timeout_ms = 10_000;

        let error = xbox_webapi::WebApiError::http(
            503,
            "Streaming error: Xccs : ErrorCallingWNS : Send command failed : State WaitingForServerToRegister",
        );

        assert!(!should_retry_home_server_registration(
            &plan,
            &error,
            0,
            plan.session.schedule.ready_timeout_ms,
        ));
    }

    #[test]
    fn waiting_for_server_registration_message_matches_non_http_error_text() {
        assert!(is_waiting_for_server_registration_message(
            "Xccs : ErrorCallingWNS : Send command failed : State WaitingForServerToRegister",
        ));
        assert!(!is_waiting_for_server_registration_message(
            "remoteConsoleNotReady"
        ));
    }

    #[test]
    fn server_never_registered_message_is_treated_as_registration_signal() {
        assert!(is_server_registration_retry_signal("ServerNeverRegistered"));
        assert!(!is_server_registration_retry_signal(
            "streamingStartTimeout:sessionId=session-1"
        ));
    }

    #[test]
    fn retry_backoff_reuses_last_entry_after_sequence_is_exhausted() {
        assert_eq!(next_retry_backoff_ms(&[1_000, 3_000, 5_000], 0), 1_000);
        assert_eq!(next_retry_backoff_ms(&[1_000, 3_000, 5_000], 2), 5_000);
        assert_eq!(next_retry_backoff_ms(&[1_000, 3_000, 5_000], 6), 5_000);
        assert_eq!(next_retry_backoff_ms(&[], 0), 1_000);
    }

    #[test]
    fn poll_ice_returns_end_of_candidates_batch_without_blocking() {
        let end_of_candidates = vec![IceCandidate {
            candidate: "a=end-of-candidates".to_string(),
            ..Default::default()
        }];

        let result = resolve_polled_ice_result("session-1", Some(end_of_candidates.clone()), true)
            .expect("eoc batch should be returned");

        assert_eq!(result, end_of_candidates);
    }

    #[test]
    fn poll_ice_returns_empty_for_duplicate_snapshot() {
        let result = resolve_polled_ice_result("session-1", None, true)
            .expect("duplicate snapshot should collapse to empty batch");

        assert!(result.is_empty());
    }

    #[test]
    fn connected_standby_retries_wake_after_cooldown() {
        assert!(!should_retry_wake_during_ready_wait(
            Some("ConnectedStandby"),
            Some(10_000),
            14_999,
        ));
        assert!(should_retry_wake_during_ready_wait(
            Some("ConnectedStandby"),
            Some(10_000),
            15_000,
        ));
        assert!(should_retry_wake_during_ready_wait(Some("Off"), None, 0));
        assert!(!should_retry_wake_during_ready_wait(
            Some("On"),
            Some(10_000),
            20_000
        ));
    }

    #[test]
    fn remote_console_ready_requires_registration_signal_after_wake() {
        let console = RemoteConsoleSnapshot {
            power_state: Some("On".to_string()),
            console_streaming_enabled: Some(true),
            ..Default::default()
        };

        assert!(is_remote_console_power_ready(&console));
        assert!(!is_remote_console_ready(&console));
    }

    #[test]
    fn remote_console_ready_signal_reason_prefers_remote_management() {
        let console = RemoteConsoleSnapshot {
            power_state: Some("On".to_string()),
            remote_management_enabled: Some(true),
            console_streaming_enabled: Some(true),
            console_addrs_count: 1,
            ..Default::default()
        };

        assert!(is_remote_console_ready(&console));
        assert_eq!(
            remote_console_ready_reason(&console),
            Some("explicitRegistration")
        );
    }

    #[test]
    fn remote_console_ready_signal_reason_rejects_console_addrs_without_registration() {
        let console = RemoteConsoleSnapshot {
            power_state: Some("On".to_string()),
            console_streaming_enabled: Some(true),
            console_addrs_count: 1,
            ..Default::default()
        };

        assert!(!is_remote_console_ready(&console));
        assert_eq!(remote_console_ready_reason(&console), None);
    }

    #[test]
    fn remote_console_ready_reason_requires_explicit_registration_signal() {
        let console = RemoteConsoleSnapshot {
            power_state: Some("On".to_string()),
            console_streaming_enabled: Some(true),
            ..Default::default()
        };

        assert_eq!(remote_console_ready_reason(&console), None);
    }

    #[test]
    fn remote_console_wake_circuit_open_message_is_detected() {
        let error =
            remote_console_wake_circuit_open_error("console-1", Some("ConnectedStandby"), 3);
        assert!(is_remote_console_wake_circuit_open_message(&error.message));
        assert_eq!(
            error.startup_hint.as_ref().map(|hint| hint.kind.clone()),
            Some(SessionFlowStartupErrorKind::HostRemotePlayUnavailable)
        );
        assert!(!is_remote_console_wake_circuit_open_message(
            "remoteConsoleNotReady:targetId=console-1"
        ));
    }

    #[test]
    fn remote_console_not_ready_error_carries_structured_hint() {
        let error = remote_console_not_ready_error("console-1");

        assert_eq!(
            error.startup_hint.as_ref().map(|hint| hint.kind.clone()),
            Some(SessionFlowStartupErrorKind::HostRemotePlayUnavailable)
        );
        assert!(error
            .startup_hint
            .as_ref()
            .is_some_and(|hint| hint.retryable));
    }

    #[test]
    fn startup_timeout_error_carries_structured_hint() {
        let error = startup_timeout_error("session-1");

        assert_eq!(
            error.startup_hint.as_ref().map(|hint| hint.kind.clone()),
            Some(SessionFlowStartupErrorKind::SessionReady)
        );
        assert!(error
            .startup_hint
            .as_ref()
            .is_some_and(|hint| hint.retryable));
    }

    fn startup_progress(
        phase: SessionPhase,
        error_message: Option<&str>,
    ) -> SessionProgressSnapshot {
        SessionProgressSnapshot {
            session_id: "session-1".to_string(),
            phase,
            status_text_key: "key".to_string(),
            queue_seconds: None,
            queue: None,
            error_code: None,
            error_message: error_message.map(str::to_string),
            error_hint: build_session_progress_error_hint(phase, None, error_message),
        }
    }

    #[test]
    fn failed_progress_server_registration_signal_carries_structured_hint() {
        let progress = SessionProgressSnapshot {
            session_id: "session-1".to_string(),
            phase: SessionPhase::Failed,
            status_text_key: "key".to_string(),
            queue_seconds: None,
            queue: None,
            error_code: Some("ServerNeverRegistered".to_string()),
            error_message: Some(
                "Agent : ServerNeverRegistered : Server never registered with service : State WaitingForServerToRegister"
                    .to_string(),
            ),
            error_hint: build_session_progress_error_hint(
                SessionPhase::Failed,
                Some("ServerNeverRegistered"),
                Some(
                    "Agent : ServerNeverRegistered : Server never registered with service : State WaitingForServerToRegister",
                ),
            ),
        };

        assert_eq!(
            progress.error_hint.as_ref().map(|hint| hint.kind.clone()),
            Some(SessionFlowStartupErrorKind::HostRegistrationRetryExhausted)
        );
        assert!(progress
            .error_hint
            .as_ref()
            .is_some_and(|hint| !hint.retryable));
    }

    #[test]
    fn failed_progress_unknown_error_defaults_to_runtime_hint() {
        let progress = startup_progress(SessionPhase::Failed, Some("decoder pipeline stalled"));
        assert_eq!(
            progress.error_hint.as_ref().map(|hint| hint.kind.clone()),
            Some(SessionFlowStartupErrorKind::Runtime)
        );
        assert!(progress
            .error_hint
            .as_ref()
            .is_some_and(|hint| hint.retryable));
    }

    #[test]
    fn progress_without_error_has_no_structured_hint() {
        let progress = startup_progress(SessionPhase::WaitingSessionReady, None);
        assert_eq!(progress.error_hint, None);
    }

    #[test]
    fn recovering_progress_network_signal_maps_network_hint() {
        let progress = startup_progress(SessionPhase::Recovering, Some("networkLost reconnecting"));
        assert_eq!(
            progress.error_hint.as_ref().map(|hint| hint.kind.clone()),
            Some(SessionFlowStartupErrorKind::Network)
        );
        assert!(progress
            .error_hint
            .as_ref()
            .is_some_and(|hint| hint.retryable));
    }

    #[test]
    fn closed_is_treated_as_transient_when_recovering_signal_is_recent() {
        let mut last_recovery_signal_at_ms = 10_000;
        let action = decide_startup_progress_action(
            &startup_progress(SessionPhase::Closed, None),
            10_900,
            &mut last_recovery_signal_at_ms,
            1_000,
        );
        assert_eq!(
            action,
            StartupProgressAction::Continue {
                transient_closed: true
            }
        );
    }

    #[test]
    fn closed_fails_after_recovery_window_expires() {
        let mut last_recovery_signal_at_ms = 10_000;
        let action = decide_startup_progress_action(
            &startup_progress(SessionPhase::Closed, Some("closed-final")),
            12_001,
            &mut last_recovery_signal_at_ms,
            2_000,
        );
        assert_eq!(
            action,
            StartupProgressAction::Fail("closed-final".to_string())
        );
    }

    #[test]
    fn recovering_phase_refreshes_recovery_signal_timestamp() {
        let mut last_recovery_signal_at_ms = 10_000;
        let action = decide_startup_progress_action(
            &startup_progress(SessionPhase::Recovering, None),
            11_234,
            &mut last_recovery_signal_at_ms,
            2_000,
        );
        assert_eq!(
            action,
            StartupProgressAction::Continue {
                transient_closed: false
            }
        );
        assert_eq!(last_recovery_signal_at_ms, 11_234);
    }

    #[test]
    fn closed_with_reconnect_signal_stays_transient_within_window() {
        let mut last_recovery_signal_at_ms = 1_000;
        let _ = decide_startup_progress_action(
            &startup_progress(
                SessionPhase::WaitingSessionReady,
                Some("networkLost reconnecting"),
            ),
            1_500,
            &mut last_recovery_signal_at_ms,
            900,
        );
        let action = decide_startup_progress_action(
            &startup_progress(SessionPhase::Closed, None),
            2_300,
            &mut last_recovery_signal_at_ms,
            1_000,
        );
        assert_eq!(
            action,
            StartupProgressAction::Continue {
                transient_closed: true
            }
        );
    }

    #[test]
    fn home_provisioning_startup_timeout_no_longer_triggers_recreate() {
        assert_eq!(
            decide_home_session_ready_recreate_retry(
                true,
                0,
                "streamingStartTimeout:sessionId=session-1",
                Some(SessionPhase::WaitingSessionReady),
                Some("Provisioning"),
                None,
                None,
            ),
            None
        );
    }

    #[test]
    fn home_provisioning_stall_timeout_no_longer_triggers_recreate() {
        assert_eq!(
            decide_home_session_ready_recreate_retry(
                true,
                0,
                "homeProvisioningStallTimeout:sessionId=session-1;elapsedMs=10000",
                Some(SessionPhase::WaitingSessionReady),
                Some("Provisioning"),
                None,
                None,
            ),
            None
        );
    }

    #[test]
    fn failed_server_never_registered_error_code_is_exhausted_immediately() {
        assert_eq!(
            decide_home_session_ready_recreate_retry(
                true,
                0,
                "streamingStartFailed",
                Some(SessionPhase::Failed),
                Some("Failed"),
                Some("ServerNeverRegistered"),
                None,
            ),
            Some(SessionReadyRetryDecision::Exhausted(
                SessionReadyRecreateRetryReason::WaitingForServerRegistration,
            ))
        );
    }

    #[test]
    fn failed_server_registration_error_is_exhausted_immediately() {
        assert_eq!(
            decide_home_session_ready_recreate_retry(
                true,
                0,
                "Agent : ServerNeverRegistered : Server never registered with service : State WaitingForServerToRegister",
                Some(SessionPhase::Failed),
                Some("Failed"),
                None,
                Some(
                    "Agent : ServerNeverRegistered : Server never registered with service : State WaitingForServerToRegister",
                ),
            ),
            Some(SessionReadyRetryDecision::Exhausted(
                SessionReadyRecreateRetryReason::WaitingForServerRegistration,
            ))
        );
    }

    #[test]
    fn waiting_for_server_registration_retry_signal_is_exhausted_bounded_retry() {
        assert_eq!(
            decide_home_session_ready_recreate_retry(
                true,
                0,
                "HTTP 500: Xccs : ErrorCallingWNS : Send command failed : State WaitingForServerToRegister",
                Some(SessionPhase::WaitingSessionReady),
                Some("Provisioning"),
                Some("ServerNeverRegistered"),
                Some("ServerNeverRegistered"),
            ),
            Some(SessionReadyRetryDecision::Exhausted(
                SessionReadyRecreateRetryReason::WaitingForServerRegistration,
            ))
        );
    }

    #[test]
    fn cleanup_terminal_state_only_accepts_closed_or_failed() {
        assert!(is_session_cleanup_terminal_state(Some("Closed")));
        assert!(is_session_cleanup_terminal_state(Some("Failed")));
        assert!(!is_session_cleanup_terminal_state(Some("Provisioning")));
        assert!(!is_session_cleanup_terminal_state(Some("ReadyToConnect")));
        assert!(!is_session_cleanup_terminal_state(None));
    }

    #[test]
    fn recreate_reused_session_only_fails_when_cleanup_did_not_settle() {
        assert!(should_fail_home_recreate_same_session(
            false,
            Some("session-1"),
            "session-1",
        ));
        assert!(!should_fail_home_recreate_same_session(
            true,
            Some("session-1"),
            "session-1",
        ));
        assert!(!should_fail_home_recreate_same_session(
            false,
            Some("session-1"),
            "session-2",
        ));
        assert!(!should_fail_home_recreate_same_session(
            false,
            None,
            "session-1",
        ));
    }

    #[test]
    fn non_home_provisioning_timeout_is_not_retryable() {
        assert_eq!(
            decide_home_session_ready_recreate_retry(
                false,
                0,
                "streamingStartTimeout:sessionId=session-1",
                Some(SessionPhase::WaitingSessionReady),
                Some("Provisioning"),
                None,
                None,
            ),
            None
        );
    }

    #[test]
    fn non_provisioning_state_is_not_retryable() {
        assert_eq!(
            decide_home_session_ready_recreate_retry(
                true,
                0,
                "streamingStartTimeout:sessionId=session-1",
                Some(SessionPhase::WaitingSessionReady),
                Some("ReadyToConnect"),
                None,
                None,
            ),
            None
        );
    }

    #[test]
    fn home_waiting_for_server_registration_is_exhausted_in_provisioning() {
        assert_eq!(
            decide_home_session_ready_recreate_retry(
                true,
                0,
                "HTTP 500: Xccs : ErrorCallingWNS : Send command failed : State WaitingForServerToRegister",
                Some(SessionPhase::Failed),
                Some("Provisioning"),
                Some("ServerNeverRegistered"),
                Some("ServerNeverRegistered"),
            ),
            Some(SessionReadyRetryDecision::Exhausted(
                SessionReadyRecreateRetryReason::WaitingForServerRegistration,
            ))
        );
    }

    #[test]
    fn home_server_registration_wait_timeout_only_triggers_after_threshold() {
        let runtime = SessionRuntimeSnapshot {
            stream_state: Some("Provisioning".to_string()),
            player_state: "pending".to_string(),
            queue: None,
            error_details: None,
        };
        let mut wait_started_at_ms = None;

        assert_eq!(
            evaluate_home_server_registration_wait_timeout(
                true,
                "console-1",
                "session-1",
                &runtime,
                10_000,
                &mut wait_started_at_ms,
            ),
            None
        );
        assert_eq!(wait_started_at_ms, Some(10_000));

        let error = evaluate_home_server_registration_wait_timeout(
            true,
            "console-1",
            "session-1",
            &runtime,
            40_000,
            &mut wait_started_at_ms,
        )
        .expect("home provisioning wait should stop after threshold");
        assert!(error.message.contains("homeServerRegistrationTimeout"));
        assert_eq!(
            error.startup_hint.as_ref().map(|hint| hint.kind.clone()),
            Some(SessionFlowStartupErrorKind::HostRegistrationRetryExhausted)
        );
        assert!(error
            .startup_hint
            .as_ref()
            .is_some_and(|hint| !hint.retryable));
    }

    #[test]
    fn home_server_registration_wait_timeout_resets_after_state_progresses() {
        let provisioning_runtime = SessionRuntimeSnapshot {
            stream_state: Some("Provisioning".to_string()),
            player_state: "pending".to_string(),
            queue: None,
            error_details: None,
        };
        let ready_runtime = SessionRuntimeSnapshot {
            stream_state: Some("ReadyToConnect".to_string()),
            player_state: "pending".to_string(),
            queue: None,
            error_details: None,
        };
        let mut wait_started_at_ms = None;

        let _ = evaluate_home_server_registration_wait_timeout(
            true,
            "console-1",
            "session-1",
            &provisioning_runtime,
            10_000,
            &mut wait_started_at_ms,
        );
        assert_eq!(wait_started_at_ms, Some(10_000));

        assert_eq!(
            evaluate_home_server_registration_wait_timeout(
                true,
                "console-1",
                "session-1",
                &ready_runtime,
                12_000,
                &mut wait_started_at_ms,
            ),
            None
        );
        assert_eq!(wait_started_at_ms, None);
    }

    #[test]
    fn home_server_registration_retry_exhausted_is_terminal_host_issue() {
        let error = build_home_session_ready_retry_exhausted_error(
            "console-1",
            SessionReadyRecreateRetryReason::WaitingForServerRegistration,
            1,
            1,
            SessionFlowError::message(
                "Agent : ServerNeverRegistered : Server never registered with service",
            ),
        );

        assert!(error.message.contains("homeSessionBoundedRetryExhausted"));
        assert!(error.message.contains("targetId=console-1"));
        assert!(error
            .message
            .contains("reason=waitingForServerRegistration"));
        assert_eq!(
            error.startup_hint.as_ref().map(|hint| hint.kind.clone()),
            Some(SessionFlowStartupErrorKind::HostRegistrationRetryExhausted)
        );
        assert_eq!(
            error.body.as_deref(),
            Some("Agent : ServerNeverRegistered : Server never registered with service")
        );
    }
