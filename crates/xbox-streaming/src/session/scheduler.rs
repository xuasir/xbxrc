use crate::policy::Plan;
use crate::session::api::session::WebApiSessionGateway;
use crate::session::flow::{
    build_session_progress_snapshot, map_webapi_error, SessionFlowError, SessionFlowProvider,
    SessionFlowServiceInner, SessionFlowSnapshot, SessionPhase,
};
use crate::session::monitor::{apply_monitor_tick_to_record, SessionMonitorInput};
use crate::session::operation::now_ms;
use crate::session::store::{SessionCancelToken, SessionRuntimeRecord};
use std::sync::Arc;

pub(crate) struct SessionScheduler<S, P>
where
    S: SessionFlowSnapshot,
    P: SessionFlowProvider,
{
    inner: Arc<SessionFlowServiceInner<S, P>>,
}

impl<S, P> SessionScheduler<S, P>
where
    S: SessionFlowSnapshot,
    P: SessionFlowProvider,
{
    pub(crate) fn new(inner: Arc<SessionFlowServiceInner<S, P>>) -> Self {
        Self { inner }
    }

    pub(crate) async fn start_loops(
        &self,
        session_id: String,
        cancelled: SessionCancelToken,
        monitor_interval_ms: u64,
        keepalive_interval_ms: u64,
    ) {
        let scheduler = self.clone();
        let monitor_cancelled = cancelled.clone();
        let monitor_session_id = session_id.clone();
        tokio::spawn(async move {
            scheduler
                .monitor_session_loop(monitor_session_id, monitor_cancelled, monitor_interval_ms)
                .await;
        });

        let scheduler = self.clone();
        let keepalive_session_id = session_id;
        tokio::spawn(async move {
            scheduler
                .keepalive_session_loop(keepalive_session_id, cancelled, keepalive_interval_ms)
                .await;
        });
    }

    async fn monitor_session_loop(
        &self,
        session_id: String,
        cancelled: SessionCancelToken,
        monitor_interval_ms: u64,
    ) {
        loop {
            let tick = cancelled
                .run(
                    "sessionObservation",
                    Some(std::time::Duration::from_secs(45)),
                    async { Ok(self.monitor_session_tick(&session_id).await) },
                )
                .await;
            if matches!(tick, Ok(false)) || cancelled.is_cancelled() {
                return;
            }
            tokio::select! {
                biased;
                _ = cancelled.cancelled() => return,
                _ = tokio::time::sleep(std::time::Duration::from_millis(monitor_interval_ms)) => {}
            }
        }
    }

    async fn keepalive_session_loop(
        &self,
        session_id: String,
        cancelled: SessionCancelToken,
        keepalive_interval_ms: u64,
    ) {
        loop {
            tokio::select! {
                biased;
                _ = cancelled.cancelled() => return,
                _ = tokio::time::sleep(std::time::Duration::from_millis(keepalive_interval_ms)) => {}
            }
            let Some(progress) = self.get_session_progress(&session_id).await else {
                return;
            };
            if matches!(progress.phase, SessionPhase::Failed | SessionPhase::Closed) {
                return;
            }
            if matches!(
                progress.phase,
                SessionPhase::SessionReady | SessionPhase::Recovering
            ) {
                let _ = self.send_keepalive(&session_id).await;
            }
        }
    }

    async fn monitor_session_tick(&self, session_id: &str) -> bool {
        let mut record = match self.get_session_record(session_id).await {
            Some(record) => record,
            None => return false,
        };

        let plan = record.plan.clone();
        let target_type = plan.session.target.as_str().to_string();
        let target_id = plan.session.target_id.clone();
        let api = match self.create_session_api(&plan).await {
            Ok(api) => api,
            Err(_) => return true,
        };

        let state_response = api.get_stream_state(session_id).await;
        let (state, error_details) = match state_response {
            Ok(value) => {
                self.inner.provider.on_session_state_polled(
                    session_id,
                    &target_type,
                    &target_id,
                    value.0.as_deref(),
                    value.1.as_ref().and_then(|details| details.code.as_ref()),
                    value
                        .1
                        .as_ref()
                        .and_then(|details| details.message.as_deref()),
                );
                value
            }
            Err(error) => {
                let flow_error = map_webapi_error(error);
                self.inner.provider.on_session_state_poll_failed(
                    session_id,
                    &target_type,
                    &target_id,
                    &flow_error,
                );
                if matches!(flow_error.status, Some(401 | 404)) {
                    self.record_terminal_error(session_id, record, flow_error)
                        .await;
                    return false;
                }
                return true;
            }
        };

        let waiting_queue = if state.as_deref() == Some("WaitingForResources") {
            match api.get_waiting_times().await {
                Ok(queue) => Some(queue),
                Err(_) => {
                    let runtime = record.snapshot.runtime_snapshot();
                    Some(runtime.queue.map(|queue| queue.details).unwrap_or_default())
                }
            }
        } else {
            None
        };

        let monitor_input = SessionMonitorInput {
            now_ms: now_ms(),
            state_timeout_ms: plan.session.schedule.startup_timeout_ms,
            stream_state: state.clone(),
            error_details,
            waiting_queue,
        };
        // 播放期状态回退：已经出图的会话被服务端退回 Provisioning / ReadyToConnect。
        // monitor 侧的门槛已经不再判失败（媒体可能仍在正常出图），这里只补一条
        // 状态迁移日志，避免线上再出现「报错但画面在动」却查不到状态轨迹。
        let playback_regression_stream_state =
            matches!(state.as_deref(), Some("Provisioning" | "ReadyToConnect"))
                .then(|| state.clone().unwrap_or_default());
        let monitor_control = apply_monitor_tick_to_record(&mut record, monitor_input);
        if let Some(regressed_state) = playback_regression_stream_state {
            if record.metadata.repeated_state_count == 1
                && record.metadata.reached_provisioned
                && record.snapshot.runtime_snapshot().player_state == "started"
            {
                log::warn!(
                    "session regressed after provisioned: session_id={side} target_id={} stream_state={} monitor_attempt_count={}",
                    plan.session.target_id,
                    regressed_state,
                    record.metadata.monitor_attempt_count,
                    side = session_id,
                );
            }
        }
        let progress = build_session_progress_snapshot(record.clone());
        let runtime = record.snapshot.runtime_snapshot();
        self.inner.provider.on_session_monitor_tick(
            session_id,
            &target_type,
            &target_id,
            &progress,
            runtime.stream_state.as_deref(),
            &runtime.player_state,
            monitor_control.should_continue,
            monitor_control.should_send_connect_token,
        );

        let Some(mutation) = self.session_mutation(session_id, &record.cancelled).await else {
            return false;
        };
        let state = mutation.state.lock().await;
        if state.closed || record.cancelled.is_cancelled() {
            return false;
        }
        let cancelled = record.cancelled.clone();
        if !self.upsert_session(session_id, record).await {
            return false;
        }
        let send_token = monitor_control.should_send_connect_token && !state.connect_token_sent;
        drop(state);
        if send_token {
            let result = cancelled
                .run(
                    "connectToken",
                    Some(std::time::Duration::from_secs(15)),
                    async {
                        let transfer_token = self.inner.provider.transfer_token().await?;
                        if cancelled.is_cancelled() {
                            return Err(crate::session::operation::cancelled_error("connectToken"));
                        }
                        api.send_connect_token(session_id, &transfer_token)
                            .await
                            .map_err(map_webapi_error)
                    },
                )
                .await;
            if cancelled.is_cancelled() {
                return false;
            }
            if result.is_ok() {
                mutation.state.lock().await.connect_token_sent = true;
            }
            self.inner.provider.on_session_connect_token_result(
                session_id,
                &target_type,
                &target_id,
                if result.is_ok() { "sent" } else { "sendFailed" },
                result.as_ref().err(),
            );
        }

        if matches!(progress.phase, SessionPhase::Failed | SessionPhase::Closed) {
            cancelled.cancel();
        }
        should_continue_monitor_loop(progress.phase, monitor_control.should_continue)
    }

    pub(crate) async fn send_keepalive(&self, session_id: &str) -> Result<bool, SessionFlowError> {
        let record = self.get_session_record(session_id).await;
        let Some(record) = record else {
            return Ok(false);
        };

        record
            .cancelled
            .run(
                "keepalive",
                Some(std::time::Duration::from_secs(15)),
                async {
                    let api = self.create_session_api(&record.plan).await?;
                    if record.cancelled.is_cancelled() {
                        return Ok(false);
                    }
                    match api.send_keepalive(session_id).await {
                        Ok(()) => Ok(true),
                        Err(error) => {
                            let error = map_webapi_error(error);
                            if matches!(error.status, Some(401 | 404)) {
                                self.record_terminal_error(session_id, record.clone(), error)
                                    .await;
                                return Ok(false);
                            }
                            use crate::session::signaling::logic::should_ignore_keepalive_error;
                            if should_ignore_keepalive_error(error.status, error.body.as_deref()) {
                                Ok(false)
                            } else {
                                Err(error)
                            }
                        }
                    }
                },
            )
            .await
    }

    async fn record_terminal_error(
        &self,
        session_id: &str,
        mut record: SessionRuntimeRecord<S>,
        error: SessionFlowError,
    ) {
        let mut runtime = record.snapshot.runtime_snapshot();
        runtime.player_state = if error.status == Some(404) {
            "closed"
        } else {
            "failed"
        }
        .to_owned();
        runtime.stream_state = Some(
            if error.status == Some(404) {
                "Closed"
            } else {
                "Failed"
            }
            .to_owned(),
        );
        runtime.error_details = Some(crate::session::monitor::SessionErrorDetails {
            code: error.status.map(|status| serde_json::json!(status)),
            message: Some(error.message),
        });
        record.snapshot.replace_runtime_snapshot(runtime);
        let cancelled = record.cancelled.clone();
        if self.upsert_session(session_id, record).await {
            cancelled.cancel();
        }
    }

    async fn get_session_record(&self, session_id: &str) -> Option<SessionRuntimeRecord<S>> {
        let sessions = self.inner.sessions.read().await;
        sessions.get(session_id)
    }

    async fn session_mutation(
        &self,
        session_id: &str,
        cancelled: &crate::session::store::SessionCancelToken,
    ) -> Option<Arc<crate::session::flow::SessionMutationEntry>> {
        self.inner
            .mutations
            .read()
            .await
            .get(session_id)
            .cloned()
            .filter(|mutation| mutation.cancelled.same_instance(cancelled))
    }

    async fn upsert_session(&self, session_id: &str, record: SessionRuntimeRecord<S>) -> bool {
        self.inner
            .sessions
            .write()
            .await
            .upsert_if_same(session_id.to_string(), record)
    }

    async fn get_session_progress(
        &self,
        session_id: &str,
    ) -> Option<crate::session::flow::SessionProgressSnapshot> {
        self.get_session_record(session_id)
            .await
            .map(build_session_progress_snapshot)
    }

    async fn create_session_api(
        &self,
        plan: &Plan,
    ) -> Result<WebApiSessionGateway, SessionFlowError> {
        use crate::session::access::StreamingToken;
        let target_type = plan.session.target.as_str();
        let token_value = self.inner.provider.get_streaming_token(target_type).await?;
        let token = StreamingToken::parse(&token_value)
            .map_err(|e| SessionFlowError::message(e.to_string()))?;

        Ok(WebApiSessionGateway::new(plan.clone(), token))
    }
}

impl<S, P> Clone for SessionScheduler<S, P>
where
    S: SessionFlowSnapshot,
    P: SessionFlowProvider,
{
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

fn should_continue_monitor_loop(phase: SessionPhase, state_should_continue: bool) -> bool {
    match phase {
        SessionPhase::Failed | SessionPhase::Closed => false,
        SessionPhase::SessionReady => true,
        _ => state_should_continue,
    }
}

#[cfg(test)]
mod tests {
    use super::should_continue_monitor_loop;
    use crate::session::flow::SessionPhase;

    #[test]
    fn monitor_remains_active_after_session_ready() {
        assert!(should_continue_monitor_loop(
            SessionPhase::SessionReady,
            false
        ));
    }

    #[test]
    fn monitor_stops_for_remote_terminal_phases() {
        assert!(!should_continue_monitor_loop(SessionPhase::Failed, false));
        assert!(!should_continue_monitor_loop(SessionPhase::Closed, true));
    }
}
