//! 清理编排：本地撤销、幂等远端回收、后台退避与退出收口。
use super::*;

impl<S, P> SessionFlowService<S, P>
where
    S: SessionFlowSnapshot,
    P: SessionFlowProvider,
{
    pub async fn close_session(&self, session_id: &str) -> Result<bool, SessionFlowError> {
        let flow = Self {
            inner: Arc::clone(&self.inner),
        };
        let session_id = session_id.to_owned();
        // 本地撤销和远端回收共同归属独立任务，调用方取消等待时清理继续完成。
        tokio::spawn(async move { flow.close_session_inner(&session_id, true).await })
            .await
            .map_err(|error| {
                SessionFlowError::message(format!("sessionCleanupTaskFailed:{error}"))
            })?
    }

    pub(super) async fn close_session_inner(
        &self,
        session_id: &str,
        cancel_startup: bool,
    ) -> Result<bool, SessionFlowError> {
        let (entry, first_close) = {
            let mut cleanups = self.inner.cleanups.lock().await;
            if let Some(entry) = cleanups.get(session_id) {
                (Arc::clone(entry), false)
            } else {
                let Some(record) = self.get_session_record(session_id).await else {
                    return Ok(false);
                };
                // 撤销读写资格立即唤醒在飞操作；远端 stop 的最新凭证获取在清理阶段独立进行。
                record.cancelled.cancel();
                let entry = Arc::new(SessionCleanupEntry {
                    plan: record.plan.clone(),
                    cancelled: record.cancelled.clone(),
                    result: tokio::sync::Mutex::new(None),
                    completion: std::sync::atomic::AtomicU64::new(0),
                    retry_scheduled: std::sync::atomic::AtomicBool::new(false),
                    started_at: std::time::Instant::now(),
                });
                cleanups.insert(session_id.to_owned(), Arc::clone(&entry));
                self.inner
                    .clear_session(session_id, &record.cancelled)
                    .await;
                (entry, true)
            }
        };
        if first_close {
            self.inner.provider.on_session_lifecycle_event("cleanupStarted", Some(session_id), serde_json::json!({
                "operationId": format!("remote-cleanup-{}", entry.cancelled.generation()),
                "stage": "remoteCleanup", "generation": entry.cancelled.generation(), "deadlineMs": 15_000,
            }));
        }
        if cancel_startup {
            self.inner
                .startups
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .cancel_session(session_id, &entry.cancelled);
        }
        let result = self.try_remote_cleanup(session_id, &entry).await;
        if result.is_err() {
            self.schedule_cleanup_retry(session_id.to_owned(), entry);
        }
        result
    }

    async fn try_remote_cleanup(
        &self,
        session_id: &str,
        entry: &Arc<SessionCleanupEntry>,
    ) -> Result<bool, SessionFlowError> {
        let observed_completion = entry.completion.load(std::sync::atomic::Ordering::Acquire);
        let mut previous = entry.result.lock().await;
        if let Some(result) = previous.as_ref() {
            if result.is_ok()
                || entry.completion.load(std::sync::atomic::Ordering::Acquire)
                    != observed_completion
            {
                return result.clone();
            }
        }
        let result = tokio::time::timeout(std::time::Duration::from_secs(15), async {
            let api = self.create_session_api(&entry.plan).await?;
            match api.stop_stream(&session_id).await {
                Ok(()) => Ok(true),
                Err(error) => {
                    let error = map_webapi_error(error);
                    if is_remote_session_not_found(error.status) {
                        Ok(false)
                    } else {
                        Err(error)
                    }
                }
            }
        })
        .await
        .unwrap_or_else(|_| {
            Err(SessionFlowError::message(
                "operationTimeout:stage=remoteCleanup",
            ))
        });
        self.inner.provider.on_session_lifecycle_event(
            if result.is_ok() {
                "cleanupFinished"
            } else {
                "cleanupPending"
            },
            Some(session_id),
            serde_json::json!({
                "operationId": format!("remote-cleanup-{}", entry.cancelled.generation()),
                "stage": "remoteCleanup", "generation": entry.cancelled.generation(),
                "result": if result.is_ok() { "success" } else { "pending" },
                "elapsedMs": entry.started_at.elapsed().as_millis() as u64,
                "status": result.as_ref().err().and_then(|error| error.status),
                "localResourceReleased": true,
            }),
        );
        *previous = Some(result.clone());
        entry
            .completion
            .fetch_add(1, std::sync::atomic::Ordering::Release);
        drop(previous);
        if result.is_ok() {
            self.inner
                .startups
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .release_session(&session_id, &entry.cancelled);
            let mut cleanups = self.inner.cleanups.lock().await;
            if cleanups
                .get(session_id)
                .is_some_and(|current| Arc::ptr_eq(current, entry))
            {
                cleanups.remove(session_id);
            }
        }
        result
    }

    fn schedule_cleanup_retry(&self, session_id: String, entry: Arc<SessionCleanupEntry>) {
        use std::sync::atomic::Ordering;
        if entry.retry_scheduled.swap(true, Ordering::AcqRel) {
            return;
        }
        let weak = Arc::downgrade(&self.inner);
        tokio::spawn(async move {
            let mut backoff_secs = 1;
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(backoff_secs)).await;
                let Some(inner) = weak.upgrade() else {
                    break;
                };
                if inner
                    .startups
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .is_shutting_down()
                {
                    break;
                }
                let current = inner
                    .cleanups
                    .lock()
                    .await
                    .get(&session_id)
                    .is_some_and(|current| Arc::ptr_eq(current, &entry));
                if !current {
                    break;
                }
                let flow = Self { inner };
                if flow.try_remote_cleanup(&session_id, &entry).await.is_ok() {
                    break;
                }
                // 保留记录并按有界退避重试；等待时释放服务强引用。
                backoff_secs = (backoff_secs * 2).min(30);
            }
            entry.retry_scheduled.store(false, Ordering::Release);
        });
    }

    pub(super) async fn ensure_target_cleanup(&self, plan: &Plan) -> Result<(), SessionFlowError> {
        self.inner
            .startups
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .ensure_create_outcome_resolved(
                plan.session.target.as_str(),
                &plan.session.target_id,
            )?;
        let pending: Vec<_> = self
            .inner
            .cleanups
            .lock()
            .await
            .iter()
            .filter(|(_, entry)| {
                entry.plan.session.target == plan.session.target
                    && entry.plan.session.target_id == plan.session.target_id
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in pending {
            self.close_session(&id).await?;
        }
        if self.inner.cleanups.lock().await.len() >= 256 {
            return Err(SessionFlowError::message(
                "sessionCleanupRegistryAtCapacity",
            ));
        }
        Ok(())
    }

    /// 退出同时撤销 startup，覆盖在飞 create、当前会话和待回收记录，总等待有界。
    pub async fn shutdown(&self) {
        self.inner
            .startups
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .shutdown();
        let _ = tokio::time::timeout(std::time::Duration::from_secs(30), async {
            let mut scheduled = std::collections::HashSet::new();
            let mut tasks = tokio::task::JoinSet::new();
            loop {
                let mut ids = self.inner.sessions.read().await.keys();
                ids.extend(self.inner.cleanups.lock().await.keys().cloned());
                for id in ids {
                    if scheduled.insert(id.clone()) {
                        let flow = Self {
                            inner: Arc::clone(&self.inner),
                        };
                        tasks.spawn(async move {
                            let _ = flow.close_session(&id).await;
                        });
                    }
                }
                while tasks.try_join_next().is_some() {}
                let active = self
                    .inner
                    .startups
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .active_count();
                if active == 0 && tasks.is_empty() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await;
    }

    pub(super) async fn cleanup_session_for_recreate(&self, session_id: &str) {
        let record = self.get_session_record(session_id).await;
        let flow = Self {
            inner: Arc::clone(&self.inner),
        };
        let id = session_id.to_owned();
        let cleanup = tokio::spawn(async move { flow.close_session_inner(&id, false).await })
            .await
            .unwrap_or_else(|error| {
                Err(SessionFlowError::message(format!(
                    "sessionCleanupTaskFailed:{error}"
                )))
            });
        if let Err(error) = cleanup {
            log::warn!(
                "best-effort startup retry cleanup failed, continuing with recreate: session_id={} error={}",
                session_id,
                error,
            );
        }
        if let Some(record) = record {
            self.inner
                .clear_session(session_id, &record.cancelled)
                .await;
        }
    }

    pub(super) async fn wait_until_session_cleanup_settled(
        &self,
        plan: &Plan,
        session_id: &str,
    ) -> bool {
        match tokio::time::timeout(
            std::time::Duration::from_millis(HOME_RECREATE_CLEANUP_SETTLE_MS),
            self.poll_session_cleanup_settled(plan, session_id),
        )
        .await
        {
            Ok(settled) => settled,
            Err(_) => {
                self.inner.provider.on_session_recreate_cleanup_result(
                    session_id,
                    plan.session.target.as_str(),
                    &plan.session.target_id,
                    "timeout",
                    None,
                    None,
                );
                false
            }
        }
    }

    async fn poll_session_cleanup_settled(&self, plan: &Plan, session_id: &str) -> bool {
        let target_type = plan.session.target.as_str().to_string();
        let target_id = plan.session.target_id.clone();
        let api = match self.create_session_api(plan).await {
            Ok(api) => api,
            Err(error) => {
                self.inner.provider.on_session_recreate_cleanup_result(
                    session_id,
                    &target_type,
                    &target_id,
                    "createApiFailed",
                    None,
                    Some(&error),
                );
                return false;
            }
        };

        let started_at_ms = now_ms();
        let mut last_state: Option<String> = None;
        let mut last_error: Option<SessionFlowError> = None;

        loop {
            match api.get_stream_state(session_id).await {
                Ok((state, _)) => {
                    last_state = state.clone();
                    if is_session_cleanup_terminal_state(state.as_deref()) {
                        self.inner.provider.on_session_recreate_cleanup_result(
                            session_id,
                            &target_type,
                            &target_id,
                            "settled",
                            last_state.as_deref(),
                            None,
                        );
                        return true;
                    }
                }
                Err(error) => {
                    let flow_error = map_webapi_error(error);
                    if flow_error.status == Some(404) {
                        self.inner.provider.on_session_recreate_cleanup_result(
                            session_id,
                            &target_type,
                            &target_id,
                            "notFound",
                            last_state.as_deref(),
                            None,
                        );
                        return true;
                    }
                    last_error = Some(flow_error);
                }
            }

            if now_ms().saturating_sub(started_at_ms) >= HOME_RECREATE_CLEANUP_SETTLE_MS {
                self.inner.provider.on_session_recreate_cleanup_result(
                    session_id,
                    &target_type,
                    &target_id,
                    "timeout",
                    last_state.as_deref(),
                    last_error.as_ref(),
                );
                return false;
            }

            tokio::time::sleep(std::time::Duration::from_millis(
                HOME_RECREATE_CLEANUP_POLL_MS,
            ))
            .await;
        }
    }
}
