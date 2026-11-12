use super::cache_repository::DataCacheRepository;
use super::types::{DataHostSummary, DataHostsSnapshot};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::oneshot;
use xbox_webapi::WebApiError;

type HostResult = Result<DataHostsSnapshot, WebApiError>;

struct HostEntry {
    snapshot: Option<DataHostsSnapshot>,
    fresh_until: Instant,
    generation: u64,
    access: u64,
    waiters: Option<Vec<oneshot::Sender<HostResult>>>,
}

#[derive(Default)]
struct HostCacheState {
    clock: u64,
    entries: HashMap<String, HostEntry>,
}

pub(crate) struct HostCache {
    state: Mutex<HostCacheState>,
    ttl: Duration,
    capacity: usize,
}

pub(crate) enum HostCacheJoin {
    Cached(DataHostsSnapshot),
    Leader(HostCacheLease),
    Follower(oneshot::Receiver<HostResult>),
}

pub(crate) struct HostCacheLease {
    cache: Arc<HostCache>,
    scope: String,
    generation: u64,
    finished: bool,
}

impl HostCache {
    pub(crate) fn new(ttl: Duration, capacity: usize) -> Self {
        Self {
            state: Mutex::new(HostCacheState::default()),
            ttl,
            capacity: capacity.max(1),
        }
    }

    pub(crate) fn begin(self: &Arc<Self>, scope: &str, force: bool) -> HostCacheJoin {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        state.clock += 1;
        let access = state.clock;
        let entry = state
            .entries
            .entry(scope.into())
            .or_insert_with(|| HostEntry {
                snapshot: None,
                fresh_until: Instant::now(),
                generation: 0,
                access,
                waiters: None,
            });
        entry.access = access;
        if !force {
            if let Some(snapshot) = &entry.snapshot {
                let mut snapshot = snapshot.clone();
                // 时效由单调时钟判定；过期内容仍可立即交付给页面。
                let remaining = entry.fresh_until.saturating_duration_since(Instant::now());
                snapshot.fresh_until = if remaining.is_zero() {
                    0
                } else {
                    DataCacheRepository::now_ms().saturating_add(remaining.as_millis() as u64)
                };
                return HostCacheJoin::Cached(snapshot);
            }
        }
        if let Some(waiters) = &mut entry.waiters {
            let (tx, rx) = oneshot::channel();
            waiters.push(tx);
            return HostCacheJoin::Follower(rx);
        }
        entry.waiters = Some(Vec::new());
        let generation = entry.generation;
        self.prune(&mut state);
        HostCacheJoin::Leader(HostCacheLease {
            cache: self.clone(),
            scope: scope.into(),
            generation,
            finished: false,
        })
    }

    pub(crate) fn invalidate(&self, scope: &str) {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(entry) = state.entries.get_mut(scope) {
            entry.generation += 1;
            entry.fresh_until = Instant::now();
            if let Some(snapshot) = &mut entry.snapshot {
                snapshot.fresh_until = 0;
            }
        }
    }

    fn complete(
        &self,
        scope: &str,
        generation: u64,
        result: Result<Vec<DataHostSummary>, WebApiError>,
    ) -> HostResult {
        let (result, waiters) = {
            let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
            let Some(entry) = state.entries.get_mut(scope) else {
                return Err(WebApiError::parse("主机查询已中断，请重试"));
            };
            let result = result.and_then(|hosts| {
                if entry.generation != generation {
                    return Err(WebApiError::parse("主机状态已更新，请重新查询"));
                }
                let updated_at = DataCacheRepository::now_ms();
                let snapshot = DataHostsSnapshot {
                    scope_key: scope.into(),
                    hosts,
                    updated_at,
                    fresh_until: updated_at.saturating_add(self.ttl.as_millis() as u64),
                };
                entry.snapshot = Some(snapshot.clone());
                entry.fresh_until = Instant::now() + self.ttl;
                Ok(snapshot)
            });
            let waiters = entry.waiters.take().unwrap_or_default();
            self.prune(&mut state);
            (result, waiters)
        };
        for waiter in waiters {
            let _ = waiter.send(result.clone());
        }
        result
    }

    fn prune(&self, state: &mut HostCacheState) {
        // 在途请求保留原槽位，完成后按 LRU 回收空闲账户缓存。
        while state.entries.len() > self.capacity {
            let oldest = state
                .entries
                .iter()
                .filter(|(_, entry)| entry.waiters.is_none())
                .min_by_key(|(_, entry)| entry.access)
                .map(|(scope, _)| scope.clone());
            let Some(scope) = oldest else { break };
            state.entries.remove(&scope);
        }
    }
}

impl HostCacheLease {
    pub(crate) fn finish(
        mut self,
        result: Result<Vec<DataHostSummary>, WebApiError>,
    ) -> HostResult {
        self.finished = true;
        self.cache.complete(&self.scope, self.generation, result)
    }
}

impl Drop for HostCacheLease {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.cache.complete(
                &self.scope,
                self.generation,
                Err(WebApiError::parse("主机查询已中断，请重试")),
            );
        }
    }
}

#[cfg(test)]
#[path = "host_cache.test.rs"]
mod tests;
