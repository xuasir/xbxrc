use super::*;
use crate::session::monitor::{SessionRuntimeBinding, SessionRuntimeSnapshot};
use crate::session::store::SessionRuntimeRecord;
use std::sync::atomic::{AtomicUsize, Ordering};

#[path = "signaling/flow.test.rs"]
mod signaling_regression;

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

#[derive(Clone, Default)]
struct ConsolePreparationProvider {
    wake_count: Arc<AtomicUsize>,
    console_poll_count: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl SessionFlowProvider for ConsolePreparationProvider {
    async fn get_streaming_token(&self, _target_type: &str) -> Result<Value, SessionFlowError> {
        Ok(serde_json::json!({ "gsToken": "token" }))
    }

    async fn transfer_token(&self) -> Result<String, SessionFlowError> {
        Ok("transfer-token".to_string())
    }

    async fn power_on_console(&self, _console_id: &str) -> Result<bool, SessionFlowError> {
        self.wake_count.fetch_add(1, Ordering::SeqCst);
        Ok(true)
    }

    async fn get_remote_consoles(&self) -> Result<Vec<RemoteConsoleSnapshot>, SessionFlowError> {
        self.console_poll_count.fetch_add(1, Ordering::SeqCst);
        let is_awake = self.wake_count.load(Ordering::SeqCst) > 0;
        Ok(vec![RemoteConsoleSnapshot {
            server_id: Some("console-1".to_string()),
            power_state: Some(if is_awake { "On" } else { "ConnectedStandby" }.to_string()),
            remote_management_enabled: Some(true),
            console_streaming_enabled: Some(true),
            ..Default::default()
        }])
    }
}

fn console_preparation_flow(
    provider: ConsolePreparationProvider,
) -> SessionFlowService<Snapshot, ConsolePreparationProvider> {
    SessionFlowService::new(provider)
}

fn home_preparation_plan(wake_console: bool, require_console_ready: bool) -> Plan {
    let mut plan = Plan::default();
    plan.session.target = crate::policy::types::Target::Home;
    plan.session.target_id = "console-1".to_string();
    plan.session.schedule.wake_console = wake_console;
    plan.session.schedule.require_console_ready = require_console_ready;
    plan.session.schedule.monitor_interval_ms = 1;
    plan.session.schedule.ready_timeout_ms = 1_000;
    plan
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

#[tokio::test]
async fn home_initial_preparation_wakes_standby_console_then_waits_for_ready() {
    let provider = ConsolePreparationProvider::default();
    let flow = console_preparation_flow(provider.clone());
    let plan = home_preparation_plan(true, true);

    flow.prepare_remote_console::<NoopSessionStartupObserver>(&plan, None)
        .await
        .expect("home console should become ready after wake");

    assert_eq!(provider.wake_count.load(Ordering::SeqCst), 1);
    assert!(provider.console_poll_count.load(Ordering::SeqCst) >= 2);
}

#[tokio::test]
async fn cloud_initial_preparation_never_wakes_or_polls_console() {
    let provider = ConsolePreparationProvider::default();
    let flow = console_preparation_flow(provider.clone());
    let mut plan = home_preparation_plan(true, true);
    plan.session.target = crate::policy::types::Target::Cloud;

    flow.prepare_remote_console::<NoopSessionStartupObserver>(&plan, None)
        .await
        .expect("cloud preparation should skip console work");

    assert_eq!(provider.wake_count.load(Ordering::SeqCst), 0);
    assert_eq!(provider.console_poll_count.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn home_initial_preparation_skips_console_when_policy_disables_both_gates() {
    let provider = ConsolePreparationProvider::default();
    let flow = console_preparation_flow(provider.clone());
    let plan = home_preparation_plan(false, false);

    flow.prepare_remote_console::<NoopSessionStartupObserver>(&plan, None)
        .await
        .expect("disabled home preparation should skip console work");

    assert_eq!(provider.wake_count.load(Ordering::SeqCst), 0);
    assert_eq!(provider.console_poll_count.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn home_initial_preparation_can_wake_without_ready_gate() {
    let provider = ConsolePreparationProvider::default();
    let flow = console_preparation_flow(provider.clone());
    let plan = home_preparation_plan(true, false);

    flow.prepare_remote_console::<NoopSessionStartupObserver>(&plan, None)
        .await
        .expect("wake-only home preparation should succeed");

    assert_eq!(provider.wake_count.load(Ordering::SeqCst), 1);
    assert_eq!(provider.console_poll_count.load(Ordering::SeqCst), 0);
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
        resolve_runtime_launch_state(phase),
        RuntimeLaunchState::Ready
    );
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
    assert_eq!(progress.runtime_launch_state, RuntimeLaunchState::Ready);
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
    let error = remote_console_wake_circuit_open_error("console-1", Some("ConnectedStandby"), 3);
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

fn startup_progress(phase: SessionPhase, error_message: Option<&str>) -> SessionProgressSnapshot {
    SessionProgressSnapshot {
        session_id: "session-1".to_string(),
        phase,
        runtime_launch_state: resolve_runtime_launch_state(phase),
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
            runtime_launch_state: RuntimeLaunchState::Failed,
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
fn runtime_starting_waits_for_provisioned_after_connect_token() {
    let mut last_recovery_signal_at_ms = 10_000;
    let action = decide_startup_progress_action(
        &startup_progress(SessionPhase::RuntimeStarting, None),
        11_000,
        &mut last_recovery_signal_at_ms,
        2_000,
    );
    assert_eq!(
        action,
        StartupProgressAction::Continue {
            transient_closed: false
        }
    );
    assert_eq!(last_recovery_signal_at_ms, 11_000);
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
fn failed_server_registration_error_is_retryable_once() {
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
        Some(SessionReadyRetryDecision::Retry(
            SessionReadyRecreateRetryReason::WaitingForServerRegistration,
        ))
    );
}

#[test]
fn waiting_for_server_registration_retry_signal_is_retryable_once_bounded_retry() {
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
        Some(SessionReadyRetryDecision::Retry(
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
            1,
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

#[derive(Clone, Default)]
struct HttpTestProvider {
    fail_access: Arc<std::sync::atomic::AtomicBool>,
    events: Arc<(Mutex<Vec<String>>, tokio::sync::Notify)>,
    credential_error: Arc<Mutex<Option<SessionFlowError>>>,
    transfer_error: Arc<Mutex<Option<SessionFlowError>>>,
    renewed_token: Arc<Mutex<Option<Result<Value, xbox_webapi::WebApiError>>>>,
    renewals: Arc<AtomicUsize>,
}

const HTTP_TEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
const SIGNAL_TEST_TIMEOUT_MS: u64 = 10_000;

async fn finish_test_future<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(HTTP_TEST_TIMEOUT, future)
        .await
        .expect("test operation should finish after the expected event")
}

async fn assert_pending<F: std::future::Future>(mut future: std::pin::Pin<&mut F>) {
    std::future::poll_fn(|context| {
        assert!(future.as_mut().poll(context).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
}

impl HttpTestProvider {
    async fn wait_for_event(&self, event: &str) {
        finish_test_future(async {
            loop {
                let changed = self.events.1.notified();
                if self
                    .events
                    .0
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|item| item == event)
                {
                    return;
                }
                changed.await;
            }
        })
        .await;
    }
}

fn http_test_flow(provider: HttpTestProvider) -> SessionFlowService<Snapshot, HttpTestProvider> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(HTTP_TEST_TIMEOUT)
        .build()
        .expect("direct test client");
    SessionFlowService::with_transport(
        provider,
        xbox_webapi::transport::HttpTransport::with_client(client),
    )
}
#[async_trait::async_trait]
impl SessionFlowProvider for HttpTestProvider {
    async fn get_streaming_token(&self, _: &str) -> Result<Value, SessionFlowError> {
        if let Some(error) = self.credential_error.lock().unwrap().clone() {
            return Err(error);
        }
        if self.fail_access.load(Ordering::SeqCst) {
            return Err(SessionFlowError::message("authenticationUnavailable"));
        }
        Ok(serde_json::json!({"gsToken":"test-token", "_authGeneration": 7}))
    }
    async fn recover_streaming_token(
        &self,
        target: &str,
        rejected: &str,
        credential_generation: Option<u64>,
    ) -> Result<Value, xbox_webapi::WebApiError> {
        assert_eq!(target, "home");
        assert_eq!(rejected, "test-token");
        assert_eq!(credential_generation, Some(7));
        self.renewals.fetch_add(1, Ordering::SeqCst);
        self.renewed_token
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_else(|| Err(xbox_webapi::WebApiError::http(401, "renewal unavailable")))
    }
    async fn transfer_token(&self) -> Result<String, SessionFlowError> {
        if let Some(error) = self.transfer_error.lock().unwrap().clone() {
            return Err(error);
        }
        Ok("transfer".to_owned())
    }
    fn on_session_lifecycle_event(&self, event: &str, _: Option<&str>, _: Value) {
        self.events.0.lock().unwrap().push(event.to_owned());
        self.events.1.notify_waiters();
    }
}

#[derive(Clone)]
struct ResponseGate {
    released: tokio::sync::watch::Sender<bool>,
}

impl Default for ResponseGate {
    fn default() -> Self {
        Self {
            released: tokio::sync::watch::channel(false).0,
        }
    }
}

impl ResponseGate {
    async fn wait(&self) {
        self.released
            .subscribe()
            .wait_for(|released| *released)
            .await
            .unwrap();
    }

    fn release(&self) {
        self.released.send_replace(true);
    }
}

struct TestServer {
    base_url: String,
    requests: Arc<Mutex<Vec<String>>>,
    request_observed: Arc<tokio::sync::Notify>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for TestServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl TestServer {
    async fn new(
        handler: impl Fn(&str) -> (u16, Value, Option<ResponseGate>) + Send + Sync + 'static,
    ) -> Self {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let observed = requests.clone();
        let request_observed = Arc::new(tokio::sync::Notify::new());
        let notification = request_observed.clone();
        let handler = Arc::new(handler);
        let task = tokio::spawn(async move {
            let mut workers = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let (mut socket, _) = accepted.unwrap();
                        let handler = handler.clone();
                        let observed = observed.clone();
                        let notification = notification.clone();
                        workers.spawn(async move {
                            let mut bytes = Vec::new();
                            loop {
                                let mut chunk = [0u8; 2048];
                                let count = socket.read(&mut chunk).await.unwrap_or(0);
                                if count == 0 { return; }
                                bytes.extend_from_slice(&chunk[..count]);
                                if let Some(end) = bytes.windows(4).position(|item| item == b"\r\n\r\n") {
                                    let headers = String::from_utf8_lossy(&bytes[..end]);
                                    let length = headers.lines().find_map(|line| line.to_ascii_lowercase().strip_prefix("content-length:").and_then(|n| n.trim().parse::<usize>().ok())).unwrap_or(0);
                                    if bytes.len() < end + 4 + length { continue; }
                                    let request = headers.lines().next().unwrap().to_owned();
                                    observed.lock().unwrap().push(request.clone());
                                    notification.notify_waiters();
                                    let (status, body, gate) = handler(&request);
                                    if let Some(gate) = gate { gate.wait().await; }
                                    let body = body.to_string();
                                    let response = format!("HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                                    let _ = socket.write_all(response.as_bytes()).await;
                                    return;
                                }
                            }
                        });
                    }
                    _ = workers.join_next(), if !workers.is_empty() => {}
                }
            }
        });
        Self {
            base_url,
            requests,
            request_observed,
            task,
        }
    }
    async fn wait_for(&self, prefix: &str) {
        self.wait_for_count(prefix, 1).await;
    }
    async fn wait_for_count(&self, prefix: &str, count: usize) {
        finish_test_future(async {
            loop {
                let changed = self.request_observed.notified();
                if self.count(prefix) >= count {
                    return;
                }
                changed.await;
            }
        })
        .await;
    }
    fn count(&self, prefix: &str) -> usize {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request.starts_with(prefix))
            .count()
    }
}

async fn install_http_record(
    flow: &SessionFlowService<Snapshot, HttpTestProvider>,
    base_url: &str,
) -> SessionCancelToken {
    let mut plan = Plan::default();
    plan.session.target = crate::policy::types::Target::Home;
    plan.session.base_url = base_url.to_owned();
    plan.session.target_id = "console".to_owned();
    plan.session.schedule.offer_exchange_timeout_ms = SIGNAL_TEST_TIMEOUT_MS;
    plan.session.schedule.ice_exchange_timeout_ms = SIGNAL_TEST_TIMEOUT_MS;
    let snapshot = Snapshot::new_pending(
        "session".to_owned(),
        "/v5/sessions/home/session".to_owned(),
        "console".to_owned(),
        "home".to_owned(),
    );
    let cancelled = flow.inner.sessions.write().await.insert_new_with_plan(
        "session".to_owned(),
        snapshot,
        plan,
        now_ms(),
    );
    flow.inner.mutations.write().await.insert(
        "session".to_owned(),
        Arc::new(SessionMutationEntry {
            cancelled: cancelled.clone(),
            state: tokio::sync::Mutex::new(SessionMutationState::default()),
            negotiation: tokio::sync::Mutex::new(()),
            ice: tokio::sync::Mutex::new(()),
        }),
    );
    cancelled
}

#[tokio::test]
async fn pending_answer_has_a_total_deadline_and_keeps_the_session_available_for_cleanup() {
    let gate = ResponseGate::default();
    let held = gate.clone();
    let reads = AtomicUsize::new(0);
    let server = TestServer::new(move |request| {
        let gate = (request.starts_with("GET") && reads.fetch_add(1, Ordering::SeqCst) >= 1)
            .then(|| held.clone());
        (200, serde_json::json!({}), gate)
    })
    .await;
    let flow = http_test_flow(HttpTestProvider::default());
    install_http_record(&flow, &server.base_url).await;
    let waiting = flow.clone();
    let offer = tokio::spawn(async move {
        waiting
            .exchange_offer("session", None, "offer", false)
            .await
    });
    server
        .wait_for_count("GET /v5/sessions/home/session/sdp", 2)
        .await;
    // HTTP 握手完成后再推进虚拟时钟，保证期限验证由测试决定。
    tokio::time::pause();
    tokio::time::advance(std::time::Duration::from_millis(SIGNAL_TEST_TIMEOUT_MS)).await;
    tokio::time::resume();
    let error = finish_test_future(offer).await.unwrap().unwrap_err();
    assert_eq!(error.message, "operationTimeout:stage=exchangeOffer");
    gate.release();
    assert_eq!(server.count("GET /v5/sessions/home/session/sdp"), 2);
    assert!(flow.get_session("session").await.is_some());
    assert!(flow.close_session("session").await.is_ok());
}

#[tokio::test]
async fn close_cancels_pending_sdp_and_does_not_wait_for_the_signal_lock() {
    let gate = ResponseGate::default();
    let held = gate.clone();
    let server = TestServer::new(move |request| {
        (
            200,
            serde_json::json!({}),
            request.starts_with("GET").then(|| held.clone()),
        )
    })
    .await;
    let flow = http_test_flow(HttpTestProvider::default());
    install_http_record(&flow, &server.base_url).await;
    let waiting = flow.clone();
    let offer = tokio::spawn(async move {
        waiting
            .exchange_offer("session", None, "offer", false)
            .await
    });
    server.wait_for("GET /v5/sessions/home/session/sdp").await;
    // GET 响应保持关闸，close 的完成证明撤销立即释放信令等待。
    assert!(finish_test_future(flow.close_session("session"))
        .await
        .is_ok());
    assert!(finish_test_future(offer)
        .await
        .unwrap()
        .unwrap_err()
        .message
        .starts_with("operationCancelled:"));
    gate.release();
    assert!(flow.inner.signaling.read().await.is_empty());
}

#[tokio::test]
async fn cleanup_revokes_local_resources_before_access_failure_and_can_retry_the_remote_stop() {
    let server = TestServer::new(|_| (200, serde_json::json!({}), None)).await;
    let provider = HttpTestProvider::default();
    let flow = http_test_flow(provider.clone());
    let cancelled = install_http_record(&flow, &server.base_url).await;
    flow.inner
        .signaling
        .write()
        .await
        .insert("session".to_owned(), SessionSignalingState::default());
    provider.fail_access.store(true, Ordering::SeqCst);
    assert!(flow.close_session("session").await.is_err());
    assert!(cancelled.is_cancelled());
    assert!(flow.inner.sessions.read().await.keys().is_empty());
    assert!(flow.inner.signaling.read().await.is_empty());
    assert!(flow.inner.mutations.read().await.is_empty());
    provider.fail_access.store(false, Ordering::SeqCst);
    assert!(flow.close_session("session").await.is_ok());
    assert_eq!(server.count("DELETE"), 1);
    assert!(flow.inner.cleanups.lock().await.is_empty());
}

#[tokio::test]
async fn concurrent_close_and_a_dropped_caller_share_one_independent_cleanup() {
    let gate = ResponseGate::default();
    let held = gate.clone();
    let server = TestServer::new(move |_| (200, serde_json::json!({}), Some(held.clone()))).await;
    let flow = http_test_flow(HttpTestProvider::default());
    install_http_record(&flow, &server.base_url).await;
    let closing = flow.clone();
    let first = tokio::spawn(async move { closing.close_session("session").await });
    server.wait_for("DELETE").await;
    first.abort();
    let second = flow.close_session_inner("session", true);
    tokio::pin!(second);
    assert_pending(second.as_mut()).await;
    gate.release();
    assert!(finish_test_future(second).await.is_ok());
    assert_eq!(server.count("DELETE"), 1);
    assert!(flow.inner.cleanups.lock().await.is_empty());
}

#[tokio::test]
async fn late_ice_from_a_replaced_session_cannot_write_signaling_state() {
    let gate = ResponseGate::default();
    let held = gate.clone();
    let server = TestServer::new(move |_| (200, serde_json::json!([{ "candidate": "candidate:1 1 UDP 1 127.0.0.1 5000 typ host", "sdpMid": "0", "sdpMLineIndex": 0 }]), Some(held.clone()))).await;
    let flow = http_test_flow(HttpTestProvider::default());
    let stale = install_http_record(&flow, &server.base_url).await;
    let polling = flow.clone();
    let response = tokio::spawn(async move { polling.poll_ice("session", false).await });
    server.wait_for("GET").await;
    let current = install_http_record(&flow, &server.base_url).await;
    gate.release();
    assert!(finish_test_future(response).await.unwrap().is_err());
    assert!(stale.is_cancelled());
    assert!(!current.is_cancelled());
    assert!(flow.inner.signaling.read().await.is_empty());
    assert!(flow.get_session("session").await.is_some());
}

#[tokio::test]
async fn ice_snapshot_dedup_preserves_new_end_markers_across_restart() {
    let requests = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = requests.clone();
    let server = TestServer::new(move |_| {
        let mut candidates = vec![serde_json::json!({"candidate":"candidate:1 1 UDP 1 127.0.0.1 5000 typ host", "sdpMid":"0", "sdpMLineIndex":0})];
        if counter.fetch_add(1, Ordering::SeqCst) >= 2 { candidates.push(serde_json::json!({"candidate":"end-of-candidates", "sdpMid":"0", "sdpMLineIndex":0})); }
        (200, serde_json::json!({"exchangeResponse": serde_json::to_string(&candidates).unwrap()}), None)
    }).await;
    let flow = http_test_flow(HttpTestProvider::default());
    install_http_record(&flow, &server.base_url).await;
    assert_eq!(flow.poll_ice("session", false).await.unwrap().len(), 1);
    assert!(flow.poll_ice("session", true).await.unwrap().is_empty());
    let next = flow.poll_ice("session", true).await.unwrap();
    assert_eq!(next.len(), 2);
    assert_eq!(next[1].candidate, "a=end-of-candidates");
    flow.close_session("session").await.unwrap();
}

#[tokio::test]
async fn missing_create_result_blocks_a_new_post_for_the_same_target() {
    let server = TestServer::new(|_| (200, serde_json::json!({}), None)).await;
    let flow = http_test_flow(HttpTestProvider::default());
    let mut plan = Plan::default();
    plan.session.target = crate::policy::types::Target::Home;
    plan.session.base_url = server.base_url.clone();
    plan.session.target_id = "console".to_owned();
    assert!(flow.create_session(plan.clone()).await.is_err());
    let error = flow.create_session(plan).await.err().unwrap();
    assert_eq!(error.message, "createOutcomeUnresolved");
    assert_eq!(server.count("POST"), 1);
    assert!(flow.inner.sessions.read().await.keys().is_empty());
}

#[tokio::test]
async fn cancelled_create_owns_its_late_response_and_closes_it_without_starting_loops() {
    let gate = ResponseGate::default();
    let held = gate.clone();
    let server = TestServer::new(move |request| {
        if request.starts_with("POST") {
            (
                200,
                serde_json::json!({"sessionPath":"/v5/sessions/home/session"}),
                Some(held.clone()),
            )
        } else {
            (200, serde_json::json!({}), None)
        }
    })
    .await;
    let flow = http_test_flow(HttpTestProvider::default());
    let scope = flow.begin_startup("attempt", "home", "console").unwrap();
    let observed_scope = scope.clone();
    let mut plan = Plan::default();
    plan.session.target = crate::policy::types::Target::Home;
    plan.session.base_url = server.base_url.clone();
    plan.session.target_id = "console".to_owned();
    let creating = flow.clone();
    let request = tokio::spawn(async move {
        creating
            .create_session_with_observer::<NoopSessionStartupObserver>(plan, None, None, &scope)
            .await
    });
    server.wait_for("POST").await;
    let cancelling = flow.clone();
    let cancellation = tokio::spawn(async move { cancelling.cancel_startup("attempt").await });
    finish_test_future(observed_scope.run::<()>(
        "cancellationObserved",
        None,
        std::future::pending(),
    ))
    .await
    .unwrap_err();
    gate.release();
    finish_test_future(cancellation).await.unwrap().unwrap();
    assert!(finish_test_future(request).await.unwrap().is_err());
    assert_eq!(server.count("DELETE"), 1);
    assert_eq!(server.count("GET"), 0);
    assert!(flow.inner.sessions.read().await.keys().is_empty());
    assert!(flow.inner.cleanups.lock().await.is_empty());
    assert!(flow.begin_startup("retry", "home", "console").is_ok());
}

#[tokio::test]
async fn playback_survives_180_seconds_of_remote_preparation_regression_then_closes_on_terminal_state(
) {
    let server = TestServer::new(|_| (200, serde_json::json!({}), None)).await;
    let flow = http_test_flow(HttpTestProvider::default());
    let cancelled = install_http_record(&flow, &server.base_url).await;
    let mut record = flow.get_session_record("session").await.unwrap();
    for second in 0..=180 {
        let state = match second {
            0 => "Provisioned",
            n if n % 2 == 0 => "Provisioning",
            _ => "ReadyToConnect",
        };
        let control = crate::session::monitor::apply_monitor_tick_to_record(
            &mut record,
            crate::session::monitor::SessionMonitorInput {
                now_ms: 1_000 + second * 1_000,
                state_timeout_ms: 90_000,
                stream_state: Some(state.to_owned()),
                error_details: None,
                waiting_queue: None,
            },
        );
        let progress = build_session_progress_snapshot(record.clone());
        assert_eq!(progress.phase, SessionPhase::SessionReady);
        assert_eq!(progress.runtime_launch_state, RuntimeLaunchState::Ready);
        assert!(progress.error_code.is_none());
        assert!(!control.should_send_connect_token);
    }
    assert!(!cancelled.is_cancelled());
    crate::session::monitor::apply_monitor_tick_to_record(
        &mut record,
        crate::session::monitor::SessionMonitorInput {
            now_ms: 182_000,
            state_timeout_ms: 90_000,
            stream_state: Some("Failed".to_owned()),
            error_details: None,
            waiting_queue: None,
        },
    );
    flow.inner
        .sessions
        .write()
        .await
        .upsert_if_same("session".to_owned(), record);
    assert_eq!(
        flow.get_session_progress("session").await.unwrap().phase,
        SessionPhase::Failed
    );
    assert!(!flow.session_allows_recovery("session").await);
    flow.close_session("session").await.unwrap();
    assert!(cancelled.is_cancelled());
}

#[tokio::test]
async fn keepalive_status_classes_preserve_transient_failures_and_publish_real_terminal_facts() {
    for status in [400, 401, 404, 429, 500] {
        let server =
            TestServer::new(move |_| (status, serde_json::json!({"code":"test"}), None)).await;
        let flow = http_test_flow(HttpTestProvider::default());
        let cancelled = install_http_record(&flow, &server.base_url).await;
        let mut record = flow.get_session_record("session").await.unwrap();
        record.snapshot.runtime.player_state = "started".to_owned();
        record.snapshot.runtime.stream_state = Some("Provisioned".to_owned());
        record.metadata.reached_provisioned = true;
        flow.inner
            .sessions
            .write()
            .await
            .upsert_if_same("session".to_owned(), record);
        let _ = flow.send_keepalive("session").await;
        let progress = flow.get_session_progress("session").await.unwrap();
        match status {
            401 => {
                assert_eq!(progress.phase, SessionPhase::Failed);
                assert_eq!(
                    progress.error_hint.unwrap().kind,
                    SessionFlowStartupErrorKind::Auth
                );
                assert!(cancelled.is_cancelled());
            }
            404 => {
                assert_eq!(progress.phase, SessionPhase::Closed);
                assert!(cancelled.is_cancelled());
            }
            _ => {
                assert_eq!(progress.phase, SessionPhase::SessionReady);
                assert!(!cancelled.is_cancelled());
            }
        }
    }
}

fn renewal_error(kind: SessionFlowStartupErrorKind) -> SessionFlowError {
    let retryable = kind != SessionFlowStartupErrorKind::Auth;
    SessionFlowError::message("renewal failure").with_startup_hint(SessionFlowStartupErrorHint {
        kind,
        retryable,
        diagnostic_summary: "safe-summary".into(),
    })
}

async fn mark_http_record_playing(flow: &SessionFlowService<Snapshot, HttpTestProvider>) {
    let mut record = flow.get_session_record("session").await.unwrap();
    record.snapshot.runtime.player_state = "started".into();
    record.snapshot.runtime.stream_state = Some("Provisioned".into());
    record.metadata.reached_provisioned = true;
    flow.inner
        .sessions
        .write()
        .await
        .upsert_if_same("session".into(), record);
}

#[tokio::test]
async fn keepalive_renews_rejected_credentials_once_and_retains_the_active_session() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let observed = attempts.clone();
    let server = TestServer::new(move |_| {
        let status = if observed.fetch_add(1, Ordering::SeqCst) == 0 {
            401
        } else {
            200
        };
        (status, serde_json::json!({}), None)
    })
    .await;
    let provider = HttpTestProvider::default();
    *provider.renewed_token.lock().unwrap() = Some(Ok(serde_json::json!({"gsToken":"renewed"})));
    let flow = http_test_flow(provider.clone());
    let cancelled = install_http_record(&flow, &server.base_url).await;
    mark_http_record_playing(&flow).await;
    assert!(flow.send_keepalive("session").await.unwrap());
    assert_eq!(
        flow.get_session_progress("session").await.unwrap().phase,
        SessionPhase::SessionReady
    );
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
    assert_eq!(provider.renewals.load(Ordering::SeqCst), 1);
    assert!(!cancelled.is_cancelled());
}

#[tokio::test]
async fn keepalive_credential_failure_publishes_auth_or_preserves_a_temporary_failure() {
    for kind in [
        SessionFlowStartupErrorKind::Auth,
        SessionFlowStartupErrorKind::Network,
    ] {
        let server = TestServer::new(|_| (200, serde_json::json!({}), None)).await;
        let provider = HttpTestProvider::default();
        *provider.credential_error.lock().unwrap() = Some(renewal_error(kind.clone()));
        let flow = http_test_flow(provider);
        let cancelled = install_http_record(&flow, &server.base_url).await;
        mark_http_record_playing(&flow).await;
        let result = flow.send_keepalive("session").await;
        let progress = flow.get_session_progress("session").await.unwrap();
        if kind == SessionFlowStartupErrorKind::Auth {
            assert!(!result.unwrap());
            assert_eq!(progress.phase, SessionPhase::Failed);
            assert_eq!(progress.error_code.as_deref(), Some("AuthRenewalRequired"));
            assert_eq!(
                progress.error_hint.unwrap().kind,
                SessionFlowStartupErrorKind::Auth
            );
            assert!(cancelled.is_cancelled());
        } else {
            assert_eq!(
                result.unwrap_err().startup_hint.unwrap().kind,
                SessionFlowStartupErrorKind::Network
            );
            assert_eq!(progress.phase, SessionPhase::SessionReady);
            assert!(!cancelled.is_cancelled());
        }
    }
}

#[tokio::test]
async fn connect_token_auth_failure_terminates_and_network_failure_retries_on_the_next_tick() {
    for kind in [
        SessionFlowStartupErrorKind::Auth,
        SessionFlowStartupErrorKind::Network,
    ] {
        let server = TestServer::new(|request| {
            if request.starts_with("GET") {
                (200, serde_json::json!({"state":"ReadyToConnect"}), None)
            } else {
                (200, serde_json::json!({}), None)
            }
        })
        .await;
        let provider = HttpTestProvider::default();
        *provider.transfer_error.lock().unwrap() = Some(renewal_error(kind.clone()));
        let flow = http_test_flow(provider.clone());
        let cancelled = install_http_record(&flow, &server.base_url).await;
        let scheduler = SessionScheduler::new(flow.inner.clone());
        let continues = scheduler.monitor_session_tick("session").await;
        let progress = flow.get_session_progress("session").await.unwrap();
        if kind == SessionFlowStartupErrorKind::Auth {
            assert!(!continues);
            assert_eq!(
                progress.error_hint.unwrap().kind,
                SessionFlowStartupErrorKind::Auth
            );
            assert!(cancelled.is_cancelled());
        } else {
            assert!(continues);
            assert!(!cancelled.is_cancelled());
            assert!(progress.error_hint.is_none());
            *provider.transfer_error.lock().unwrap() = None;
            assert!(scheduler.monitor_session_tick("session").await);
            assert!(
                flow.session_mutation("session", &cancelled)
                    .await
                    .unwrap()
                    .state
                    .lock()
                    .await
                    .connect_token_sent
            );
        }
    }
}

#[tokio::test]
async fn fifty_create_close_cycles_leave_remote_and_local_resources_empty() {
    let active_remote = Arc::new(AtomicUsize::new(0));
    let active = active_remote.clone();
    let server = TestServer::new(move |request| {
        if request.starts_with("POST") && request.contains("/play ") {
            assert_eq!(active.fetch_add(1, Ordering::SeqCst), 0);
            (
                200,
                serde_json::json!({"sessionPath":"/v5/sessions/home/session"}),
                None,
            )
        } else if request.starts_with("DELETE") {
            assert_eq!(active.fetch_sub(1, Ordering::SeqCst), 1);
            (200, serde_json::json!({}), None)
        } else {
            (200, serde_json::json!({"state":"Provisioned"}), None)
        }
    })
    .await;
    let flow = http_test_flow(HttpTestProvider::default());
    let mut plan = Plan::default();
    plan.session.target = crate::policy::types::Target::Home;
    plan.session.target_id = "console".to_owned();
    plan.session.base_url = server.base_url.clone();
    for _ in 0..50 {
        let session = flow.create_session(plan.clone()).await.unwrap();
        let (first, second) = tokio::join!(
            flow.close_session(session.session_id()),
            flow.close_session(session.session_id())
        );
        assert!(first.is_ok());
        assert!(second.is_ok());
        assert_eq!(active_remote.load(Ordering::SeqCst), 0);
        assert!(flow.inner.sessions.read().await.keys().is_empty());
        assert!(flow.inner.signaling.read().await.is_empty());
        assert!(flow.inner.mutations.read().await.is_empty());
        assert!(flow.inner.cleanups.lock().await.is_empty());
    }
    assert_eq!(server.count("DELETE"), 50);
}

#[tokio::test]
async fn automatic_recreate_cleanup_preserves_startup_but_user_close_revokes_it() {
    let server = TestServer::new(|_| (200, serde_json::json!({}), None)).await;
    let flow = http_test_flow(HttpTestProvider::default());
    let scope = flow.begin_startup("attempt", "home", "console").unwrap();
    let cancelled = install_http_record(&flow, &server.base_url).await;
    scope.set_owned("session".to_owned(), cancelled);
    flow.cleanup_session_for_recreate("session").await;
    assert!(!scope.is_cancelled());
    assert!(scope.owned().is_none());
    assert!(flow
        .begin_startup("overlapping", "home", "console")
        .is_err());
    assert!(scope
        .run("nextCreate", None, async { Ok(()) })
        .await
        .is_ok());
    let cancelled = install_http_record(&flow, &server.base_url).await;
    scope.set_owned("session".to_owned(), cancelled);
    flow.close_session("session").await.unwrap();
    assert!(scope.is_cancelled());
    assert!(scope.owned().is_none());
}

#[tokio::test]
async fn failed_cleanup_is_reclaimed_after_access_recovers_without_user_retry() {
    let server = TestServer::new(|_| (200, serde_json::json!({}), None)).await;
    let provider = HttpTestProvider::default();
    let flow = http_test_flow(provider.clone());
    install_http_record(&flow, &server.base_url).await;
    provider.fail_access.store(true, Ordering::SeqCst);
    assert!(flow.close_session("session").await.is_err());
    tokio::time::pause();
    // 先让退避任务注册计时，再显式触发一秒后的首次重试。
    tokio::task::yield_now().await;
    provider.fail_access.store(false, Ordering::SeqCst);
    tokio::time::advance(std::time::Duration::from_secs(1)).await;
    tokio::time::resume();
    provider.wait_for_event("cleanupFinished").await;
    assert!(flow.inner.cleanups.lock().await.is_empty());
    assert_eq!(server.count("DELETE"), 1);
    assert!(flow.inner.sessions.read().await.keys().is_empty());
}

#[tokio::test]
async fn shutdown_waits_for_a_late_create_and_closes_its_remote_resource() {
    let gate = ResponseGate::default();
    let held = gate.clone();
    let server = TestServer::new(move |request| {
        if request.starts_with("POST") {
            (
                200,
                serde_json::json!({"sessionPath":"/v5/sessions/home/session"}),
                Some(held.clone()),
            )
        } else {
            (200, serde_json::json!({}), None)
        }
    })
    .await;
    let flow = http_test_flow(HttpTestProvider::default());
    let scope = SessionStartupScope::default();
    let observed_scope = scope.clone();
    let mut plan = Plan::default();
    plan.session.target = crate::policy::types::Target::Home;
    plan.session.base_url = server.base_url.clone();
    plan.session.target_id = "console".to_owned();
    let creating = flow.clone();
    let request = tokio::spawn(async move {
        creating
            .create_session_with_observer::<NoopSessionStartupObserver>(plan, None, None, &scope)
            .await
    });
    server.wait_for("POST").await;
    let closing = flow.clone();
    let shutdown = tokio::spawn(async move { closing.shutdown().await });
    finish_test_future(observed_scope.run::<()>("shutdownObserved", None, std::future::pending()))
        .await
        .unwrap_err();
    assert!(!shutdown.is_finished());
    gate.release();
    finish_test_future(shutdown).await.unwrap();
    assert!(finish_test_future(request).await.unwrap().is_err());
    assert_eq!(server.count("DELETE"), 1);
    assert_eq!(server.count("GET"), 0);
    assert!(flow.inner.sessions.read().await.keys().is_empty());
    assert!(flow.inner.signaling.read().await.is_empty());
    assert!(flow.inner.cleanups.lock().await.is_empty());
    assert_eq!(flow.inner.startups.lock().unwrap().active_count(), 0);
    assert!(flow.begin_startup("new", "home", "console").is_err());
}

#[tokio::test]
async fn cleanup_retry_task_releases_the_service_while_waiting_for_connectivity() {
    let server = TestServer::new(|_| (200, serde_json::json!({}), None)).await;
    let provider = HttpTestProvider::default();
    let flow = http_test_flow(provider.clone());
    install_http_record(&flow, &server.base_url).await;
    let weak = Arc::downgrade(&flow.inner);
    provider.fail_access.store(true, Ordering::SeqCst);
    assert!(flow.close_session("session").await.is_err());
    drop(flow);
    assert!(weak.upgrade().is_none());
}

#[tokio::test]
async fn concurrent_failed_closes_share_one_result_and_a_later_retry_uses_fresh_access() {
    let gate = ResponseGate::default();
    let held = gate.clone();
    let server = TestServer::new(move |_| {
        (
            500,
            serde_json::json!({"message":"temporary"}),
            Some(held.clone()),
        )
    })
    .await;
    let flow = http_test_flow(HttpTestProvider::default());
    install_http_record(&flow, &server.base_url).await;
    let closing = flow.clone();
    let first = tokio::spawn(async move { closing.close_session("session").await });
    server.wait_for("DELETE").await;
    let second = flow.close_session_inner("session", true);
    tokio::pin!(second);
    assert_pending(second.as_mut()).await;
    gate.release();
    assert!(finish_test_future(second).await.is_err());
    assert!(finish_test_future(first).await.unwrap().is_err());
    assert_eq!(server.count("DELETE"), 1);
    assert!(flow.close_session("session").await.is_err());
    assert_eq!(server.count("DELETE"), 2);
}
