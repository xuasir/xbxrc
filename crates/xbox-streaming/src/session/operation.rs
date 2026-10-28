use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::flow::SessionFlowError;
use super::store::SessionCancelToken;

#[derive(Clone)]
pub(crate) struct OwnedSession {
    pub id: String,
    pub cancelled: SessionCancelToken,
}

#[derive(Default)]
struct StartupState {
    cancelled: SessionCancelToken,
    owned: Mutex<Option<OwnedSession>>,
    creating: AtomicBool,
}

/// 启动作用域在解析上下文前建立，远端创建完成后的清理继续归属同一作用域。
#[derive(Clone, Default)]
pub struct SessionStartupScope {
    inner: Arc<StartupState>,
}

impl SessionStartupScope {
    pub fn generation(&self) -> u64 {
        self.inner.cancelled.generation()
    }

    pub fn cancel(&self) {
        self.inner.cancelled.cancel();
    }
    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.is_cancelled()
    }

    pub async fn run<T>(
        &self,
        stage: &str,
        timeout: Option<Duration>,
        operation: impl Future<Output = Result<T, SessionFlowError>>,
    ) -> Result<T, SessionFlowError> {
        let result = self.inner.cancelled.run(stage, timeout, operation).await;
        if result
            .as_ref()
            .is_err_and(|error| error.message.starts_with("operationTimeout:"))
        {
            self.cancel();
        }
        result
    }

    pub(crate) async fn wait_for_creation(&self) -> Result<(), SessionFlowError> {
        tokio::time::timeout(Duration::from_secs(15), async {
            while self.inner.creating.load(Ordering::Acquire) {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .map_err(|_| SessionFlowError::message("previousStartupStillSettling"))
    }

    pub(crate) fn owned(&self) -> Option<OwnedSession> {
        self.inner
            .owned
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    pub(crate) fn set_owned(&self, id: String, cancelled: SessionCancelToken) {
        *self.inner.owned.lock().unwrap_or_else(|p| p.into_inner()) =
            Some(OwnedSession { id, cancelled });
    }

    pub(crate) fn release_owned(&self) {
        self.inner
            .owned
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take();
    }

    pub(crate) fn begin_creation(&self) -> CreationGuard {
        self.inner.creating.store(true, Ordering::Release);
        CreationGuard(self.clone())
    }
}

pub(crate) struct CreationGuard(SessionStartupScope);
impl Drop for CreationGuard {
    fn drop(&mut self) {
        self.0.inner.creating.store(false, Ordering::Release);
    }
}

struct StartupEntry {
    scope: SessionStartupScope,
    target: Option<(String, String)>,
    updated: Instant,
    finished: bool,
}

#[derive(Default)]
pub(crate) struct StartupRegistry {
    entries: HashMap<String, StartupEntry>,
    active: HashMap<u64, SessionStartupScope>,
    unknown_targets: HashMap<(String, String), u64>,
    sequence: u64,
    shutting_down: bool,
}

impl StartupRegistry {
    fn prune(&mut self) {
        self.entries.retain(|_, entry| {
            !entry.finished
                || entry.scope.inner.creating.load(Ordering::Acquire)
                || entry.scope.owned().is_some()
                || entry.updated.elapsed() < Duration::from_secs(300)
        });
    }

    pub fn begin(
        &mut self,
        id: &str,
        target_type: &str,
        target_id: &str,
    ) -> Result<SessionStartupScope, SessionFlowError> {
        if self.shutting_down {
            return Err(cancelled_error("shutdown"));
        }
        self.prune();
        if let Some(entry) = self.entries.get(id) {
            return Err(if entry.scope.is_cancelled() {
                cancelled_error("startup")
            } else {
                SessionFlowError::message("startupAttemptAlreadyExists")
            });
        }
        let target = (target_type.to_owned(), target_id.to_owned());
        self.ensure_create_outcome_resolved(target_type, target_id)?;
        if self.entries.values().any(|entry| {
            entry.target.as_ref() == Some(&target)
                && (entry.scope.inner.creating.load(Ordering::Acquire)
                    || !entry.finished
                    || entry.scope.owned().is_some())
        }) {
            return Err(SessionFlowError::message("previousStartupStillSettling"));
        }
        if self.entries.len() >= 256 {
            return Err(SessionFlowError::message("startupRegistryAtCapacity"));
        }
        let scope = SessionStartupScope::default();
        self.entries.insert(
            id.to_owned(),
            StartupEntry {
                scope: scope.clone(),
                target: Some(target),
                updated: Instant::now(),
                finished: false,
            },
        );
        Ok(scope)
    }

    pub fn cancel(&mut self, id: &str) -> Result<SessionStartupScope, SessionFlowError> {
        self.prune();
        if !self.entries.contains_key(id) {
            if self.entries.len() >= 256 {
                return Err(SessionFlowError::message("startupRegistryAtCapacity"));
            }
            self.entries.insert(
                id.to_owned(),
                StartupEntry {
                    scope: SessionStartupScope::default(),
                    target: None,
                    updated: Instant::now(),
                    finished: true,
                },
            );
        }
        let entry = self.entries.get_mut(id).expect("entry inserted above");
        entry.updated = Instant::now();
        entry.scope.cancel();
        Ok(entry.scope.clone())
    }

    pub fn finish(&mut self, id: &str) {
        if let Some(entry) = self.entries.get_mut(id) {
            entry.finished = true;
            entry.updated = Instant::now();
        }
    }

    /// create 结果缺失时保留原目标归属；后续入口须先取得服务端资源结论。
    pub fn record_unknown_create(&mut self, target_type: &str, target_id: &str, generation: u64) {
        self.unknown_targets
            .insert((target_type.to_owned(), target_id.to_owned()), generation);
    }

    pub fn ensure_create_outcome_resolved(
        &self,
        target_type: &str,
        target_id: &str,
    ) -> Result<(), SessionFlowError> {
        if self
            .unknown_targets
            .contains_key(&(target_type.to_owned(), target_id.to_owned()))
        {
            return Err(SessionFlowError::message("createOutcomeUnresolved").with_startup_hint(super::flow::SessionFlowStartupErrorHint {
                kind: super::flow::SessionFlowStartupErrorKind::SessionCreate,
                retryable: false,
                diagnostic_summary: "createOutcomeUnresolved; remote resource identity requires service confirmation".to_owned(),
            }));
        }
        if self.unknown_targets.len() >= 256 {
            return Err(SessionFlowError::message("unknownCreateRegistryAtCapacity"));
        }
        Ok(())
    }

    pub fn cancel_session(&mut self, id: &str, cancelled: &SessionCancelToken) {
        for entry in self.entries.values_mut() {
            if entry
                .scope
                .owned()
                .is_some_and(|owned| owned.id == id && owned.cancelled.same_instance(cancelled))
            {
                entry.scope.cancel();
                entry.updated = Instant::now();
            }
        }
    }

    pub fn release_session(&mut self, id: &str, cancelled: &SessionCancelToken) {
        for entry in self.entries.values_mut() {
            if entry
                .scope
                .owned()
                .is_some_and(|owned| owned.id == id && owned.cancelled.same_instance(cancelled))
            {
                entry.scope.release_owned();
                entry.updated = Instant::now();
            }
        }
    }

    /// 所有公共入口共享退出作用域；后台 create 完成清理后才释放活动记录。
    pub fn track(
        registry: &Arc<Mutex<Self>>,
        scope: &SessionStartupScope,
    ) -> Result<StartupOperationGuard, SessionFlowError> {
        let mut state = registry.lock().unwrap_or_else(|p| p.into_inner());
        if state.shutting_down {
            scope.cancel();
            return Err(cancelled_error("shutdown"));
        }
        if state.active.len() + state.unknown_targets.len() >= 256 {
            return Err(SessionFlowError::message("startupRegistryAtCapacity"));
        }
        state.sequence = state.sequence.wrapping_add(1);
        let id = state.sequence;
        state.active.insert(id, scope.clone());
        Ok(StartupOperationGuard {
            registry: Arc::clone(registry),
            id,
        })
    }

    pub fn shutdown(&mut self) {
        self.shutting_down = true;
        for scope in self.active.values() {
            scope.cancel();
        }
        for entry in self.entries.values_mut() {
            entry.scope.cancel();
            entry.finished = true;
        }
    }

    pub fn is_shutting_down(&self) -> bool {
        self.shutting_down
    }

    pub fn active_count(&self) -> usize {
        self.active.len()
    }
}

pub(crate) struct StartupOperationGuard {
    registry: Arc<Mutex<StartupRegistry>>,
    id: u64,
}

impl Drop for StartupOperationGuard {
    fn drop(&mut self) {
        self.registry
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .active
            .remove(&self.id);
    }
}

/// 健康云排队暂停准备预算，准备状态反复切换继续消耗同一个预算。
pub(crate) struct PreparationBudget {
    last_tick_ms: u64,
    elapsed_ms: u64,
    queued: bool,
}
impl PreparationBudget {
    pub fn new(now_ms: u64) -> Self {
        Self {
            last_tick_ms: now_ms,
            elapsed_ms: 0,
            queued: false,
        }
    }
    pub fn observe(&mut self, now_ms: u64, queued: bool, timeout_ms: u64) -> bool {
        if !self.queued {
            self.elapsed_ms = self
                .elapsed_ms
                .saturating_add(now_ms.saturating_sub(self.last_tick_ms));
        }
        self.last_tick_ms = now_ms;
        self.queued = queued;
        self.elapsed_ms >= timeout_ms
    }
}

/// 同一单调时钟供启动、monitor 与恢复等待共享；墙钟由 trace 单独记录。
pub(crate) fn now_ms() -> u64 {
    static ORIGIN: std::sync::OnceLock<tokio::time::Instant> = std::sync::OnceLock::new();
    ORIGIN
        .get_or_init(tokio::time::Instant::now)
        .elapsed()
        .as_millis() as u64
}

impl SessionCancelToken {
    pub(crate) async fn run<T>(
        &self,
        stage: &str,
        timeout: Option<Duration>,
        operation: impl Future<Output = Result<T, SessionFlowError>>,
    ) -> Result<T, SessionFlowError> {
        tokio::select! {
            biased;
            _ = self.cancelled() => Err(cancelled_error(stage)),
            result = async {
                match timeout {
                    Some(duration) => tokio::time::timeout(duration, operation).await
                        .map_err(|_| SessionFlowError::message(format!("operationTimeout:stage={stage}")))?,
                    None => operation.await,
                }
            } => result,
        }
    }
}

pub(crate) fn cancelled_error(stage: &str) -> SessionFlowError {
    SessionFlowError::message(format!("operationCancelled:stage={stage}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_create_keeps_target_ownership_after_attempt_finishes() {
        let mut registry = StartupRegistry::default();
        let scope = registry.begin("attempt", "cloud", "title").unwrap();
        registry.record_unknown_create("cloud", "title", scope.generation());
        registry.finish("attempt");
        registry.cancel("attempt").unwrap();
        let error = registry.begin("retry", "cloud", "title").err().unwrap();
        assert_eq!(error.message, "createOutcomeUnresolved");
        assert!(!error.startup_hint.unwrap().retryable);
        assert!(registry.begin("different", "cloud", "other-title").is_ok());
    }

    #[test]
    fn cancellation_before_registration_is_absorbing() {
        let mut registry = StartupRegistry::default();
        registry.cancel("attempt-1").unwrap();
        assert!(registry.begin("attempt-1", "home", "console").is_err());
        assert!(registry.begin("attempt-2", "home", "console").is_ok());
    }

    #[tokio::test]
    async fn cancellation_wakes_an_in_flight_wait_and_is_replayed_to_late_waiters() {
        let scope = SessionStartupScope::default();
        let waiting = scope.clone();
        let task = tokio::spawn(async move {
            waiting
                .run::<()>("context", None, std::future::pending())
                .await
        });
        tokio::task::yield_now().await;
        scope.cancel();
        assert!(tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap()
            .is_err());
        assert!(scope.run("late", None, async { Ok(()) }).await.is_err());
    }

    #[test]
    fn a_cancelled_remote_create_keeps_ownership_until_its_response_settles() {
        let mut registry = StartupRegistry::default();
        let scope = registry.begin("attempt-1", "home", "console").unwrap();
        let creation = scope.begin_creation();
        registry.cancel("attempt-1").unwrap();
        registry.finish("attempt-1");
        assert!(registry.begin("attempt-2", "home", "console").is_err());
        drop(creation);
        assert!(registry.begin("attempt-2", "home", "console").is_ok());
    }

    #[test]
    fn shutdown_cancels_tracked_public_operations_and_blocks_new_entries() {
        let registry = Arc::new(Mutex::new(StartupRegistry::default()));
        let scope = SessionStartupScope::default();
        let guard = StartupRegistry::track(&registry, &scope).unwrap();
        registry.lock().unwrap().shutdown();
        assert!(scope.is_cancelled());
        assert_eq!(registry.lock().unwrap().active_count(), 1);
        drop(guard);
        assert_eq!(registry.lock().unwrap().active_count(), 0);
        assert!(registry
            .lock()
            .unwrap()
            .begin("new", "cloud", "title")
            .is_err());
        assert!(StartupRegistry::track(&registry, &SessionStartupScope::default()).is_err());
    }

    #[test]
    fn active_operations_are_bounded_and_completed_tombstones_preserve_live_resources() {
        let registry = Arc::new(Mutex::new(StartupRegistry::default()));
        let scope = SessionStartupScope::default();
        let guards: Vec<_> = (0..256)
            .map(|_| StartupRegistry::track(&registry, &scope).unwrap())
            .collect();
        assert!(StartupRegistry::track(&registry, &scope).is_err());
        drop(guards);
        assert_eq!(registry.lock().unwrap().active_count(), 0);
        let mut state = registry.lock().unwrap();
        let retained = state.begin("retained", "home", "console").unwrap();
        let token = SessionCancelToken::default();
        retained.set_owned("session".to_owned(), token.clone());
        state.finish("retained");
        state.entries.get_mut("retained").unwrap().updated =
            Instant::now() - Duration::from_secs(301);
        for i in 0..255 {
            let id = format!("completed-{i}");
            state.begin(&id, "cloud", &id).unwrap();
            state.finish(&id);
            state.entries.get_mut(&id).unwrap().updated = Instant::now() - Duration::from_secs(301);
        }
        state.prune();
        assert!(state.entries.contains_key("retained"));
        assert!(state.begin("same-target", "home", "console").is_err());
        state.release_session("session", &token);
        assert!(state.begin("next-target", "home", "console").is_ok());
    }
    #[test]
    fn healthy_cloud_queue_preserves_preparation_budget_across_180_seconds() {
        let mut budget = PreparationBudget::new(0);
        assert!(!budget.observe(1_000, true, 90_000));
        for now in (2_000..=181_000).step_by(1_000) {
            assert!(!budget.observe(now, true, 90_000));
        }
        assert!(!budget.observe(182_000, false, 90_000));
        assert!(!budget.observe(250_000, false, 90_000));
        assert!(budget.observe(271_000, false, 90_000));
    }

    #[tokio::test]
    async fn startup_deadline_revokes_the_scope_and_absorbs_late_work() {
        let scope = SessionStartupScope::default();
        let error = scope
            .run::<()>(
                "context",
                Some(Duration::from_millis(1)),
                std::future::pending(),
            )
            .await
            .unwrap_err();
        assert_eq!(error.message, "operationTimeout:stage=context");
        assert!(scope.is_cancelled());
        assert!(scope.run("late", None, async { Ok(()) }).await.is_err());
    }
}
