use std::sync::Arc;
use crate::session::flow::{SessionFlowSnapshot, SessionFlowProvider, map_webapi_error, SessionFlowError, build_session_progress_snapshot, SessionFlowServiceInner};
use crate::session::store::{SessionCancelToken, SessionRuntimeRecord};
use crate::session::monitor::{apply_monitor_tick_to_record, SessionMonitorInput};
use crate::session::api::session::WebApiSessionGateway;
use crate::policy::Plan;

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
            if cancelled.is_cancelled() {
                return;
            }

            let should_continue = self.monitor_session_tick(&session_id).await;
            if !should_continue {
                return;
            }

            tokio::time::sleep(std::time::Duration::from_millis(monitor_interval_ms)).await;
        }
    }

    async fn keepalive_session_loop(
        &self,
        session_id: String,
        cancelled: SessionCancelToken,
        keepalive_interval_ms: u64,
    ) {
        loop {
            if cancelled.is_cancelled() {
                return;
            }

            tokio::time::sleep(std::time::Duration::from_millis(keepalive_interval_ms)).await;

            if cancelled.is_cancelled() {
                return;
            }

            let progress = self.get_session_progress(&session_id).await;
            let Some(progress) = progress else {
                return;
            };
            
            use crate::session::flow::SessionPhase;
            if progress.phase != SessionPhase::SessionReady
                && progress.phase != SessionPhase::Recovering
            {
                continue;
            }

            match self.send_keepalive(&session_id).await {
                Ok(true) => {}
                Ok(false) => {
                    if self.get_session_record(&session_id).await.is_none() {
                        return;
                    }
                }
                Err(_) => {
                    // keepalive 失败不应中断循环，等待下一轮再尝试。
                }
            }
        }
    }

    async fn monitor_session_tick(&self, session_id: &str) -> bool {
        let mut record = match self.get_session_record(session_id).await {
            Some(record) => record,
            None => return false,
        };
        
        let plan = record.plan.clone();
        let api = match self.create_session_api(&plan).await {
            Ok(api) => api,
            Err(_) => return true,
        };

        let state_response = api.get_stream_state(session_id).await;
        let (state, error_details) = match state_response {
            Ok(value) => value,
            Err(error) => {
                let flow_error = map_webapi_error(error);
                if flow_error.status == Some(404) {
                    self.clear_session(session_id).await;
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
            stream_state: state,
            error_details,
            waiting_queue,
        };
        let monitor_control = apply_monitor_tick_to_record(&mut record, monitor_input);
        self.upsert_session(session_id, record).await;

        if monitor_control.should_send_connect_token {
            let transfer_token = match self.inner.provider.transfer_token().await {
                Ok(token) => token,
                Err(_) => return true,
            };

            let _ = api.send_connect_token(session_id, &transfer_token).await;
        }

        monitor_control.should_continue
    }

    pub(crate) async fn send_keepalive(&self, session_id: &str) -> Result<bool, SessionFlowError> {
        let record = self.get_session_record(session_id).await;
        let Some(record) = record else {
            return Ok(false);
        };

        let api = self.create_session_api(&record.plan).await?;
        let result = api.send_keepalive(session_id).await;
        match result {
            Ok(()) => Ok(true),
            Err(error) => {
                let flow_error = map_webapi_error(error);
                use crate::session::signaling::logic::should_ignore_keepalive_error;
                if should_ignore_keepalive_error(flow_error.status, flow_error.body.as_deref()) {
                    Ok(false)
                } else {
                    Err(flow_error)
                }
            }
        }
    }

    async fn get_session_record(&self, session_id: &str) -> Option<SessionRuntimeRecord<S>> {
        let sessions = self.inner.sessions.read().await;
        sessions.get(session_id)
    }

    async fn upsert_session(&self, session_id: &str, record: SessionRuntimeRecord<S>) {
        self.inner.sessions
            .write()
            .await
            .upsert(session_id.to_string(), record);
    }

    async fn clear_session(&self, session_id: &str) {
        let _ = self.inner.sessions.write().await.remove(session_id);
    }

    async fn get_session_progress(&self, session_id: &str) -> Option<crate::session::flow::SessionProgressSnapshot> {
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

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}
