use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use std::sync::Arc;

use crate::policy::Plan;
use crate::session::access::StreamingToken;
use crate::session::api::session::WebApiSessionGateway;
use crate::session::api::signaling::{AnswerPayload, WebApiSignalingGateway};
use crate::session::lifecycle::{
    is_remote_session_not_found, parse_session_id_from_path, resolve_active_target_type,
};
use crate::session::monitor::SessionRuntimeBinding;
use crate::session::scheduler::SessionScheduler;
use crate::session::signaling::ice::IceCandidate;
use crate::session::signaling::logic::{decide_ice_poll, decide_offer_poll, PollDecision};
use crate::session::store::{SessionRuntimeRecord, SessionRuntimeStore};

/// session flow 的统一错误，便于 adapter 只做一次映射。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionFlowError {
    pub message: String,
    pub status: Option<u16>,
    pub body: Option<String>,
}

impl SessionFlowError {
    pub fn message(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            status: None,
            body: None,
        }
    }

    pub fn http(status: u16, message: impl Into<String>, body: Option<String>) -> Self {
        Self {
            message: message.into(),
            status: Some(status),
            body,
        }
    }
}

impl Display for SessionFlowError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for SessionFlowError {}

/// session 快照最小契约：flow 只关心标识、路径与 runtime 绑定。
/// RFC: Snapshot 必须是可安全返回 UI 的状态对象，严禁持有敏感凭证。
pub trait SessionFlowSnapshot: SessionRuntimeBinding + Clone + Send + Sync + 'static {
    fn new_pending(
        session_id: String,
        session_path: String,
        target_id: String,
        target_type: String,
    ) -> Self;
    fn session_id(&self) -> &str;
    fn session_path(&self) -> &str;
    fn target_id(&self) -> &str;
    fn target_type(&self) -> &str;
}

/// list_active_sessions 的标准输出，包含默认 targetType 使用标记。
#[derive(Debug, Clone)]
pub struct ListActiveSessions<S: Clone> {
    pub target_type: String,
    pub used_default_target_type: bool,
    pub sessions: Vec<S>,
}

/// session 对外执行快照：远端会话状态 + 本地执行层投影。
#[derive(Debug, Clone)]
pub struct SessionExecutionSnapshot<S: Clone, R: Clone, E: Clone> {
    pub session: S,
    pub runtime: R,
    pub render: E,
}

/// session 进度阶段：用于给 UI/adapter 提供稳定、可序列化的调度观察面。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SessionPhase {
    Creating,
    WaitingSessionReady,
    RuntimeStarting,
    SessionReady,
    Recovering,
    Closing,
    Closed,
    Failed,
}

/// 会话进度快照：不泄漏 plan/token，仅表达当前阶段和展示所需信息。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionProgressSnapshot {
    pub session_id: String,
    pub phase: SessionPhase,
    pub status_text_key: String,
    pub retry_count: u8,
    pub queue_seconds: Option<u64>,
    pub queue: Option<crate::session::monitor::QueueDetails>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
}

/// 远端主机快照：仅保留 session 预检所需字段。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct RemoteConsoleSnapshot {
    pub id: Option<String>,
    pub device_id: Option<String>,
    pub server_id: Option<String>,
    pub power_state: Option<String>,
    pub console_streaming_enabled: Option<bool>,
}

/// session flow 外部依赖：仅负责提供凭证。
/// RFC: 策略与执行细节已收口在 crate，Provider 仅提供运行事实。
#[async_trait]
pub trait SessionFlowProvider: Send + Sync + 'static {
    async fn get_streaming_token(&self, target_type: &str) -> Result<Value, SessionFlowError>;
    async fn transfer_token(&self) -> Result<String, SessionFlowError>;
    async fn power_on_console(&self, _console_id: &str) -> Result<bool, SessionFlowError> {
        Ok(false)
    }
    async fn get_remote_consoles(&self) -> Result<Vec<RemoteConsoleSnapshot>, SessionFlowError> {
        Ok(Vec::new())
    }
}

#[derive(Clone)]
pub struct SessionFlowService<S, P>
where
    S: SessionFlowSnapshot,
    P: SessionFlowProvider,
{
    pub(crate) inner: Arc<SessionFlowServiceInner<S, P>>,
}

pub(crate) struct SessionFlowServiceInner<S, P>
where
    S: SessionFlowSnapshot,
    P: SessionFlowProvider,
{
    pub(crate) provider: P,
    pub(crate) sessions: tokio::sync::RwLock<SessionRuntimeStore<S>>,
    pub(crate) signaling: tokio::sync::RwLock<HashMap<String, SessionSignalingState>>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct SessionSignalingState {
    pub last_polled_ice: Option<Vec<IceCandidate>>,
    pub restart_baseline_ice: Option<Vec<IceCandidate>>,
}

impl<S, P> SessionFlowService<S, P>
where
    S: SessionFlowSnapshot,
    P: SessionFlowProvider,
{
    pub fn new(provider: P) -> Self {
        Self {
            inner: Arc::new(SessionFlowServiceInner {
                provider,
                sessions: tokio::sync::RwLock::new(SessionRuntimeStore::default()),
                signaling: tokio::sync::RwLock::new(HashMap::new()),
            }),
        }
    }

    pub async fn create_session(&self, plan: Plan) -> Result<S, SessionFlowError> {
        let monitor_interval_ms = plan.session.schedule.monitor_interval_ms.max(200);
        let keepalive_interval_ms = plan.session.schedule.keepalive_interval_ms.max(1_000);
        let api = self.create_session_api(&plan).await?;
        let session_path = api.start_stream().await.map_err(map_webapi_error)?;

        let session_id = parse_session_id_from_path(&session_path)
            .map_err(|error| SessionFlowError::message(error.to_string()))?;

        let target_type = plan.session.target.as_str();
        let snapshot = S::new_pending(
            session_id.clone(),
            session_path,
            plan.session.target_id.clone(),
            target_type.to_string(),
        );
        let cancelled = {
            let mut sessions = self.inner.sessions.write().await;
            sessions.insert_new_with_plan(session_id.clone(), snapshot.clone(), plan, now_ms())
        };

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
        let runtime = project_runtime(&plan);
        let render = project_render(&plan);
        let schedule = plan.session.schedule.clone();

        self.prepare_remote_console(&plan).await?;
        let session = self.create_session(plan).await?;
        let session_id = session.session_id().to_string();
        self.wait_until_session_started_or_failed(&session_id, &schedule)
            .await?;

        let started_session = self
            .get_session(&session_id)
            .await
            .ok_or_else(|| missing_session_error(&session_id))?;

        Ok(SessionExecutionSnapshot {
            session: started_session,
            runtime,
            render,
        })
    }

    pub async fn get_session(&self, session_id: &str) -> Option<S> {
        self.get_session_record(session_id)
            .await
            .map(|record| record.snapshot)
    }

    pub async fn get_session_execution<R, E, FR, FE>(
        &self,
        session_id: &str,
        project_runtime: FR,
        project_render: FE,
    ) -> Option<SessionExecutionSnapshot<S, R, E>>
    where
        R: Clone,
        E: Clone,
        FR: Fn(&Plan) -> R,
        FE: Fn(&Plan) -> E,
    {
        self.get_session_record(session_id)
            .await
            .map(|record| SessionExecutionSnapshot {
                runtime: project_runtime(&record.plan),
                render: project_render(&record.plan),
                session: record.snapshot,
            })
    }

    /// 对外暴露稳定进度快照，供 adapter/UI 做状态展示。
    pub async fn get_session_progress(&self, session_id: &str) -> Option<SessionProgressSnapshot> {
        self.get_session_record(session_id)
            .await
            .map(build_session_progress_snapshot)
    }

    pub async fn close_session(&self, session_id: &str) -> Result<bool, SessionFlowError> {
        let record = self.get_session_record(session_id).await;
        let Some(record) = record else {
            return Ok(false);
        };

        let api = self.create_session_api(&record.plan).await?;
        let result = api.stop_stream(session_id).await;
        self.clear_session(session_id).await;

        match result {
            Ok(()) => Ok(true),
            Err(error) => {
                let flow_error = map_webapi_error(error);
                if is_remote_session_not_found(flow_error.status) {
                    Ok(false)
                } else {
                    Err(flow_error)
                }
            }
        }
    }

    pub async fn exchange_offer(
        &self,
        session_id: &str,
        channel: Option<&str>,
        sdp: &str,
        restart: bool,
    ) -> Result<AnswerPayload, SessionFlowError> {
        let record = self.get_session_record(session_id).await;
        let Some(record) = record else {
            return Err(missing_session_error(session_id));
        };
        let plan = &record.plan;
        let poll_interval_ms = plan.session.schedule.offer_poll_interval_ms.max(100);
        let api = self.create_signaling_api(plan).await?;
        let previous_answer = if restart {
            api.get_sdp_exchange_response(session_id)
                .await
                .map_err(map_webapi_error)?
        } else {
            None
        };

        if channel == Some("chat") {
            api.send_chat_sdp(session_id, sdp)
                .await
                .map_err(map_webapi_error)?;
        } else {
            api.send_sdp(session_id, sdp)
                .await
                .map_err(map_webapi_error)?;
        }

        loop {
            let answer = api
                .get_sdp_exchange_response(session_id)
                .await
                .map_err(map_webapi_error)?;
            let answer = filter_stale_offer_response(answer, previous_answer.as_ref(), restart);
            let session_exists = self.get_session_record(session_id).await.is_some();
            let decision = decide_offer_poll(answer.is_some(), session_exists);

            match (decision, answer) {
                (PollDecision::Completed, Some(answer)) => return Ok(answer),
                (PollDecision::SessionMissing, _) => {
                    return Err(missing_session_error(session_id));
                }
                (PollDecision::Continue, _) => {
                    tokio::time::sleep(std::time::Duration::from_millis(poll_interval_ms)).await;
                }
                (PollDecision::Completed, None) => {}
            }
        }
    }

    pub async fn submit_ice(
        &self,
        session_id: &str,
        candidates: &[IceCandidate],
        restart: bool,
    ) -> Result<(), SessionFlowError> {
        let record = self.get_session_record(session_id).await;
        let Some(record) = record else {
            return Err(missing_session_error(session_id));
        };
        let plan = &record.plan;
        let api = self.create_signaling_api(plan).await?;
        let restart_baseline = if restart {
            api.get_ice_exchange_response(session_id)
                .await
                .map_err(map_webapi_error)?
        } else {
            None
        };

        api.send_ice(session_id, candidates)
            .await
            .map_err(map_webapi_error)?;
        let mut signaling = self.inner.signaling.write().await;
        let state = signaling.entry(session_id.to_string()).or_default();
        if restart {
            state.restart_baseline_ice = restart_baseline;
            state.last_polled_ice = None;
        }
        Ok(())
    }

    pub async fn poll_ice(
        &self,
        session_id: &str,
        restart: bool,
    ) -> Result<Vec<IceCandidate>, SessionFlowError> {
        let record = self.get_session_record(session_id).await;
        let Some(record) = record else {
            return Err(missing_session_error(session_id));
        };
        let plan = &record.plan;
        let poll_interval_ms = plan.session.schedule.ice_poll_interval_ms.max(100);
        let api = self.create_signaling_api(plan).await?;
        loop {
            let response = api
                .get_ice_exchange_response(session_id)
                .await
                .map_err(map_webapi_error)?;
            let response = self
                .filter_polled_ice_response(session_id, response, restart)
                .await;
            let session_exists = self.get_session_record(session_id).await.is_some();
            let decision = decide_ice_poll(response.as_deref(), session_exists);

            match (decision, response) {
                (PollDecision::Completed, Some(candidates)) => return Ok(candidates),
                (PollDecision::SessionMissing, _) => {
                    return Err(missing_session_error(session_id));
                }
                (PollDecision::Continue, _) => {
                    tokio::time::sleep(std::time::Duration::from_millis(poll_interval_ms)).await;
                }
                (PollDecision::Completed, None) => {}
            }
        }
    }

    pub async fn send_keepalive(&self, session_id: &str) -> Result<bool, SessionFlowError> {
        let scheduler = SessionScheduler::new(Arc::clone(&self.inner));
        scheduler.send_keepalive(session_id).await
    }

    pub async fn list_active_sessions(&self, target_type: Option<String>) -> ListActiveSessions<S> {
        let resolved = resolve_active_target_type(target_type);
        let sessions = self
            .inner
            .sessions
            .read()
            .await
            .list_snapshots(|session| session.target_type() == resolved.value);

        ListActiveSessions {
            target_type: resolved.value,
            used_default_target_type: resolved.used_default,
            sessions,
        }
    }

    /// 进程退出前 best-effort 关闭会话，避免服务端残留活跃会话。
    pub async fn shutdown(&self) {
        let session_ids = {
            let sessions = self.inner.sessions.read().await;
            sessions.keys()
        };

        for session_id in session_ids {
            let _ = self.close_session(&session_id).await;
        }
    }

    async fn prepare_remote_console(&self, plan: &Plan) -> Result<(), SessionFlowError> {
        if !plan.session.target.is_home() || !plan.session.schedule.wake_console {
            return Ok(());
        }
        let wake_accepted = self
            .inner
            .provider
            .power_on_console(&plan.session.target_id)
            .await?;
        if !wake_accepted || !plan.session.schedule.require_console_ready {
            return Ok(());
        }

        self.wait_until_console_ready(&plan.session.target_id, &plan.session.schedule)
            .await
    }

    async fn wait_until_console_ready(
        &self,
        target_id: &str,
        schedule: &crate::policy::session::SessionSchedulePlan,
    ) -> Result<(), SessionFlowError> {
        let interval_ms = schedule.monitor_interval_ms.max(200);

        wait_until(
            interval_ms,
            schedule.ready_timeout_ms,
            || async {
                let consoles = self.inner.provider.get_remote_consoles().await?;
                let matched = consoles
                    .iter()
                    .find(|console| matches_remote_console_id(target_id, console));
                if let Some(console) = matched {
                    if is_remote_console_ready(console) {
                        return Ok(Some(()));
                    }
                }
                Ok(None)
            },
            || SessionFlowError::message(format!("remoteConsoleNotReady:targetId={target_id}")),
        )
        .await
    }

    async fn wait_until_session_started_or_failed(
        &self,
        session_id: &str,
        schedule: &crate::policy::session::SessionSchedulePlan,
    ) -> Result<SessionProgressSnapshot, SessionFlowError> {
        let interval_ms = schedule.monitor_interval_ms.max(200);
        // 给 monitor 一次额外 tick，把精确踩线的“卡住”状态先收敛成 failed，
        // 避免这里抢先抛出通用 timeout，丢掉更具体的上下文。
        let timeout_with_grace_ms = schedule.startup_timeout_ms.saturating_add(interval_ms);
        let session_id_owned = session_id.to_string();

        wait_until(
            interval_ms,
            timeout_with_grace_ms,
            || async {
                let progress = self
                    .get_session_progress(&session_id_owned)
                    .await
                    .ok_or_else(|| missing_session_error(&session_id_owned))?;

                if progress.phase == SessionPhase::SessionReady {
                    return Ok(Some(progress));
                }
                if progress.phase == SessionPhase::Failed || progress.phase == SessionPhase::Closed
                {
                    let message = progress
                        .error_message
                        .clone()
                        .unwrap_or_else(|| "streamingStartFailed".to_string());
                    return Err(SessionFlowError::message(message));
                }
                Ok(None)
            },
            || {
                SessionFlowError::message(format!(
                    "streamingStartTimeout:sessionId={session_id_owned}"
                ))
            },
        )
        .await
    }

    async fn get_session_record(&self, session_id: &str) -> Option<SessionRuntimeRecord<S>> {
        let sessions = self.inner.sessions.read().await;
        sessions.get(session_id)
    }

    async fn clear_session(&self, session_id: &str) {
        let _ = self.inner.sessions.write().await.remove(session_id);
        let _ = self.inner.signaling.write().await.remove(session_id);
    }

    async fn create_session_api(
        &self,
        plan: &Plan,
    ) -> Result<WebApiSessionGateway, SessionFlowError> {
        let target_type = plan.session.target.as_str();
        let token_value = self.inner.provider.get_streaming_token(target_type).await?;
        let token = StreamingToken::parse(&token_value)
            .map_err(|e| SessionFlowError::message(e.to_string()))?;

        Ok(WebApiSessionGateway::new(plan.clone(), token))
    }

    async fn create_signaling_api(
        &self,
        plan: &Plan,
    ) -> Result<WebApiSignalingGateway, SessionFlowError> {
        let target_type = plan.session.target.as_str();
        let token_value = self.inner.provider.get_streaming_token(target_type).await?;
        let token = StreamingToken::parse(&token_value)
            .map_err(|e| SessionFlowError::message(e.to_string()))?;

        Ok(WebApiSignalingGateway::new(plan.clone(), token))
    }

    async fn filter_polled_ice_response(
        &self,
        session_id: &str,
        response: Option<Vec<IceCandidate>>,
        restart: bool,
    ) -> Option<Vec<IceCandidate>> {
        let mut signaling = self.inner.signaling.write().await;
        let state = signaling.entry(session_id.to_string()).or_default();
        let response =
            filter_stale_ice_response(response, state.restart_baseline_ice.as_deref(), restart);
        if response.as_deref() == state.last_polled_ice.as_deref() {
            return None;
        }
        if let Some(candidates) = response.as_ref() {
            state.last_polled_ice = Some(candidates.clone());
        }
        response
    }
}

fn filter_stale_offer_response(
    answer: Option<AnswerPayload>,
    previous_answer: Option<&AnswerPayload>,
    restart: bool,
) -> Option<AnswerPayload> {
    if !restart {
        return answer;
    }
    if answer.as_ref() == previous_answer {
        return None;
    }
    answer
}

fn filter_stale_ice_response(
    candidates: Option<Vec<IceCandidate>>,
    previous_candidates: Option<&[IceCandidate]>,
    restart: bool,
) -> Option<Vec<IceCandidate>> {
    if !restart {
        return candidates;
    }
    if candidates.as_deref() == previous_candidates {
        return None;
    }
    candidates
}

pub(crate) fn missing_session_error(session_id: &str) -> SessionFlowError {
    SessionFlowError::message(format!("Session not found: {session_id}"))
}

pub(crate) fn map_webapi_error(error: xbox_webapi::WebApiError) -> SessionFlowError {
    use xbox_webapi::WebApiError;
    match error {
        WebApiError::Http {
            status, message, ..
        } => SessionFlowError::http(status, format!("HTTP {status}: {message}"), Some(message)),
        other => SessionFlowError::message(other.to_string()),
    }
}

pub(crate) fn build_session_progress_snapshot<S: SessionFlowSnapshot>(
    record: SessionRuntimeRecord<S>,
) -> SessionProgressSnapshot {
    let runtime = record.snapshot.runtime_snapshot();
    let phase = resolve_session_phase(&runtime);
    let queue_seconds = runtime
        .queue
        .as_ref()
        .and_then(|queue| queue.details.estimated_total_wait_time_in_seconds);
    let queue = runtime.queue.as_ref().map(|queue| queue.details.clone());

    SessionProgressSnapshot {
        session_id: record.snapshot.session_id().to_string(),
        phase,
        status_text_key: default_status_text_key(phase).to_string(),
        // 当前尚无独立重试计数器，M1 先占位为 0，后续由 orchestrator 接管。
        retry_count: 0,
        queue_seconds,
        queue,
        error_code: runtime
            .error_details
            .as_ref()
            .and_then(|details| stringify_error_code(details.code.as_ref())),
        error_message: runtime.error_details.and_then(|details| details.message),
    }
}

fn resolve_session_phase(
    runtime: &crate::session::monitor::SessionRuntimeSnapshot,
) -> SessionPhase {
    match runtime.player_state.as_str() {
        "started" => SessionPhase::SessionReady,
        "failed" => SessionPhase::Failed,
        "queued" => SessionPhase::WaitingSessionReady,
        "pending" => match runtime.stream_state.as_deref() {
            Some("ReadyToConnect") => SessionPhase::RuntimeStarting,
            Some("Closing") => SessionPhase::Closing,
            Some("Closed") => SessionPhase::Closed,
            Some("Recovering") => SessionPhase::Recovering,
            _ => SessionPhase::WaitingSessionReady,
        },
        _ => SessionPhase::Creating,
    }
}

fn default_status_text_key(phase: SessionPhase) -> &'static str {
    match phase {
        SessionPhase::Creating => "streamPage.status.creatingSession",
        SessionPhase::WaitingSessionReady => "streamPage.status.waitingSession",
        SessionPhase::RuntimeStarting => "streamPage.status.startingPlayer",
        SessionPhase::SessionReady => "streamPage.status.startingPlayer",
        SessionPhase::Recovering => "streamPage.status.reconnecting",
        SessionPhase::Closing => "streamPage.status.disconnecting",
        SessionPhase::Closed => "streamPage.status.disconnected",
        SessionPhase::Failed => "streamPage.errors.startFailed",
    }
}

fn stringify_error_code(value: Option<&Value>) -> Option<String> {
    let value = value?;
    match value {
        Value::String(raw) => Some(raw.clone()),
        _ => Some(value.to_string()),
    }
}

fn matches_remote_console_id(target_id: &str, console: &RemoteConsoleSnapshot) -> bool {
    console.server_id.as_deref() == Some(target_id)
        || console.id.as_deref() == Some(target_id)
        || console.device_id.as_deref() == Some(target_id)
}

fn is_remote_console_ready(console: &RemoteConsoleSnapshot) -> bool {
    console.power_state.as_deref() == Some("On") && console.console_streaming_enabled != Some(false)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

async fn wait_until<F, Fut, T, E>(
    interval_ms: u64,
    timeout_ms: u64,
    check: F,
    timeout_error: E,
) -> Result<T, SessionFlowError>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<Option<T>, SessionFlowError>>,
    E: Fn() -> SessionFlowError,
{
    let started_at_ms = now_ms();
    loop {
        if let Some(result) = check().await? {
            return Ok(result);
        }

        let elapsed_ms = now_ms().saturating_sub(started_at_ms);
        if elapsed_ms >= timeout_ms {
            return Err(timeout_error());
        }

        tokio::time::sleep(std::time::Duration::from_millis(interval_ms)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::monitor::{SessionRuntimeBinding, SessionRuntimeSnapshot};
    use crate::session::store::SessionRuntimeRecord;

    #[derive(Clone)]
    struct Snapshot {
        session_id: String,
        session_path: String,
        target_id: String,
        target_type: String,
        runtime: SessionRuntimeSnapshot,
    }

    impl SessionRuntimeBinding for Snapshot {
        fn runtime_snapshot(&self) -> SessionRuntimeSnapshot {
            self.runtime.clone()
        }

        fn replace_runtime_snapshot(&mut self, runtime: SessionRuntimeSnapshot) {
            self.runtime = runtime;
        }
    }

    impl SessionFlowSnapshot for Snapshot {
        fn new_pending(
            session_id: String,
            session_path: String,
            target_id: String,
            target_type: String,
        ) -> Self {
            Self {
                session_id,
                session_path,
                target_id,
                target_type,
                runtime: SessionRuntimeSnapshot {
                    stream_state: None,
                    player_state: "pending".to_string(),
                    queue: None,
                    error_details: None,
                },
            }
        }

        fn session_id(&self) -> &str {
            &self.session_id
        }

        fn session_path(&self) -> &str {
            &self.session_path
        }

        fn target_id(&self) -> &str {
            &self.target_id
        }

        fn target_type(&self) -> &str {
            &self.target_type
        }
    }

    fn snapshot_with_runtime(runtime: SessionRuntimeSnapshot) -> Snapshot {
        Snapshot {
            session_id: "session-1".to_string(),
            session_path: "/v5/sessions/cloud/session-1".to_string(),
            target_id: "target-1".to_string(),
            target_type: "cloud".to_string(),
            runtime,
        }
    }

    #[test]
    fn started_player_state_maps_to_session_ready_phase() {
        let phase = resolve_session_phase(&SessionRuntimeSnapshot {
            stream_state: Some("Provisioned".to_string()),
            player_state: "started".to_string(),
            queue: None,
            error_details: None,
        });

        assert_eq!(phase, SessionPhase::SessionReady);
        assert_eq!(
            default_status_text_key(phase),
            "streamPage.status.startingPlayer"
        );
    }

    #[test]
    fn build_session_progress_snapshot_uses_session_ready_for_started_session() {
        let record = SessionRuntimeRecord::new(
            snapshot_with_runtime(SessionRuntimeSnapshot {
                stream_state: Some("Provisioned".to_string()),
                player_state: "started".to_string(),
                queue: None,
                error_details: None,
            }),
            crate::policy::Plan::default(),
            0,
        );

        let progress = build_session_progress_snapshot(record);

        assert_eq!(progress.phase, SessionPhase::SessionReady);
        assert_eq!(progress.status_text_key, "streamPage.status.startingPlayer");
    }

    #[test]
    fn restart_offer_filter_ignores_previous_answer_snapshot() {
        let answer = AnswerPayload {
            sdp: "answer-1".to_string(),
            message_type: Some("answer".to_string()),
        };

        assert_eq!(
            filter_stale_offer_response(Some(answer.clone()), Some(&answer), true),
            None
        );
        assert_eq!(
            filter_stale_offer_response(Some(answer.clone()), Some(&answer), false),
            Some(answer)
        );
    }

    #[test]
    fn restart_ice_filter_ignores_previous_candidate_snapshot() {
        let candidates = vec![IceCandidate {
            candidate: "a=candidate:1 1 UDP 1 10.0.0.1 9000 typ host".to_string(),
            ..Default::default()
        }];

        assert_eq!(
            filter_stale_ice_response(Some(candidates.clone()), Some(&candidates), true),
            None
        );
        assert_eq!(
            filter_stale_ice_response(Some(candidates.clone()), Some(&candidates), false),
            Some(candidates)
        );
    }
}
