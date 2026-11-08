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
