use super::*;
use std::future::Future;

const COOLDOWN: Duration = Duration::from_secs(15);

#[test]
fn first_check_proceeds_and_monotonic_cooldown_expires_at_fifteen_seconds() {
    let state = AuthRuntimeState::new("xal".into(), true, 1);
    let started = Instant::now();
    assert!(matches!(
        state.begin_check(started, COOLDOWN, true).unwrap(),
        BeginCheckOutcome::Proceed { .. }
    ));
    state.mark_authenticating_idle().unwrap();
    assert!(matches!(
        state
            .begin_check(started + Duration::from_millis(14_999), COOLDOWN, true)
            .unwrap(),
        BeginCheckOutcome::ShortCircuit(_)
    ));
    assert!(matches!(
        state
            .begin_check(started + COOLDOWN, COOLDOWN, true)
            .unwrap(),
        BeginCheckOutcome::Proceed { .. }
    ));
}

#[test]
fn in_flight_check_preserves_its_lease_after_the_cooldown_deadline() {
    let state = AuthRuntimeState::new("xal".into(), true, 1);
    let started = Instant::now();
    state.begin_check(started, COOLDOWN, true).unwrap();
    assert!(matches!(
        state
            .begin_check(started + Duration::from_secs(60), COOLDOWN, false)
            .unwrap(),
        BeginCheckOutcome::ShortCircuit(_)
    ));
    assert!(state.current_state().is_authenticating);
}

#[test]
fn clearing_credentials_resets_the_monotonic_cooldown() {
    let state = AuthRuntimeState::new("xal".into(), true, 1);
    let started = Instant::now();
    state.begin_check(started, COOLDOWN, true).unwrap();
    state.clear_credentials(|| Ok(())).unwrap();
    state.mark_authenticated(1).unwrap();
    assert!(matches!(
        state
            .begin_check(started + Duration::from_millis(1), COOLDOWN, true)
            .unwrap(),
        BeginCheckOutcome::Proceed { .. }
    ));
}

#[test]
fn expired_credentials_bypass_cooldown_and_cancelled_checks_release_authenticating_state() {
    let state = AuthRuntimeState::new("xal".into(), true, 1);
    let started = Instant::now();
    assert!(matches!(
        state.begin_check(started, COOLDOWN, true).unwrap(),
        BeginCheckOutcome::Proceed { .. }
    ));
    let lease = state.check_lease().unwrap();
    state.finish_check_success(false).unwrap();
    drop(lease);
    assert!(matches!(
        state
            .begin_check(started + Duration::from_millis(1), COOLDOWN, true)
            .unwrap(),
        BeginCheckOutcome::ShortCircuit(_)
    ));
    assert!(matches!(
        state
            .begin_check(started + Duration::from_millis(2), COOLDOWN, false)
            .unwrap(),
        BeginCheckOutcome::Proceed { .. }
    ));
    let lease = state.check_lease().unwrap();
    assert!(state.current_state().is_authenticating);
    drop(lease);
    assert!(!state.current_state().is_authenticating);
}

#[test]
fn logout_prevents_late_refresh_from_persisting_and_leaves_new_login_lease_intact() {
    let state = AuthRuntimeState::new("xal".into(), true, 1);
    state
        .begin_check(Instant::now(), Duration::ZERO, false)
        .unwrap();
    let old = state.check_lease().unwrap();
    let persisted = std::cell::Cell::new(false);
    state.clear_credentials(|| Ok(())).unwrap();
    state.begin_login().unwrap();
    assert!(state
        .commit_authentication(old.generation, 2, || {
            persisted.set(true);
            Ok(())
        })
        .is_err());
    assert!(!persisted.get());
    drop(old);
    assert!(state.current_state().is_authenticating);
}

#[test]
fn rejected_credentials_remain_excluded_until_successful_commit() {
    let state = AuthRuntimeState::new("xal".into(), true, 1);
    state.reject_web_token("rejected-secret").unwrap();
    assert!(state.is_web_token_rejected("rejected-secret"));
    assert!(!state.is_web_token_rejected("other-secret"));
    state
        .commit_authentication(state.generation().unwrap(), 1, || Ok(()))
        .unwrap();
    assert!(!state.is_web_token_rejected("rejected-secret"));
    let snapshot = state.sync_state_from_snapshot(None, false);
    assert!(!snapshot.is_authenticated);
    assert_eq!(snapshot.app_level, 0);
}

#[tokio::test]
async fn concurrent_checks_wait_for_the_active_refresh_result() {
    let state = std::sync::Arc::new(AuthRuntimeState::new("xal".into(), false, 0));
    let first_gate = state.lock_check().await;
    let second = state.lock_check();
    tokio::pin!(second);
    std::future::poll_fn(|context| {
        assert!(second.as_mut().poll(context).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    state
        .commit_authentication(state.generation().unwrap(), 1, || Ok(()))
        .unwrap();
    drop(first_gate);
    let _second_gate = second.await;
    assert!(state.current_state().is_authenticated);
}

#[test]
fn oauth_checkpoint_survives_downstream_failure_and_preserves_rejected_targets() {
    let state = AuthRuntimeState::new("xal".into(), true, 2);
    let generation = state.generation().unwrap();
    let stored_refresh_token = std::cell::RefCell::new("old-refresh".to_owned());
    state.reject_web_token("old-web").unwrap();
    state
        .reject_stream_token(AuthTokenScope::Home, "old-home")
        .unwrap();
    state
        .reject_stream_token(AuthTokenScope::Cloud, "old-cloud")
        .unwrap();
    state.reject_refresh_token(generation).unwrap();
    state
        .checkpoint_credentials(generation, || {
            *stored_refresh_token.borrow_mut() = "rotated-refresh".into();
            Ok(())
        })
        .unwrap();
    let previous = state.current_state();
    state
        .finish_check_transient_failure(&previous, false, 0)
        .unwrap();
    assert_eq!(&*stored_refresh_token.borrow(), "rotated-refresh");
    assert!(state.is_web_token_rejected("old-web"));
    assert!(state.is_stream_token_rejected(AuthTokenScope::Home, "old-home"));
    assert!(state.is_stream_token_rejected(AuthTokenScope::Cloud, "old-cloud"));
    assert!(!state.is_refresh_token_rejected());
    state
        .commit_scoped_authentication(generation, 1, AuthTokenScope::Home, || Ok(()))
        .unwrap();
    assert!(!state.is_web_token_rejected("old-web"));
    assert!(!state.is_stream_token_rejected(AuthTokenScope::Home, "old-home"));
    assert!(state.is_stream_token_rejected(AuthTokenScope::Cloud, "old-cloud"));
}

#[test]
fn oauth_checkpoint_is_generation_guarded_after_logout_or_interactive_cancellation() {
    for logout in [true, false] {
        let state = AuthRuntimeState::new("xal".into(), true, 1);
        if !logout {
            state.begin_login().unwrap();
        }
        let old_generation = state.generation().unwrap();
        if logout {
            state.clear_credentials(|| Ok(())).unwrap();
        } else {
            state.cancel_pending_login();
        }
        let persisted = std::cell::Cell::new(false);
        assert!(state
            .checkpoint_credentials(old_generation, || {
                persisted.set(true);
                Ok(())
            })
            .is_err());
        assert!(!persisted.get());
    }
}

#[test]
fn oauth_window_close_keeps_an_unrelated_silent_refresh_running() {
    let state = AuthRuntimeState::new("xal".into(), true, 1);
    state.begin_check(Instant::now(), COOLDOWN, false).unwrap();
    let lease = state.check_lease().unwrap();
    state
        .reject_stream_token(AuthTokenScope::Home, "rejected")
        .unwrap();
    state.cancel_pending_login();
    assert!(state.current_state().is_authenticating);
    assert_eq!(state.generation().unwrap(), lease.generation);
    assert!(state.is_stream_token_rejected(AuthTokenScope::Home, "rejected"));
    state
        .checkpoint_credentials(lease.generation, || Ok(()))
        .unwrap();
}

#[tokio::test]
async fn concurrent_failed_refreshes_share_a_bounded_failure_and_release_the_gate() {
    let state = AuthRuntimeState::new("xal".into(), false, 0);
    let first_gate = state.lock_check().await;
    let second = state.lock_check();
    tokio::pin!(second);
    std::future::poll_fn(|context| {
        assert!(second.as_mut().poll(context).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    let now = Instant::now();
    let error = xbox_webapi::WebApiError::network_kind(xbox_webapi::NetworkErrorKind::Timeout);
    state
        .remember_refresh_failure(
            state.generation().unwrap(),
            AuthTokenScope::Home,
            now,
            error.clone(),
        )
        .unwrap();
    drop(first_gate);
    let _second_gate = second.await;
    assert_eq!(
        state
            .cached_refresh_failure(AuthTokenScope::Home, now)
            .unwrap()
            .safe_summary(),
        error.safe_summary()
    );
    assert!(state
        .cached_refresh_failure(AuthTokenScope::Cloud, now)
        .is_none());
    assert!(state
        .cached_refresh_failure(AuthTokenScope::Home, now + Duration::from_secs(1))
        .is_none());
    state
        .commit_scoped_authentication(state.generation().unwrap(), 1, AuthTokenScope::Home, || {
            Ok(())
        })
        .unwrap();
    assert!(state
        .cached_refresh_failure(AuthTokenScope::Home, now)
        .is_none());
}
