use super::*;

fn cache(ttl: Duration, capacity: usize) -> Arc<HostCache> {
    Arc::new(HostCache::new(ttl, capacity))
}

fn host(id: &str, power: &str) -> DataHostSummary {
    DataHostSummary {
        id: Some(id.into()),
        server_id: Some(format!("stream-{id}")),
        device_id: Some(format!("device-{id}")),
        power_state: Some(power.into()),
        ..Default::default()
    }
}

fn leader(cache: &Arc<HostCache>, scope: &str, force: bool) -> HostCacheLease {
    let HostCacheJoin::Leader(lease) = cache.begin(scope, force) else {
        panic!("expected leader");
    };
    lease
}

fn snapshot(cache: &Arc<HostCache>, scope: &str) -> DataHostsSnapshot {
    let HostCacheJoin::Cached(snapshot) = cache.begin(scope, false) else {
        panic!("expected cached snapshot");
    };
    snapshot
}

#[tokio::test]
async fn cold_reads_and_forced_refreshes_join_one_account_flight() {
    let cache = cache(Duration::from_secs(60), 8);
    let lease = leader(&cache, "account-a", false);
    let HostCacheJoin::Follower(first) = cache.begin("account-a", false) else {
        panic!("follower")
    };
    let HostCacheJoin::Follower(second) = cache.begin("account-a", true) else {
        panic!("follower")
    };
    let result = lease.finish(Ok(vec![host("A", "On")])).unwrap();
    assert_eq!(first.await.unwrap().unwrap().updated_at, result.updated_at);
    assert_eq!(
        second.await.unwrap().unwrap().hosts[0].server_id.as_deref(),
        Some("stream-A")
    );
    let cached = snapshot(&cache, "account-a");
    assert_eq!(cached.hosts[0].id.as_deref(), Some("A"));
    assert_eq!(cached.hosts[0].device_id.as_deref(), Some("device-A"));
    assert!(cached.fresh_until > cached.updated_at);
}

#[test]
fn expired_snapshots_remain_available_with_explicit_stale_freshness() {
    let cache = cache(Duration::ZERO, 8);
    let result = leader(&cache, "account-a", false)
        .finish(Ok(vec![host("A", "On")]))
        .unwrap();
    let cached = snapshot(&cache, "account-a");
    assert_eq!(cached.updated_at, result.updated_at);
    assert_eq!(cached.fresh_until, 0);
    assert_eq!(cached.hosts[0].id.as_deref(), Some("A"));
    leader(&cache, "account-a", true)
        .finish(Ok(vec![host("A", "Off")]))
        .unwrap();
    assert_eq!(
        snapshot(&cache, "account-a").hosts[0]
            .power_state
            .as_deref(),
        Some("Off")
    );
}

#[tokio::test]
async fn force_bypasses_freshness_while_cached_reads_stay_immediate_and_empty_is_authoritative() {
    let cache = cache(Duration::from_secs(60), 8);
    leader(&cache, "account-a", false)
        .finish(Ok(vec![host("A", "On")]))
        .unwrap();
    let lease = leader(&cache, "account-a", true);
    let HostCacheJoin::Follower(waiter) = cache.begin("account-a", true) else {
        panic!("follower")
    };
    assert_eq!(snapshot(&cache, "account-a").hosts.len(), 1);
    let result = lease.finish(Ok(vec![])).unwrap();
    assert!(waiter.await.unwrap().unwrap().hosts.is_empty());
    let cached = snapshot(&cache, "account-a");
    assert!(cached.hosts.is_empty());
    assert_eq!(cached.updated_at, result.updated_at);
    assert!(cached.fresh_until > cached.updated_at);
}

#[tokio::test]
async fn refresh_failure_retains_success_and_preserves_the_request_error_for_waiters() {
    let cache = cache(Duration::from_secs(60), 8);
    let previous = leader(&cache, "account-a", false)
        .finish(Ok(vec![host("A", "On")]))
        .unwrap();
    let lease = leader(&cache, "account-a", true);
    let HostCacheJoin::Follower(waiter) = cache.begin("account-a", true) else {
        panic!("follower")
    };
    let failure = WebApiError::http_with_code(403, "Forbidden", "safe-error");
    assert_eq!(
        lease
            .finish(Err(failure.clone()))
            .unwrap_err()
            .safe_summary(),
        failure.safe_summary()
    );
    assert_eq!(
        waiter.await.unwrap().unwrap_err().safe_summary(),
        failure.safe_summary()
    );
    let cached = snapshot(&cache, "account-a");
    assert_eq!(cached.updated_at, previous.updated_at);
    assert_eq!(cached.hosts[0].id.as_deref(), Some("A"));
}

#[tokio::test]
async fn cancelling_a_leader_releases_waiters_and_keeps_a_retry_slot() {
    let cache = cache(Duration::from_secs(60), 8);
    let lease = leader(&cache, "account-a", false);
    let HostCacheJoin::Follower(waiter) = cache.begin("account-a", true) else {
        panic!("follower")
    };
    drop(lease);
    assert!(waiter.await.unwrap().is_err());
    leader(&cache, "account-a", true)
        .finish(Ok(vec![host("A", "On")]))
        .unwrap();
    assert_eq!(snapshot(&cache, "account-a").hosts.len(), 1);
}

#[tokio::test]
async fn power_invalidation_blocks_an_older_flight_from_restoring_freshness() {
    let cache = cache(Duration::from_secs(60), 8);
    let previous = leader(&cache, "account-a", false)
        .finish(Ok(vec![host("A", "On")]))
        .unwrap();
    let lease = leader(&cache, "account-a", true);
    let HostCacheJoin::Follower(waiter) = cache.begin("account-a", true) else {
        panic!("follower")
    };
    cache.invalidate("account-a");
    assert_eq!(snapshot(&cache, "account-a").fresh_until, 0);
    assert!(lease.finish(Ok(vec![host("OLD", "On")])).is_err());
    assert!(waiter.await.unwrap().is_err());
    assert_eq!(
        snapshot(&cache, "account-a").updated_at,
        previous.updated_at
    );
    assert_eq!(
        snapshot(&cache, "account-a").hosts[0].id.as_deref(),
        Some("A")
    );
    leader(&cache, "account-a", true)
        .finish(Ok(vec![host("A", "Off")]))
        .unwrap();
    assert_eq!(
        snapshot(&cache, "account-a").hosts[0]
            .power_state
            .as_deref(),
        Some("Off")
    );
}

#[test]
fn account_scopes_and_idle_lru_preserve_inflight_ownership() {
    let cache = cache(Duration::from_secs(60), 2);
    let pending_a = leader(&cache, "account-a", false);
    leader(&cache, "account-b", false)
        .finish(Ok(vec![host("B", "On")]))
        .unwrap();
    leader(&cache, "account-c", false)
        .finish(Ok(vec![host("C", "On")]))
        .unwrap();
    pending_a.finish(Ok(vec![host("A", "On")])).unwrap();
    assert_eq!(snapshot(&cache, "account-a").scope_key, "account-a");
    assert_eq!(
        snapshot(&cache, "account-c").hosts[0].id.as_deref(),
        Some("C")
    );
    assert_eq!(cache.state.lock().unwrap().entries.len(), 2);
    let new_b = leader(&cache, "account-b", false);
    cache.invalidate("account-c");
    new_b.finish(Ok(vec![host("B", "Off")])).unwrap();
    assert_eq!(
        snapshot(&cache, "account-b").hosts[0].id.as_deref(),
        Some("B")
    );
}

#[test]
fn active_scopes_survive_capacity_pressure_until_each_flight_completes() {
    let cache = cache(Duration::from_secs(60), 1);
    let a = leader(&cache, "a", false);
    let b = leader(&cache, "b", false);
    let c = leader(&cache, "c", false);
    assert_eq!(cache.state.lock().unwrap().entries.len(), 3);
    a.finish(Ok(vec![])).unwrap();
    assert_eq!(cache.state.lock().unwrap().entries.len(), 2);
    b.finish(Ok(vec![])).unwrap();
    c.finish(Ok(vec![host("C", "On")])).unwrap();
    assert_eq!(cache.state.lock().unwrap().entries.len(), 1);
    assert_eq!(snapshot(&cache, "c").hosts[0].id.as_deref(), Some("C"));
}
