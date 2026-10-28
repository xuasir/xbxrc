//! 启动编排：作用域、创建、排队/准备与有限自动重试。
use super::*;

impl<S, P> SessionFlowService<S, P>
where
    S: SessionFlowSnapshot,
    P: SessionFlowProvider,
{
    pub async fn create_session(&self, plan: Plan) -> Result<S, SessionFlowError> {
        self.create_session_with_observer::<NoopSessionStartupObserver>(
            plan,
            None,
            None,
            &SessionStartupScope::default(),
        )
        .await
    }

    pub fn begin_startup(
        &self,
        attempt_id: &str,
        target_type: &str,
        target_id: &str,
    ) -> Result<SessionStartupScope, SessionFlowError> {
        self.inner
            .startups
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .begin(attempt_id, target_type, target_id)
    }

    pub fn finish_startup(&self, attempt_id: &str) {
        self.inner
            .startups
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .finish(attempt_id);
    }

    pub async fn cancel_startup(&self, attempt_id: &str) -> Result<(), SessionFlowError> {
        let scope = self
            .inner
            .startups
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .cancel(attempt_id)?;
        scope.wait_for_creation().await?;
        self.finish_startup(attempt_id);
        if let Some(owned) = scope.owned() {
            let record = self.get_session_record(&owned.id).await;
            if record
                .as_ref()
                .is_some_and(|record| !record.cancelled.same_instance(&owned.cancelled))
            {
                scope.release_owned();
                return Ok(());
            }
            if self
                .inner
                .cleanups
                .lock()
                .await
                .get(&owned.id)
                .is_some_and(|entry| !entry.cancelled.same_instance(&owned.cancelled))
            {
                scope.release_owned();
                return Ok(());
            }
            self.close_session(&owned.id).await?;
            scope.release_owned();
        }
        Ok(())
    }

    pub(super) async fn create_session_with_observer<O>(
        &self,
        plan: Plan,
        observer: Option<&O>,
        recreate_from_session_id: Option<&str>,
        scope: &SessionStartupScope,
    ) -> Result<S, SessionFlowError>
    where
        O: SessionStartupObserver,
    {
        let active_scope = StartupRegistry::track(&self.inner.startups, scope)?;
        self.ensure_target_cleanup(&plan).await?;
        let api = scope
            .run(
                "sessionAccess",
                Some(std::time::Duration::from_secs(15)),
                self.create_session_api(&plan),
            )
            .await?;
        notify_startup_phase(
            observer,
            SessionStartupPhase::CreatingSession,
            SessionStartupPhaseStatus::Entered,
            None,
        );
        // 已发送 create 的 HTTP 保留到响应落定，取消后的响应由原作用域自行清理。
        let flow = Self {
            inner: Arc::clone(&self.inner),
        };
        let task_scope = scope.clone();
        let previous = recreate_from_session_id.map(str::to_owned);
        let create_timeout =
            std::time::Duration::from_millis(plan.session.schedule.ready_timeout_ms);
        let creation = scope.begin_creation();
        let created = tokio::spawn(async move {
            let _creation = creation;
            let _active_scope = active_scope;
            flow.create_session_record(plan, api, previous.as_deref(), &task_scope)
                .await
        });
        let result = scope
            .run("creatingSession", Some(create_timeout), async {
                created.await.map_err(|error| {
                    SessionFlowError::message(format!("sessionCreateTaskFailed:{error}"))
                })?
            })
            .await?;
        notify_startup_phase(
            observer,
            SessionStartupPhase::CreatingSession,
            SessionStartupPhaseStatus::Succeeded,
            None,
        );

        Ok(result)
    }

    async fn create_session_record(
        &self,
        plan: Plan,
        api: WebApiSessionGateway,
        recreate_from_session_id: Option<&str>,
        scope: &SessionStartupScope,
    ) -> Result<S, SessionFlowError> {
        let monitor_interval_ms = plan.session.schedule.monitor_interval_ms.max(200);
        let keepalive_interval_ms = plan.session.schedule.keepalive_interval_ms.max(1_000);
        let session_path = self.start_stream_with_retry(&api, &plan, scope).await?;

        let session_id = parse_session_id_from_path(&session_path).map_err(|error| {
            self.record_unknown_create(&plan, scope);
            SessionFlowError::message(error.to_string())
        })?;

        let target_type = plan.session.target.as_str();
        self.inner.provider.on_session_created(
            &session_id,
            &session_path,
            target_type,
            &plan.session.target_id,
            recreate_from_session_id,
        );
        let snapshot = S::new_pending(
            session_id.clone(),
            session_path,
            plan.session.target_id.clone(),
            target_type.to_string(),
        );
        let cancelled = {
            let mut sessions = self.inner.sessions.write().await;
            let cancelled =
                sessions.insert_new_with_plan(session_id.clone(), snapshot.clone(), plan, now_ms());
            self.inner.signaling.write().await.remove(&session_id);
            self.inner.mutations.write().await.insert(
                session_id.clone(),
                Arc::new(SessionMutationEntry {
                    cancelled: cancelled.clone(),
                    state: tokio::sync::Mutex::new(SessionMutationState::default()),
                    negotiation: tokio::sync::Mutex::new(()),
                    ice: tokio::sync::Mutex::new(()),
                }),
            );
            scope.set_owned(session_id.clone(), cancelled.clone());
            cancelled
        };
        self.inner.provider.on_session_lifecycle_event(
            "sessionResourceCreated",
            Some(&session_id),
            serde_json::json!({
                "generation": cancelled.generation(), "startupGeneration": scope.generation(),
                "targetType": target_type,
            }),
        );
        if scope.is_cancelled() {
            self.close_session(&session_id).await?;
            return Err(cancelled_error("creatingSession"));
        }

        let scheduler = SessionScheduler::new(Arc::clone(&self.inner));
        scheduler
            .start_loops(
                session_id,
                cancelled,
                monitor_interval_ms,
                keepalive_interval_ms,
            )
            .await;

        Ok(snapshot)
    }

    pub async fn create_session_execution<R, E, FR, FE>(
        &self,
        plan: Plan,
        project_runtime: FR,
        project_render: FE,
    ) -> Result<SessionExecutionSnapshot<S, R, E>, SessionFlowError>
    where
        R: Clone,
        E: Clone,
        FR: Fn(&Plan) -> R,
        FE: Fn(&Plan) -> E,
    {
        let runtime = project_runtime(&plan);
        let render = project_render(&plan);
        let session = self.create_session(plan).await?;
        Ok(SessionExecutionSnapshot {
            session,
            runtime,
            render,
        })
    }

    /// 启动编排入口：在 session 层串起 wake/preflight/create/wait-started。
    pub async fn start_session_execution<R, E, FR, FE>(
        &self,
        plan: Plan,
        project_runtime: FR,
        project_render: FE,
    ) -> Result<SessionExecutionSnapshot<S, R, E>, SessionFlowError>
    where
        R: Clone,
        E: Clone,
        FR: Fn(&Plan) -> R,
        FE: Fn(&Plan) -> E,
    {
        self.start_session_execution_with_observer::<R, E, FR, FE, NoopSessionStartupObserver>(
            plan,
            project_runtime,
            project_render,
            None,
        )
        .await
    }

    pub async fn start_session_execution_with_observer<R, E, FR, FE, O>(
        &self,
        plan: Plan,
        project_runtime: FR,
        project_render: FE,
        observer: Option<&O>,
    ) -> Result<SessionExecutionSnapshot<S, R, E>, SessionFlowError>
    where
        R: Clone,
        E: Clone,
        FR: Fn(&Plan) -> R,
        FE: Fn(&Plan) -> E,
        O: SessionStartupObserver,
    {
        self.start_session_execution_with_scope(
            plan,
            project_runtime,
            project_render,
            observer,
            &SessionStartupScope::default(),
        )
        .await
    }

    pub async fn start_session_execution_with_scope<R, E, FR, FE, O>(
        &self,
        plan: Plan,
        project_runtime: FR,
        project_render: FE,
        observer: Option<&O>,
        scope: &SessionStartupScope,
    ) -> Result<SessionExecutionSnapshot<S, R, E>, SessionFlowError>
    where
        R: Clone,
        E: Clone,
        FR: Fn(&Plan) -> R,
        FE: Fn(&Plan) -> E,
        O: SessionStartupObserver,
    {
        let runtime = project_runtime(&plan);
        let render = project_render(&plan);
        let schedule = plan.session.schedule.clone();
        let mut recreate_retry_count = 0u8;
        let mut recreate_context: Option<SessionRecreateContext> = None;

        scope
            .run(
                "consolePreparation",
                Some(std::time::Duration::from_millis(schedule.ready_timeout_ms)),
                self.prepare_remote_console(&plan, observer),
            )
            .await?;

        loop {
            let recreate_from_session_id = recreate_context
                .as_ref()
                .map(|context| context.previous_session_id.as_str());
            let session = self
                .create_session_with_observer(
                    plan.clone(),
                    observer,
                    recreate_from_session_id,
                    scope,
                )
                .await?;
            let session_id = session.session_id().to_string();
            if let Some(context) = recreate_context.take() {
                if should_fail_home_recreate_same_session(
                    context.cleanup_settled,
                    Some(context.previous_session_id.as_str()),
                    &session_id,
                ) {
                    notify_startup_phase(
                        observer,
                        SessionStartupPhase::WaitingSessionReady,
                        SessionStartupPhaseStatus::Failed,
                        None,
                    );
                    let _ = self.close_session(&session_id).await;
                    return Err(SessionFlowError::message(format!(
                        "homeRecreateReusedSession:sessionId={session_id};cleanupSettled={}",
                        context.cleanup_settled
                    )));
                }
            }
            notify_startup_phase(
                observer,
                SessionStartupPhase::WaitingSessionReady,
                SessionStartupPhaseStatus::Entered,
                None,
            );

            match scope
                .run(
                    "sessionReady",
                    None,
                    self.wait_until_session_started_or_failed(&session_id, &schedule),
                )
                .await
            {
                Ok(_) => {
                    notify_startup_phase(
                        observer,
                        SessionStartupPhase::WaitingSessionReady,
                        SessionStartupPhaseStatus::Succeeded,
                        None,
                    );

                    let started_session = self
                        .get_session(&session_id)
                        .await
                        .ok_or_else(|| missing_session_error(&session_id))?;

                    return Ok(SessionExecutionSnapshot {
                        session: started_session,
                        runtime,
                        render,
                    });
                }
                Err(wait_error) => {
                    let retry_probe = self.get_session_ready_retry_probe(&session_id).await;
                    let retry_decision = decide_home_session_ready_recreate_retry(
                        plan.session.target.is_home(),
                        recreate_retry_count,
                        &wait_error.message,
                        retry_probe.phase,
                        retry_probe.stream_state.as_deref(),
                        retry_probe.error_code.as_deref(),
                        retry_probe.error_message.as_deref(),
                    );

                    if let Some(decision) = retry_decision {
                        match decision {
                            SessionReadyRetryDecision::Retry(reason) => {
                                log::warn!(
                                    "home startup retry will recreate session after session-ready failure: target_id={} session_id={} retry_count={} reason={:?} phase={:?} stream_state={:?} wait_error={} progress_error={:?}",
                                    plan.session.target_id,
                                    session_id,
                                    recreate_retry_count + 1,
                                    reason,
                                    retry_probe.phase,
                                    retry_probe.stream_state,
                                    wait_error,
                                    retry_probe.error_message,
                                );
                                notify_startup_bounded_retry(
                                    observer,
                                    SessionStartupPhase::WaitingSessionReady,
                                    SessionStartupBoundedRetrySnapshot {
                                        reason: map_session_ready_retry_reason(reason),
                                        status: SessionStartupBoundedRetryStatus::Retrying,
                                        retry_count: recreate_retry_count.saturating_add(1),
                                        retry_limit: HOME_SESSION_READY_RETRY_LIMIT,
                                    },
                                );
                                self.cleanup_session_for_recreate(&session_id).await;
                                let cleanup_settled = scope
                                    .run("recreateCleanupSettle", None, async {
                                        Ok(self
                                            .wait_until_session_cleanup_settled(&plan, &session_id)
                                            .await)
                                    })
                                    .await?;
                                // home 场景下 recreate 前保留空钩子，统一由 play -> state 主链裁决主机唤醒/准备。
                                scope
                                    .run(
                                        "consolePreparation",
                                        Some(std::time::Duration::from_millis(
                                            schedule.ready_timeout_ms,
                                        )),
                                        self.prepare_remote_console(&plan, observer),
                                    )
                                    .await?;
                                recreate_context = Some(SessionRecreateContext {
                                    previous_session_id: session_id,
                                    cleanup_settled,
                                });
                                recreate_retry_count = recreate_retry_count.saturating_add(1);
                                continue;
                            }
                            SessionReadyRetryDecision::Exhausted(reason) => {
                                notify_startup_bounded_retry(
                                    observer,
                                    SessionStartupPhase::WaitingSessionReady,
                                    SessionStartupBoundedRetrySnapshot {
                                        reason: map_session_ready_retry_reason(reason),
                                        status: SessionStartupBoundedRetryStatus::Exhausted,
                                        retry_count: recreate_retry_count,
                                        retry_limit: HOME_SESSION_READY_RETRY_LIMIT,
                                    },
                                );
                                notify_startup_phase(
                                    observer,
                                    SessionStartupPhase::WaitingSessionReady,
                                    SessionStartupPhaseStatus::Failed,
                                    None,
                                );
                                let _ = self.close_session(&session_id).await;
                                return Err(build_home_session_ready_retry_exhausted_error(
                                    &plan.session.target_id,
                                    reason,
                                    recreate_retry_count,
                                    HOME_SESSION_READY_RETRY_LIMIT,
                                    wait_error,
                                ));
                            }
                        }
                    }

                    notify_startup_phase(
                        observer,
                        SessionStartupPhase::WaitingSessionReady,
                        SessionStartupPhaseStatus::Failed,
                        None,
                    );
                    let _ = self.close_session(&session_id).await;
                    return Err(wait_error);
                }
            }
        }
    }

    pub(super) async fn prepare_remote_console<O>(
        &self,
        plan: &Plan,
        observer: Option<&O>,
    ) -> Result<(), SessionFlowError>
    where
        O: SessionStartupObserver,
    {
        let _ = observer;
        if !plan.session.target.is_home() {
            return Ok(());
        }

        let schedule = &plan.session.schedule;
        if schedule.require_console_ready {
            self.wait_until_console_ready(
                plan.session.target.as_str(),
                &plan.session.target_id,
                schedule,
                schedule.wake_console,
            )
            .await?;
            return Ok(());
        }

        if schedule.wake_console {
            self.inner
                .provider
                .power_on_console(&plan.session.target_id)
                .await?;
        }
        Ok(())
    }

    async fn wait_until_console_ready(
        &self,
        target_type: &str,
        target_id: &str,
        schedule: &crate::policy::session::SessionSchedulePlan,
        allow_wake: bool,
    ) -> Result<&'static str, SessionFlowError> {
        let interval_ms = schedule.monitor_interval_ms.max(200);
        let started_at_ms = now_ms();
        let mut last_wake_attempt_at_ms = (!allow_wake).then_some(started_at_ms);
        let mut transient_wake_failure_count = 0u8;

        loop {
            let consoles = self.inner.provider.get_remote_consoles().await?;
            let matched = consoles
                .iter()
                .find(|console| matches_remote_console_id(target_id, console));

            if let Some(console) = matched {
                let now_ms = now_ms();
                if let Some(reason) = remote_console_ready_reason(console) {
                    self.inner.provider.on_console_ready_wait_result(
                        target_type,
                        target_id,
                        "succeeded",
                        reason,
                        Some(console),
                    );
                    return Ok(reason);
                }

                let power_state = console.power_state.as_deref();
                if allow_wake
                    && should_retry_wake_during_ready_wait(
                        power_state,
                        last_wake_attempt_at_ms,
                        now_ms,
                    )
                {
                    match self.inner.provider.power_on_console(target_id).await {
                        Ok(accepted) => {
                            log::info!(
                                "home ready wait issued follow-up wake command: target_id={} power_state={:?} accepted={}",
                                target_id,
                                power_state,
                                accepted,
                            );
                            last_wake_attempt_at_ms = Some(now_ms);
                            if accepted {
                                transient_wake_failure_count = 0;
                            }
                        }
                        Err(error)
                            if is_waiting_for_server_registration_message(&error.message) =>
                        {
                            log::warn!(
                                "home ready wait wake command is still waiting for server registration: target_id={} power_state={:?} error={}",
                                target_id,
                                power_state,
                                error,
                            );
                            last_wake_attempt_at_ms = Some(now_ms);
                            transient_wake_failure_count =
                                transient_wake_failure_count.saturating_add(1);
                            let elapsed_ms = now_ms.saturating_sub(started_at_ms);
                            if transient_wake_failure_count >= 3
                                && elapsed_ms >= 15_000
                                && matches!(
                                    power_state,
                                    Some("ConnectedStandby") | Some("Off") | None
                                )
                            {
                                return Err(remote_console_wake_circuit_open_error(
                                    target_id,
                                    power_state,
                                    transient_wake_failure_count,
                                ));
                            }
                        }
                        Err(error) => return Err(error),
                    }
                }
            }

            let elapsed_ms = now_ms().saturating_sub(started_at_ms);
            if elapsed_ms >= schedule.ready_timeout_ms {
                self.inner.provider.on_console_ready_wait_result(
                    target_type,
                    target_id,
                    "failed",
                    "timeout",
                    matched,
                );
                return Err(remote_console_not_ready_error(target_id));
            }

            tokio::time::sleep(std::time::Duration::from_millis(interval_ms)).await;
        }
    }

    async fn start_stream_with_retry(
        &self,
        api: &WebApiSessionGateway,
        plan: &Plan,
        scope: &SessionStartupScope,
    ) -> Result<String, SessionFlowError> {
        let retry_backoff_ms = &plan.session.schedule.retry_backoff_ms;
        let retry_timeout_ms = plan
            .session
            .schedule
            .ready_timeout_ms
            .max(plan.session.schedule.monitor_interval_ms.max(200));
        let retry_started_at_ms = now_ms();
        let mut attempt = 0usize;

        loop {
            if scope.is_cancelled() {
                return Err(cancelled_error("creatingSession"));
            }
            match api.start_stream().await {
                Ok(session_path) => return Ok(session_path),
                Err(error) => {
                    let elapsed_ms = now_ms().saturating_sub(retry_started_at_ms);
                    let should_retry = should_retry_home_server_registration(
                        plan,
                        &error,
                        elapsed_ms,
                        retry_timeout_ms,
                    );
                    if !should_retry {
                        if matches!(
                            &error,
                            xbox_webapi::WebApiError::Network { .. }
                                | xbox_webapi::WebApiError::Parse { .. }
                        ) {
                            self.record_unknown_create(plan, scope);
                        }
                        return Err(map_webapi_error(error));
                    }

                    let backoff_ms = next_retry_backoff_ms(retry_backoff_ms, attempt);
                    let error_message = error.to_string();
                    log::warn!(
                        "home start_stream waiting for server registration: target_id={} attempt={} elapsed_ms={} backoff_ms={} error={}",
                        plan.session.target_id,
                        attempt + 1,
                        elapsed_ms,
                        backoff_ms,
                        error_message,
                    );

                    // xHome 主机可能已经开机，但串流服务尚未重新注册；
                    // 在重试前补一次 ready 轮询，避免直接对 XCCS 打无效重试风暴。
                    if let Err(wait_error) = scope
                        .run(
                            "registrationPrecheck",
                            Some(std::time::Duration::from_millis(
                                retry_timeout_ms.saturating_sub(elapsed_ms),
                            )),
                            self.wait_until_console_ready(
                                plan.session.target.as_str(),
                                &plan.session.target_id,
                                &plan.session.schedule,
                                plan.session.schedule.wake_console,
                            ),
                        )
                        .await
                    {
                        log::warn!(
                            "home console ready precheck did not pass before start_stream retry: target_id={} error={}",
                            plan.session.target_id,
                            wait_error,
                        );
                    }
                    scope
                        .run(
                            "registrationBackoff",
                            Some(std::time::Duration::from_millis(
                                retry_timeout_ms
                                    .saturating_sub(now_ms().saturating_sub(retry_started_at_ms)),
                            )),
                            async {
                                tokio::time::sleep(std::time::Duration::from_millis(backoff_ms))
                                    .await;
                                Ok(())
                            },
                        )
                        .await?;
                    attempt += 1;
                }
            }
        }
    }

    fn record_unknown_create(&self, plan: &Plan, scope: &SessionStartupScope) {
        self.inner
            .startups
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .record_unknown_create(
                plan.session.target.as_str(),
                &plan.session.target_id,
                scope.generation(),
            );
        self.inner.provider.on_session_lifecycle_event(
            "createResultUnknown",
            None,
            serde_json::json!({
                "startupGeneration": scope.generation(), "targetType": plan.session.target.as_str(),
                "stage": "creatingSession", "result": "unknown", "retryBlocked": true,
            }),
        );
    }

    async fn wait_until_session_started_or_failed(
        &self,
        session_id: &str,
        schedule: &crate::policy::session::SessionSchedulePlan,
    ) -> Result<SessionProgressSnapshot, SessionFlowError> {
        let interval_ms = schedule.monitor_interval_ms.max(200);
        let grace_ms = startup_closed_recovery_grace_ms(interval_ms);
        let mut budget = PreparationBudget::new(now_ms());
        let mut last_recovery_signal_at_ms = now_ms();
        let mut registration_started_at_ms = None;
        let identity = self
            .get_session_record(session_id)
            .await
            .ok_or_else(|| missing_session_error(session_id))?
            .cancelled;
        identity
            .run("sessionReady", None, async {
                loop {
                    let record = self
                        .get_session_record(session_id)
                        .await
                        .ok_or_else(|| missing_session_error(session_id))?;
                    if !record.cancelled.same_instance(&identity) {
                        return Err(missing_session_error(session_id));
                    }
                    let runtime = record.snapshot.runtime_snapshot();
                    let is_home = record.plan.session.target.is_home();
                    let now = now_ms();
                    let queued =
                        !is_home && runtime.stream_state.as_deref() == Some("WaitingForResources");
                    if queued && now.saturating_sub(record.metadata.last_observed_at_ms) >= 45_000 {
                        return Err(SessionFlowError::message(
                            "operationTimeout:stage=queueObservation",
                        ));
                    }
                    if let Some(error) = evaluate_home_server_registration_wait_timeout(
                        is_home,
                        &record.plan.session.target_id,
                        session_id,
                        &runtime,
                        now,
                        &mut registration_started_at_ms,
                    ) {
                        return Err(error);
                    }
                    let progress = build_session_progress_snapshot(record);
                    match decide_startup_progress_action(
                        &progress,
                        now,
                        &mut last_recovery_signal_at_ms,
                        grace_ms,
                    ) {
                        StartupProgressAction::Ready => return Ok(progress),
                        StartupProgressAction::Fail(message) => {
                            return Err(SessionFlowError::message(message))
                        }
                        StartupProgressAction::Continue { .. } => {}
                    }
                    if budget.observe(
                        now,
                        queued,
                        schedule.startup_timeout_ms.saturating_add(interval_ms),
                    ) {
                        return Err(startup_timeout_error(session_id));
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(interval_ms)).await;
                }
            })
            .await
    }

    async fn get_session_ready_retry_probe(&self, session_id: &str) -> SessionReadyRetryProbe {
        let Some(record) = self.get_session_record(session_id).await else {
            return SessionReadyRetryProbe::default();
        };
        let runtime = record.snapshot.runtime_snapshot();
        let progress = build_session_progress_snapshot(record);

        SessionReadyRetryProbe {
            phase: Some(progress.phase),
            stream_state: runtime.stream_state,
            error_code: progress.error_code,
            error_message: progress.error_message,
        }
    }
}
