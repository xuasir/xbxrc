use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::policy::Plan;
use crate::session::monitor::SessionMonitorMetadata;

/// 会话取消令牌：监控循环只读这个标记，不关心存储实现。
#[derive(Debug, Clone, Default)]
pub struct SessionCancelToken {
    inner: Arc<AtomicBool>,
}

impl SessionCancelToken {
    pub fn is_cancelled(&self) -> bool {
        self.inner.load(Ordering::Relaxed)
    }

    pub fn cancel(&self) {
        self.inner.store(true, Ordering::Relaxed);
    }
}

/// 运行态会话记录：快照 + 策略计划 + 监控元数据。
/// RFC: Plan 是执行决策，持有它能让 provider 结合最新凭证随时重建网关。
#[derive(Debug, Clone)]
pub struct SessionRuntimeRecord<T: Clone> {
    pub snapshot: T,
    pub plan: Plan,
    pub metadata: SessionMonitorMetadata,
    pub cancelled: SessionCancelToken,
}

impl<T: Clone> SessionRuntimeRecord<T> {
    pub fn new(snapshot: T, plan: Plan, created_at_ms: u64) -> Self {
        Self {
            snapshot,
            plan,
            metadata: SessionMonitorMetadata {
                created_at_ms,
                last_observed_state: None,
                state_observed_at_ms: None,
                repeated_state_count: 0,
                monitor_attempt_count: 0,
            },
            cancelled: SessionCancelToken::default(),
        }
    }
}

/// 会话内存存储：负责 insert/remove/list 与取消标记联动。
#[derive(Debug, Clone)]
pub struct SessionRuntimeStore<T: Clone> {
    records: HashMap<String, SessionRuntimeRecord<T>>,
}

impl<T: Clone> Default for SessionRuntimeStore<T> {
    fn default() -> Self {
        Self {
            records: HashMap::new(),
        }
    }
}

impl<T: Clone> SessionRuntimeStore<T> {
    pub fn insert_new_with_plan(
        &mut self,
        session_id: String,
        snapshot: T,
        plan: Plan,
        created_at_ms: u64,
    ) -> SessionCancelToken {
        let record = SessionRuntimeRecord::new(snapshot, plan, created_at_ms);
        let cancel = record.cancelled.clone();

        if let Some(old) = self.records.insert(session_id, record) {
            old.cancelled.cancel();
        }

        cancel
    }

    pub fn get(&self, session_id: &str) -> Option<SessionRuntimeRecord<T>> {
        self.records.get(session_id).cloned()
    }

    pub fn upsert(&mut self, session_id: String, mut record: SessionRuntimeRecord<T>) {
        if let Some(old) = self.records.remove(&session_id) {
            // 同一 session 的状态回写必须沿用既有 cancel token，
            // 否则后台 monitor/keepalive loop 会被自己的 upsert 提前取消。
            record.cancelled = old.cancelled;
        }
        self.records.insert(session_id, record);
    }

    pub fn remove(&mut self, session_id: &str) -> Option<SessionRuntimeRecord<T>> {
        self.records.remove(session_id).map(|record| {
            record.cancelled.cancel();
            record
        })
    }

    pub fn keys(&self) -> Vec<String> {
        self.records.keys().cloned().collect::<Vec<_>>()
    }

    pub fn list_snapshots<P>(&self, mut predicate: P) -> Vec<T>
    where
        P: FnMut(&T) -> bool,
    {
        self.records
            .values()
            .filter(|record| predicate(&record.snapshot))
            .map(|record| record.snapshot.clone())
            .collect::<Vec<_>>()
    }
}

#[cfg(test)]
mod tests {
    use super::{SessionRuntimeRecord, SessionRuntimeStore};

    #[test]
    fn upsert_preserves_cancel_token_for_same_session() {
        let mut store = SessionRuntimeStore::default();
        let cancel = store.insert_new_with_plan(
            "session-1".to_string(),
            "snapshot-1".to_string(),
            crate::policy::Plan::default(),
            1,
        );

        let mut updated =
            SessionRuntimeRecord::new("snapshot-2".to_string(), crate::policy::Plan::default(), 2);
        updated.metadata.repeated_state_count = 3;

        store.upsert("session-1".to_string(), updated);

        let record = store.get("session-1").expect("record should exist");
        assert_eq!(record.snapshot, "snapshot-2");
        assert_eq!(record.metadata.repeated_state_count, 3);
        assert!(!cancel.is_cancelled());
        assert!(!record.cancelled.is_cancelled());
    }
}
