use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use std::sync::{Arc, Mutex};

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

const STARTUP_CLOSED_RECOVERY_GRACE_MS_MIN: u64 = 1_200;
const STARTUP_CLOSED_RECOVERY_GRACE_MS_MAX: u64 = 5_000;
const HOME_SESSION_READY_RETRY_LIMIT: u8 = 0;
const HOME_SERVER_REGISTRATION_WAIT_TIMEOUT_MS: u64 = 30_000;
const HOME_RECREATE_CLEANUP_SETTLE_MS: u64 = 8_000;
const HOME_RECREATE_CLEANUP_POLL_MS: u64 = 500;

/// session flow 的统一错误，便于 adapter 只做一次映射。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionFlowError {
    pub message: String,
    pub status: Option<u16>,
    pub body: Option<String>,
    pub startup_hint: Option<SessionFlowStartupErrorHint>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SessionFlowStartupErrorKind {
    Wake,
    ConsoleReady,
    SessionCreate,
    SessionReady,
    Runtime,
    Network,
    Auth,
    Target,
    HostRemotePlayUnavailable,
    HostRegistrationRetryExhausted,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionFlowStartupErrorHint {
    pub kind: SessionFlowStartupErrorKind,
    pub retryable: bool,
    pub diagnostic_summary: String,
}

impl SessionFlowError {
    pub fn message(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            status: None,
            body: None,
            startup_hint: None,
        }
    }

    pub fn http(status: u16, message: impl Into<String>, body: Option<String>) -> Self {
        Self {
            message: message.into(),
            status: Some(status),
            body,
            startup_hint: None,
        }
    }

    pub fn with_startup_hint(mut self, startup_hint: SessionFlowStartupErrorHint) -> Self {
        self.startup_hint = Some(startup_hint);
        self
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
    pub queue_seconds: Option<u64>,
    pub queue: Option<crate::session::monitor::QueueDetails>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub error_hint: Option<SessionFlowStartupErrorHint>,
}

/// 会话创建前阶段：供 adapter 订阅真实启动进度，而不是在 UI 侧猜测。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SessionStartupPhase {
    ResolvingContext,
    WakingConsole,
    WaitingConsoleReady,
    CreatingSession,
    WaitingSessionReady,
    StartingRuntime,
    Ready,
    Failed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SessionStartupPhaseStatus {
    Entered,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SessionStartupBoundedRetryStatus {
    Retrying,
    Exhausted,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SessionStartupBoundedRetryReason {
    WaitingForServerRegistration,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionStartupBoundedRetrySnapshot {
    pub reason: SessionStartupBoundedRetryReason,
    pub status: SessionStartupBoundedRetryStatus,
    pub retry_count: u8,
    pub retry_limit: u8,
}

pub trait SessionStartupObserver: Send + Sync {
    fn on_phase_event(
        &self,
        phase: SessionStartupPhase,
        status: SessionStartupPhaseStatus,
        details: Option<&str>,
    );

    fn on_bounded_retry(
        &self,
        _phase: SessionStartupPhase,
        _bounded_retry: &SessionStartupBoundedRetrySnapshot,
    ) {
    }
}

/// 远端主机快照：仅保留 session 预检所需字段。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct RemoteConsoleSnapshot {
    pub id: Option<String>,
    pub device_id: Option<String>,
    pub server_id: Option<String>,
    pub power_state: Option<String>,
    pub remote_management_enabled: Option<bool>,
    pub console_streaming_enabled: Option<bool>,
    pub console_addrs_count: u32,
    pub ready_source: Option<String>,
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
    /// 诊断钩子：记录 session state 轮询结果，默认 no-op，避免 core 层绑定具体日志设施。
    fn on_session_state_polled(
        &self,
        _session_id: &str,
        _target_type: &str,
        _target_id: &str,
        _state: Option<&str>,
        _error_code: Option<&Value>,
        _error_message: Option<&str>,
    ) {
    }
    /// 诊断钩子：记录 session state 轮询失败。
    fn on_session_state_poll_failed(
        &self,
        _session_id: &str,
        _target_type: &str,
        _target_id: &str,
        _error: &SessionFlowError,
    ) {
    }
    /// 诊断钩子：记录 monitor tick 投影结果，便于定位 waitingSessionReady 卡点。
    fn on_session_monitor_tick(
        &self,
        _session_id: &str,
        _target_type: &str,
        _target_id: &str,
        _progress: &SessionProgressSnapshot,
        _stream_state: Option<&str>,
        _player_state: &str,
        _should_continue: bool,
        _should_send_connect_token: bool,
    ) {
    }
    /// 诊断钩子：记录 connect token 分发结果。
    fn on_session_connect_token_result(
        &self,
        _session_id: &str,
        _target_type: &str,
        _target_id: &str,
        _status: &str,
        _error: Option<&SessionFlowError>,
    ) {
    }
    /// 诊断钩子：记录 create_session 返回的 session 标识，便于确认 recreate 是否真的换了 session。
    fn on_session_created(
        &self,
        _session_id: &str,
        _session_path: &str,
        _target_type: &str,
        _target_id: &str,
        _recreate_from_session_id: Option<&str>,
    ) {
    }
    /// 诊断钩子：记录 recreate 前旧 session 清理是否真正收敛。
    fn on_session_recreate_cleanup_result(
        &self,
        _session_id: &str,
        _target_type: &str,
        _target_id: &str,
        _status: &str,
        _last_state: Option<&str>,
        _error: Option<&SessionFlowError>,
    ) {
    }
    /// 诊断钩子：记录 waitingConsoleReady 的收敛结果，便于区分显式注册与超时。
    fn on_console_ready_wait_result(
        &self,
        _target_type: &str,
        _target_id: &str,
        _status: &str,
        _reason: &str,
        _console: Option<&RemoteConsoleSnapshot>,
    ) {
    }
}

#[derive(Debug, Clone)]
struct SessionRecreateContext {
    previous_session_id: String,
    cleanup_settled: bool,
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
        self.create_session_with_observer::<NoopSessionStartupObserver>(plan, None, None)
            .await
    }

    async fn create_session_with_observer<O>(
        &self,
        plan: Plan,
        observer: Option<&O>,
        recreate_from_session_id: Option<&str>,
    ) -> Result<S, SessionFlowError>
    where
        O: SessionStartupObserver,
    {
        let monitor_interval_ms = plan.session.schedule.monitor_interval_ms.max(200);
        let keepalive_interval_ms = plan.session.schedule.keepalive_interval_ms.max(1_000);
        let api = self.create_session_api(&plan).await?;
        notify_startup_phase(
            observer,
            SessionStartupPhase::CreatingSession,
            SessionStartupPhaseStatus::Entered,
            None,
        );
        let session_path = self.start_stream_with_retry(&api, &plan).await?;
        notify_startup_phase(
            observer,
            SessionStartupPhase::CreatingSession,
            SessionStartupPhaseStatus::Succeeded,
            None,
        );

        let session_id = parse_session_id_from_path(&session_path)
            .map_err(|error| SessionFlowError::message(error.to_string()))?;

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
        let runtime = project_runtime(&plan);
        let render = project_render(&plan);
        let schedule = plan.session.schedule.clone();
        let mut recreate_retry_count = 0u8;
        let mut recreate_context: Option<SessionRecreateContext> = None;

        self.prepare_remote_console(&plan, observer).await?;

        loop {
            let recreate_from_session_id = recreate_context
                .as_ref()
                .map(|context| context.previous_session_id.as_str());
            let session = self
                .create_session_with_observer(plan.clone(), observer, recreate_from_session_id)
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

            match self
                .wait_until_session_started_or_failed(&session_id, &schedule)
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
                                let cleanup_settled = self
                                    .wait_until_session_cleanup_settled(&plan, &session_id)
                                    .await;
                                // home 场景下 recreate 前补一次主机准备，给服务端重新注册留出机会。
                                self.prepare_remote_console(&plan, observer).await?;
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
                    return Err(wait_error);
                }
            }
        }
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
        let api = self.create_signaling_api(plan).await?;
        let response = api
            .get_ice_exchange_response(session_id)
            .await
            .map_err(map_webapi_error)?;
        let response = self
            .filter_polled_ice_response(session_id, response, restart)
            .await;
        let session_exists = self.get_session_record(session_id).await.is_some();
        resolve_polled_ice_result(session_id, response, session_exists)
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

    async fn prepare_remote_console<O>(
        &self,
        plan: &Plan,
        observer: Option<&O>,
    ) -> Result<(), SessionFlowError>
    where
        O: SessionStartupObserver,
    {
        if !plan.session.target.is_home() || !plan.session.schedule.wake_console {
            return Ok(());
        }
        notify_startup_phase(
            observer,
            SessionStartupPhase::WakingConsole,
            SessionStartupPhaseStatus::Entered,
            None,
        );
        let wake_accepted = match self
            .inner
            .provider
            .power_on_console(&plan.session.target_id)
            .await
        {
            Ok(accepted) => accepted,
            Err(error) if is_waiting_for_server_registration_message(&error.message) => {
                log::warn!(
                    "home wake command is still waiting for server registration: target_id={} error={}",
                    plan.session.target_id,
                    error,
                );
                true
            }
            Err(error) => return Err(error),
        };
        notify_startup_phase(
            observer,
            SessionStartupPhase::WakingConsole,
            SessionStartupPhaseStatus::Succeeded,
            Some(if wake_accepted {
                "wakeCommandAccepted"
            } else {
                "wakeCommandRejected"
            }),
        );
        if !wake_accepted || !plan.session.schedule.require_console_ready {
            return Ok(());
        }

        notify_startup_phase(
            observer,
            SessionStartupPhase::WaitingConsoleReady,
            SessionStartupPhaseStatus::Entered,
            None,
        );
        let ready_reason = self
            .wait_until_console_ready(
                plan.session.target.as_str(),
                &plan.session.target_id,
                &plan.session.schedule,
            )
            .await?;
        notify_startup_phase(
            observer,
            SessionStartupPhase::WaitingConsoleReady,
            SessionStartupPhaseStatus::Succeeded,
            Some(ready_reason),
        );
        Ok(())
    }

    async fn wait_until_console_ready(
        &self,
        target_type: &str,
        target_id: &str,
        schedule: &crate::policy::session::SessionSchedulePlan,
    ) -> Result<&'static str, SessionFlowError> {
        let interval_ms = schedule.monitor_interval_ms.max(200);
        let started_at_ms = now_ms();
        let mut last_wake_attempt_at_ms = Some(started_at_ms);
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
                if should_retry_wake_during_ready_wait(power_state, last_wake_attempt_at_ms, now_ms)
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
                    if let Err(wait_error) = self
                        .wait_until_console_ready(
                            plan.session.target.as_str(),
                            &plan.session.target_id,
                            &plan.session.schedule,
                        )
                        .await
                    {
                        log::warn!(
                            "home console ready precheck did not pass before start_stream retry: target_id={} error={}",
                            plan.session.target_id,
                            wait_error,
                        );
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
                    attempt += 1;
                }
            }
        }
    }

    async fn wait_until_session_started_or_failed(
        &self,
        session_id: &str,
        schedule: &crate::policy::session::SessionSchedulePlan,
    ) -> Result<SessionProgressSnapshot, SessionFlowError> {
        let interval_ms = schedule.monitor_interval_ms.max(200);
        let closed_recovery_grace_ms = startup_closed_recovery_grace_ms(interval_ms);
        // 给 monitor 一次额外 tick，把精确踩线的“卡住”状态先收敛成 failed，
        // 避免这里抢先抛出通用 timeout，丢掉更具体的上下文。
        let timeout_with_grace_ms = schedule.startup_timeout_ms.saturating_add(interval_ms);
        let session_id_owned = session_id.to_string();
        let last_recovery_signal_at_ms = Arc::new(Mutex::new(now_ms()));
        let home_registration_wait_started_at_ms = Arc::new(Mutex::new(None::<u64>));
        let session_id_for_check = session_id_owned.clone();
        let recovery_signal_for_check = Arc::clone(&last_recovery_signal_at_ms);
        let home_registration_wait_for_check = Arc::clone(&home_registration_wait_started_at_ms);

        wait_until(
            interval_ms,
            timeout_with_grace_ms,
            move || {
                let session_id_for_check = session_id_for_check.clone();
                let recovery_signal_for_check = Arc::clone(&recovery_signal_for_check);
                let home_registration_wait_for_check = Arc::clone(&home_registration_wait_for_check);
                async move {
                let record = self
                    .get_session_record(&session_id_for_check)
                    .await
                    .ok_or_else(|| missing_session_error(&session_id_for_check))?;
                let runtime = record.snapshot.runtime_snapshot();
                let is_home = record.plan.session.target.is_home();
                let target_id = record.plan.session.target_id.clone();
                let progress = build_session_progress_snapshot(record);

                let now_ms = now_ms();
                let mut home_registration_wait_started_at_ms =
                    home_registration_wait_for_check.lock().map_err(|_| {
                        SessionFlowError::message(
                            "startupWaitRegistrationStateLockFailed:sessionFlowClosedGuard",
                        )
                    })?;
                if let Some(error) = evaluate_home_server_registration_wait_timeout(
                    is_home,
                    &target_id,
                    &session_id_for_check,
                    &runtime,
                    now_ms,
                    &mut home_registration_wait_started_at_ms,
                ) {
                    return Err(error);
                }
                let mut last_recovery_signal_at_ms =
                    recovery_signal_for_check.lock().map_err(|_| {
                        SessionFlowError::message(
                            "startupWaitRecoveryStateLockFailed:sessionFlowClosedGuard",
                        )
                    })?;
                match decide_startup_progress_action(
                    &progress,
                    now_ms,
                    &mut last_recovery_signal_at_ms,
                    closed_recovery_grace_ms,
                ) {
                    StartupProgressAction::Ready => return Ok(Some(progress)),
                    StartupProgressAction::Continue { transient_closed } => {
                        if transient_closed {
                            log::warn!(
                                "startup wait keeps polling after transient closed: session_id={} phase={:?} grace_ms={} since_recovery_ms={}",
                                session_id_for_check,
                                progress.phase,
                                closed_recovery_grace_ms,
                                now_ms.saturating_sub(*last_recovery_signal_at_ms),
                            );
                        }
                    }
                    StartupProgressAction::Fail(message) => {
                        log::warn!(
                            "startup wait failing: session_id={} phase={:?} message={}",
                            session_id_for_check,
                            progress.phase,
                            message
                        );
                        return Err(SessionFlowError::message(message));
                    }
                }
                Ok(None)
            }
            },
            || {
                startup_timeout_error(&session_id_owned)
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

    async fn cleanup_session_for_recreate(&self, session_id: &str) {
        if let Err(error) = self.close_session(session_id).await {
            log::warn!(
                "best-effort startup retry cleanup failed, continuing with recreate: session_id={} error={}",
                session_id,
                error,
            );
        }
        self.clear_session(session_id).await;
    }

    async fn wait_until_session_cleanup_settled(&self, plan: &Plan, session_id: &str) -> bool {
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

#[derive(Debug, Clone, Copy)]
struct NoopSessionStartupObserver;

impl SessionStartupObserver for NoopSessionStartupObserver {
    fn on_phase_event(
        &self,
        _phase: SessionStartupPhase,
        _status: SessionStartupPhaseStatus,
        _details: Option<&str>,
    ) {
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

    let error_code = runtime
        .error_details
        .as_ref()
        .and_then(|details| stringify_error_code(details.code.as_ref()));
    let error_message = runtime.error_details.and_then(|details| details.message);
    let error_hint =
        build_session_progress_error_hint(phase, error_code.as_deref(), error_message.as_deref());

    SessionProgressSnapshot {
        session_id: record.snapshot.session_id().to_string(),
        phase,
        status_text_key: default_status_text_key(phase).to_string(),
        queue_seconds,
        queue,
        error_code,
        error_message,
        error_hint,
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

fn build_session_progress_error_hint(
    phase: SessionPhase,
    error_code: Option<&str>,
    error_message: Option<&str>,
) -> Option<SessionFlowStartupErrorHint> {
    let primary_signal = error_message
        .filter(|value| !value.trim().is_empty())
        .or_else(|| error_code.filter(|value| !value.trim().is_empty()))?;
    let kind = classify_session_progress_error_kind(phase, primary_signal);

    Some(SessionFlowStartupErrorHint {
        kind: kind.clone(),
        retryable: is_session_progress_error_retryable(&kind),
        diagnostic_summary: build_session_progress_diagnostic_summary(
            phase,
            kind,
            error_code,
            error_message,
        ),
    })
}

fn classify_session_progress_error_kind(
    phase: SessionPhase,
    primary_signal: &str,
) -> SessionFlowStartupErrorKind {
    let normalized = primary_signal.to_ascii_lowercase();
    if is_server_registration_retry_signal(primary_signal) {
        return SessionFlowStartupErrorKind::HostRegistrationRetryExhausted;
    }
    if normalized.contains("remoteconsolenotready") {
        return SessionFlowStartupErrorKind::ConsoleReady;
    }
    if normalized.contains("streamingstarttimeout") {
        return SessionFlowStartupErrorKind::SessionReady;
    }
    if normalized.contains("targetmissing") {
        return SessionFlowStartupErrorKind::Target;
    }
    if normalized.contains("unauthorized")
        || normalized.contains("forbidden")
        || normalized.contains("authentication")
        || normalized.contains("auth")
    {
        return SessionFlowStartupErrorKind::Auth;
    }
    if normalized.contains("network")
        || normalized.contains("reconnect")
        || normalized.contains("recover")
    {
        return SessionFlowStartupErrorKind::Network;
    }

    match phase {
        SessionPhase::Failed | SessionPhase::Closed | SessionPhase::Recovering => {
            SessionFlowStartupErrorKind::Runtime
        }
        _ => SessionFlowStartupErrorKind::Unknown,
    }
}

fn is_session_progress_error_retryable(kind: &SessionFlowStartupErrorKind) -> bool {
    matches!(
        kind,
        SessionFlowStartupErrorKind::ConsoleReady
            | SessionFlowStartupErrorKind::SessionCreate
            | SessionFlowStartupErrorKind::SessionReady
            | SessionFlowStartupErrorKind::Runtime
            | SessionFlowStartupErrorKind::Network
    )
}

fn build_session_progress_diagnostic_summary(
    phase: SessionPhase,
    kind: SessionFlowStartupErrorKind,
    error_code: Option<&str>,
    error_message: Option<&str>,
) -> String {
    let error_code = error_code.unwrap_or("none");
    let error_message = error_message.unwrap_or("none");
    let hint = match kind {
        SessionFlowStartupErrorKind::ConsoleReady => "remoteConsoleNotReady",
        SessionFlowStartupErrorKind::SessionReady => "streamingStartTimeout",
        SessionFlowStartupErrorKind::HostRegistrationRetryExhausted => {
            "hostRegistrationRetryExhausted"
        }
        SessionFlowStartupErrorKind::HostRemotePlayUnavailable => "hostRemotePlayUnavailable",
        SessionFlowStartupErrorKind::Wake => "wakeFailed",
        SessionFlowStartupErrorKind::SessionCreate => "sessionCreateFailed",
        SessionFlowStartupErrorKind::Runtime => "runtimeFailed",
        SessionFlowStartupErrorKind::Network => "networkFailed",
        SessionFlowStartupErrorKind::Auth => "authFailed",
        SessionFlowStartupErrorKind::Target => "targetMissing",
        SessionFlowStartupErrorKind::Unknown => "unknown",
    };
    format!("phase={phase:?}; errorCode={error_code}; errorMessage={error_message}; hint={hint}")
}

fn matches_remote_console_id(target_id: &str, console: &RemoteConsoleSnapshot) -> bool {
    console.server_id.as_deref() == Some(target_id)
        || console.id.as_deref() == Some(target_id)
        || console.device_id.as_deref() == Some(target_id)
}

#[cfg(test)]
fn is_remote_console_ready(console: &RemoteConsoleSnapshot) -> bool {
    remote_console_ready_reason(console).is_some()
}

fn remote_console_ready_reason(console: &RemoteConsoleSnapshot) -> Option<&'static str> {
    if !is_remote_console_power_ready(console) {
        return None;
    }

    if console.remote_management_enabled == Some(true) {
        return Some("explicitRegistration");
    }

    None
}

fn is_remote_console_power_ready(console: &RemoteConsoleSnapshot) -> bool {
    console.power_state.as_deref() == Some("On") && console.console_streaming_enabled != Some(false)
}

fn remote_console_wake_circuit_open_error(
    target_id: &str,
    power_state: Option<&str>,
    wake_failure_count: u8,
) -> SessionFlowError {
    let power_state = power_state.unwrap_or("unknown");
    let diagnostic_summary = format!(
        "targetId={target_id}; powerState={power_state}; wakeFailureCount={wake_failure_count}; hint=hostRemotePlayUnavailable"
    );
    SessionFlowError::message(format!(
        "remoteConsoleWakeCircuitOpen:targetId={target_id};powerState={power_state};wakeFailureCount={wake_failure_count}"
    ))
    .with_startup_hint(SessionFlowStartupErrorHint {
        kind: SessionFlowStartupErrorKind::HostRemotePlayUnavailable,
        retryable: false,
        diagnostic_summary,
    })
}

#[cfg(test)]
fn is_remote_console_wake_circuit_open_message(message: &str) -> bool {
    message
        .to_ascii_lowercase()
        .contains("remoteconsolewakecircuitopen")
}

fn should_retry_wake_during_ready_wait(
    power_state: Option<&str>,
    last_wake_attempt_at_ms: Option<u64>,
    now_ms: u64,
) -> bool {
    if !matches!(power_state, Some("ConnectedStandby") | Some("Off")) {
        return false;
    }

    let elapsed_since_last_wake_ms = last_wake_attempt_at_ms
        .map(|last_ms| now_ms.saturating_sub(last_ms))
        .unwrap_or(u64::MAX);
    elapsed_since_last_wake_ms >= 5_000
}

fn is_session_cleanup_terminal_state(state: Option<&str>) -> bool {
    matches!(state, Some("Closed") | Some("Failed"))
}

fn should_fail_home_recreate_same_session(
    cleanup_settled: bool,
    previous_session_id: Option<&str>,
    next_session_id: &str,
) -> bool {
    !cleanup_settled && previous_session_id == Some(next_session_id)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum StartupProgressAction {
    Ready,
    Continue { transient_closed: bool },
    Fail(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct SessionReadyRetryProbe {
    phase: Option<SessionPhase>,
    stream_state: Option<String>,
    error_code: Option<String>,
    error_message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionReadyRetryDecision {
    Retry(SessionReadyRecreateRetryReason),
    Exhausted(SessionReadyRecreateRetryReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionReadyRecreateRetryReason {
    WaitingForServerRegistration,
}

fn startup_closed_recovery_grace_ms(interval_ms: u64) -> u64 {
    interval_ms.saturating_mul(3).clamp(
        STARTUP_CLOSED_RECOVERY_GRACE_MS_MIN,
        STARTUP_CLOSED_RECOVERY_GRACE_MS_MAX,
    )
}

fn has_startup_recovery_signal(progress: &SessionProgressSnapshot) -> bool {
    if matches!(
        progress.phase,
        SessionPhase::Creating
            | SessionPhase::WaitingSessionReady
            | SessionPhase::RuntimeStarting
            | SessionPhase::Recovering
    ) {
        return true;
    }
    progress
        .error_message
        .as_deref()
        .map(str::to_ascii_lowercase)
        .is_some_and(|message| {
            message.contains("reconnect")
                || message.contains("recover")
                || message.contains("networklost")
        })
}

fn decide_startup_progress_action(
    progress: &SessionProgressSnapshot,
    now_ms: u64,
    last_recovery_signal_at_ms: &mut u64,
    closed_recovery_grace_ms: u64,
) -> StartupProgressAction {
    if progress.phase == SessionPhase::SessionReady {
        return StartupProgressAction::Ready;
    }

    if progress.phase == SessionPhase::Failed {
        return StartupProgressAction::Fail(
            progress
                .error_message
                .clone()
                .unwrap_or_else(|| "streamingStartFailed".to_string()),
        );
    }

    if has_startup_recovery_signal(progress) {
        *last_recovery_signal_at_ms = now_ms;
    }

    if progress.phase == SessionPhase::Closed {
        let in_recovery_window =
            now_ms.saturating_sub(*last_recovery_signal_at_ms) <= closed_recovery_grace_ms;
        if in_recovery_window {
            return StartupProgressAction::Continue {
                transient_closed: true,
            };
        }
        return StartupProgressAction::Fail(
            progress
                .error_message
                .clone()
                .unwrap_or_else(|| "streamingStartFailed".to_string()),
        );
    }

    StartupProgressAction::Continue {
        transient_closed: false,
    }
}

fn notify_startup_phase<O>(
    observer: Option<&O>,
    phase: SessionStartupPhase,
    status: SessionStartupPhaseStatus,
    details: Option<&str>,
) where
    O: SessionStartupObserver,
{
    if let Some(observer) = observer {
        observer.on_phase_event(phase, status, details);
    }
}

fn notify_startup_bounded_retry<O>(
    observer: Option<&O>,
    phase: SessionStartupPhase,
    bounded_retry: SessionStartupBoundedRetrySnapshot,
) where
    O: SessionStartupObserver,
{
    if let Some(observer) = observer {
        observer.on_bounded_retry(phase, &bounded_retry);
    }
}

fn remote_console_not_ready_error(target_id: &str) -> SessionFlowError {
    let diagnostic_summary = format!("targetId={target_id}; hint=remoteConsoleNotReady");
    SessionFlowError::message(format!("remoteConsoleNotReady:targetId={target_id}"))
        .with_startup_hint(SessionFlowStartupErrorHint {
            kind: SessionFlowStartupErrorKind::ConsoleReady,
            retryable: true,
            diagnostic_summary,
        })
}

fn startup_timeout_error(session_id: &str) -> SessionFlowError {
    let diagnostic_summary = format!("sessionId={session_id}; hint=streamingStartTimeout");
    SessionFlowError::message(format!("streamingStartTimeout:sessionId={session_id}"))
        .with_startup_hint(SessionFlowStartupErrorHint {
            kind: SessionFlowStartupErrorKind::SessionReady,
            retryable: true,
            diagnostic_summary,
        })
}

fn home_server_registration_timeout_error(
    target_id: &str,
    session_id: &str,
    elapsed_ms: u64,
) -> SessionFlowError {
    let diagnostic_summary = format!(
        "targetId={target_id}; sessionId={session_id}; reason=waitingForServerRegistration; elapsedMs={elapsed_ms}; hint=hostRegistrationRetryExhausted"
    );
    SessionFlowError::message(format!(
        "homeServerRegistrationTimeout:targetId={target_id};sessionId={session_id};reason=waitingForServerRegistration;elapsedMs={elapsed_ms}"
    ))
    .with_startup_hint(SessionFlowStartupErrorHint {
        kind: SessionFlowStartupErrorKind::HostRegistrationRetryExhausted,
        retryable: false,
        diagnostic_summary,
    })
}

fn evaluate_home_server_registration_wait_timeout(
    is_home: bool,
    target_id: &str,
    session_id: &str,
    runtime: &crate::session::monitor::SessionRuntimeSnapshot,
    now_ms: u64,
    wait_started_at_ms: &mut Option<u64>,
) -> Option<SessionFlowError> {
    let is_provisioning_pending =
        runtime.player_state == "pending" && runtime.stream_state.as_deref() == Some("Provisioning");
    if !is_home || !is_provisioning_pending {
        *wait_started_at_ms = None;
        return None;
    }

    let started_at_ms = wait_started_at_ms.get_or_insert(now_ms);
    let elapsed_ms = now_ms.saturating_sub(*started_at_ms);
    if elapsed_ms < HOME_SERVER_REGISTRATION_WAIT_TIMEOUT_MS {
        return None;
    }

    Some(home_server_registration_timeout_error(
        target_id,
        session_id,
        elapsed_ms,
    ))
}

fn should_retry_home_server_registration(
    plan: &Plan,
    error: &xbox_webapi::WebApiError,
    elapsed_ms: u64,
    retry_timeout_ms: u64,
) -> bool {
    plan.session.target.is_home()
        && elapsed_ms < retry_timeout_ms
        && is_waiting_for_server_registration_error(error)
}

fn decide_home_session_ready_recreate_retry(
    is_home: bool,
    retry_count: u8,
    wait_error_message: &str,
    latest_phase: Option<SessionPhase>,
    latest_stream_state: Option<&str>,
    latest_error_code: Option<&str>,
    latest_error_message: Option<&str>,
) -> Option<SessionReadyRetryDecision> {
    if !is_home {
        return None;
    }

    if !matches!(latest_stream_state, Some("Provisioning") | Some("Failed")) {
        return None;
    }

    if !matches!(
        latest_phase,
        Some(SessionPhase::WaitingSessionReady) | Some(SessionPhase::Failed)
    ) {
        return None;
    }

    // 对齐 XStreamingDesktop：Provisioning 本身只继续轮询，不再因为本地超时或卡点推断而主动 recreate。
    // 这里仅保留“服务端尚未完成注册”的显式错误分支，避免把本来会继续推进的 session 提前打断。
    // 同时允许该错误落在最终 Failed 终态，覆盖 wake 后第一次 create 最终以 ServerNeverRegistered 收敛、
    // 但稍后手动重试即可成功的场景。
    if is_server_registration_retry_signal(wait_error_message)
        || latest_error_code.is_some_and(is_server_registration_retry_signal)
        || latest_error_message.is_some_and(is_server_registration_retry_signal)
    {
        let reason = SessionReadyRecreateRetryReason::WaitingForServerRegistration;
        if retry_count < HOME_SESSION_READY_RETRY_LIMIT {
            return Some(SessionReadyRetryDecision::Retry(reason));
        }
        return Some(SessionReadyRetryDecision::Exhausted(reason));
    }

    None
}

fn map_session_ready_retry_reason(
    reason: SessionReadyRecreateRetryReason,
) -> SessionStartupBoundedRetryReason {
    match reason {
        SessionReadyRecreateRetryReason::WaitingForServerRegistration => {
            SessionStartupBoundedRetryReason::WaitingForServerRegistration
        }
    }
}

fn session_ready_retry_reason_code(reason: SessionReadyRecreateRetryReason) -> &'static str {
    match reason {
        SessionReadyRecreateRetryReason::WaitingForServerRegistration => {
            "waitingForServerRegistration"
        }
    }
}

fn build_home_session_ready_retry_exhausted_error(
    target_id: &str,
    reason: SessionReadyRecreateRetryReason,
    retry_count: u8,
    retry_limit: u8,
    source: SessionFlowError,
) -> SessionFlowError {
    SessionFlowError {
        message: format!(
            "homeSessionBoundedRetryExhausted:targetId={target_id};reason={};retryCount={retry_count};retryLimit={retry_limit}",
            session_ready_retry_reason_code(reason)
        ),
        status: source.status,
        body: source.body.or(Some(source.message)),
        startup_hint: Some(SessionFlowStartupErrorHint {
            kind: SessionFlowStartupErrorKind::HostRegistrationRetryExhausted,
            retryable: false,
            diagnostic_summary: format!(
                "targetId={target_id}; reason={}; retryCount={retry_count}; retryLimit={retry_limit}; hint=hostRegistrationRetryExhausted",
                session_ready_retry_reason_code(reason)
            ),
        }),
    }
}

fn next_retry_backoff_ms(retry_backoff_ms: &[u64], attempt: usize) -> u64 {
    retry_backoff_ms
        .get(attempt)
        .copied()
        .or_else(|| retry_backoff_ms.last().copied())
        .unwrap_or(1_000)
}

fn resolve_polled_ice_result(
    session_id: &str,
    response: Option<Vec<IceCandidate>>,
    session_exists: bool,
) -> Result<Vec<IceCandidate>, SessionFlowError> {
    match decide_ice_poll(response.as_deref(), session_exists) {
        PollDecision::SessionMissing => Err(missing_session_error(session_id)),
        // ICE 轮询节奏由 runtime 控制，这里只做单次快照读取，避免把上层阶段机阻塞在服务层。
        PollDecision::Completed | PollDecision::Continue => Ok(response.unwrap_or_default()),
    }
}

fn is_waiting_for_server_registration_error(error: &xbox_webapi::WebApiError) -> bool {
    is_server_registration_retry_signal(&error.to_string())
}

fn is_server_registration_retry_signal(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    normalized.contains("serverneverregistered")
        || is_waiting_for_server_registration_message(message)
}

fn is_waiting_for_server_registration_message(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    normalized.contains("waitingforservertoregister")
        || normalized.contains("errorcallingwns")
        || (normalized.contains("xccs") && normalized.contains("send command failed"))
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
    mut check: F,
    timeout_error: E,
) -> Result<T, SessionFlowError>
where
    F: FnMut() -> Fut,
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

    #[test]
    fn waiting_for_server_registration_http_error_is_retryable_for_home() {
        let mut plan = crate::policy::Plan::default();
        plan.session.target = crate::policy::types::Target::Home;
        plan.session.schedule.ready_timeout_ms = 10_000;

        let error = xbox_webapi::WebApiError::http(
            503,
            "Streaming error: Xccs : ErrorCallingWNS : Send command failed : State WaitingForServerToRegister",
        );

        assert!(should_retry_home_server_registration(
            &plan,
            &error,
            0,
            plan.session.schedule.ready_timeout_ms,
        ));
        assert!(!should_retry_home_server_registration(
            &plan,
            &error,
            plan.session.schedule.ready_timeout_ms,
            plan.session.schedule.ready_timeout_ms,
        ));
    }

    #[test]
    fn waiting_for_server_registration_retry_does_not_apply_to_cloud() {
        let mut plan = crate::policy::Plan::default();
        plan.session.target = crate::policy::types::Target::Cloud;
        plan.session.schedule.ready_timeout_ms = 10_000;

        let error = xbox_webapi::WebApiError::http(
            503,
            "Streaming error: Xccs : ErrorCallingWNS : Send command failed : State WaitingForServerToRegister",
        );

        assert!(!should_retry_home_server_registration(
            &plan,
            &error,
            0,
            plan.session.schedule.ready_timeout_ms,
        ));
    }

    #[test]
    fn waiting_for_server_registration_message_matches_non_http_error_text() {
        assert!(is_waiting_for_server_registration_message(
            "Xccs : ErrorCallingWNS : Send command failed : State WaitingForServerToRegister",
        ));
        assert!(!is_waiting_for_server_registration_message(
            "remoteConsoleNotReady"
        ));
    }

    #[test]
    fn server_never_registered_message_is_treated_as_registration_signal() {
        assert!(is_server_registration_retry_signal("ServerNeverRegistered"));
        assert!(!is_server_registration_retry_signal(
            "streamingStartTimeout:sessionId=session-1"
        ));
    }

    #[test]
    fn retry_backoff_reuses_last_entry_after_sequence_is_exhausted() {
        assert_eq!(next_retry_backoff_ms(&[1_000, 3_000, 5_000], 0), 1_000);
        assert_eq!(next_retry_backoff_ms(&[1_000, 3_000, 5_000], 2), 5_000);
        assert_eq!(next_retry_backoff_ms(&[1_000, 3_000, 5_000], 6), 5_000);
        assert_eq!(next_retry_backoff_ms(&[], 0), 1_000);
    }

    #[test]
    fn poll_ice_returns_end_of_candidates_batch_without_blocking() {
        let end_of_candidates = vec![IceCandidate {
            candidate: "a=end-of-candidates".to_string(),
            ..Default::default()
        }];

        let result = resolve_polled_ice_result("session-1", Some(end_of_candidates.clone()), true)
            .expect("eoc batch should be returned");

        assert_eq!(result, end_of_candidates);
    }

    #[test]
    fn poll_ice_returns_empty_for_duplicate_snapshot() {
        let result = resolve_polled_ice_result("session-1", None, true)
            .expect("duplicate snapshot should collapse to empty batch");

        assert!(result.is_empty());
    }

    #[test]
    fn connected_standby_retries_wake_after_cooldown() {
        assert!(!should_retry_wake_during_ready_wait(
            Some("ConnectedStandby"),
            Some(10_000),
            14_999,
        ));
        assert!(should_retry_wake_during_ready_wait(
            Some("ConnectedStandby"),
            Some(10_000),
            15_000,
        ));
        assert!(should_retry_wake_during_ready_wait(Some("Off"), None, 0));
        assert!(!should_retry_wake_during_ready_wait(
            Some("On"),
            Some(10_000),
            20_000
        ));
    }

    #[test]
    fn remote_console_ready_requires_registration_signal_after_wake() {
        let console = RemoteConsoleSnapshot {
            power_state: Some("On".to_string()),
            console_streaming_enabled: Some(true),
            ..Default::default()
        };

        assert!(is_remote_console_power_ready(&console));
        assert!(!is_remote_console_ready(&console));
    }

    #[test]
    fn remote_console_ready_signal_reason_prefers_remote_management() {
        let console = RemoteConsoleSnapshot {
            power_state: Some("On".to_string()),
            remote_management_enabled: Some(true),
            console_streaming_enabled: Some(true),
            console_addrs_count: 1,
            ..Default::default()
        };

        assert!(is_remote_console_ready(&console));
        assert_eq!(
            remote_console_ready_reason(&console),
            Some("explicitRegistration")
        );
    }

    #[test]
    fn remote_console_ready_signal_reason_rejects_console_addrs_without_registration() {
        let console = RemoteConsoleSnapshot {
            power_state: Some("On".to_string()),
            console_streaming_enabled: Some(true),
            console_addrs_count: 1,
            ..Default::default()
        };

        assert!(!is_remote_console_ready(&console));
        assert_eq!(remote_console_ready_reason(&console), None);
    }

    #[test]
    fn remote_console_ready_reason_requires_explicit_registration_signal() {
        let console = RemoteConsoleSnapshot {
            power_state: Some("On".to_string()),
            console_streaming_enabled: Some(true),
            ..Default::default()
        };

        assert_eq!(remote_console_ready_reason(&console), None);
    }

    #[test]
    fn remote_console_wake_circuit_open_message_is_detected() {
        let error =
            remote_console_wake_circuit_open_error("console-1", Some("ConnectedStandby"), 3);
        assert!(is_remote_console_wake_circuit_open_message(&error.message));
        assert_eq!(
            error.startup_hint.as_ref().map(|hint| hint.kind.clone()),
            Some(SessionFlowStartupErrorKind::HostRemotePlayUnavailable)
        );
        assert!(!is_remote_console_wake_circuit_open_message(
            "remoteConsoleNotReady:targetId=console-1"
        ));
    }

    #[test]
    fn remote_console_not_ready_error_carries_structured_hint() {
        let error = remote_console_not_ready_error("console-1");

        assert_eq!(
            error.startup_hint.as_ref().map(|hint| hint.kind.clone()),
            Some(SessionFlowStartupErrorKind::ConsoleReady)
        );
        assert!(error
            .startup_hint
            .as_ref()
            .is_some_and(|hint| hint.retryable));
    }

    #[test]
    fn startup_timeout_error_carries_structured_hint() {
        let error = startup_timeout_error("session-1");

        assert_eq!(
            error.startup_hint.as_ref().map(|hint| hint.kind.clone()),
            Some(SessionFlowStartupErrorKind::SessionReady)
        );
        assert!(error
            .startup_hint
            .as_ref()
            .is_some_and(|hint| hint.retryable));
    }

    fn startup_progress(
        phase: SessionPhase,
        error_message: Option<&str>,
    ) -> SessionProgressSnapshot {
        SessionProgressSnapshot {
            session_id: "session-1".to_string(),
            phase,
            status_text_key: "key".to_string(),
            queue_seconds: None,
            queue: None,
            error_code: None,
            error_message: error_message.map(str::to_string),
            error_hint: build_session_progress_error_hint(phase, None, error_message),
        }
    }

    #[test]
    fn failed_progress_server_registration_signal_carries_structured_hint() {
        let progress = SessionProgressSnapshot {
            session_id: "session-1".to_string(),
            phase: SessionPhase::Failed,
            status_text_key: "key".to_string(),
            queue_seconds: None,
            queue: None,
            error_code: Some("ServerNeverRegistered".to_string()),
            error_message: Some(
                "Agent : ServerNeverRegistered : Server never registered with service : State WaitingForServerToRegister"
                    .to_string(),
            ),
            error_hint: build_session_progress_error_hint(
                SessionPhase::Failed,
                Some("ServerNeverRegistered"),
                Some(
                    "Agent : ServerNeverRegistered : Server never registered with service : State WaitingForServerToRegister",
                ),
            ),
        };

        assert_eq!(
            progress.error_hint.as_ref().map(|hint| hint.kind.clone()),
            Some(SessionFlowStartupErrorKind::HostRegistrationRetryExhausted)
        );
        assert!(progress
            .error_hint
            .as_ref()
            .is_some_and(|hint| !hint.retryable));
    }

    #[test]
    fn failed_progress_unknown_error_defaults_to_runtime_hint() {
        let progress = startup_progress(SessionPhase::Failed, Some("decoder pipeline stalled"));
        assert_eq!(
            progress.error_hint.as_ref().map(|hint| hint.kind.clone()),
            Some(SessionFlowStartupErrorKind::Runtime)
        );
        assert!(progress
            .error_hint
            .as_ref()
            .is_some_and(|hint| hint.retryable));
    }

    #[test]
    fn progress_without_error_has_no_structured_hint() {
        let progress = startup_progress(SessionPhase::WaitingSessionReady, None);
        assert_eq!(progress.error_hint, None);
    }

    #[test]
    fn recovering_progress_network_signal_maps_network_hint() {
        let progress = startup_progress(SessionPhase::Recovering, Some("networkLost reconnecting"));
        assert_eq!(
            progress.error_hint.as_ref().map(|hint| hint.kind.clone()),
            Some(SessionFlowStartupErrorKind::Network)
        );
        assert!(progress
            .error_hint
            .as_ref()
            .is_some_and(|hint| hint.retryable));
    }

    #[test]
    fn closed_is_treated_as_transient_when_recovering_signal_is_recent() {
        let mut last_recovery_signal_at_ms = 10_000;
        let action = decide_startup_progress_action(
            &startup_progress(SessionPhase::Closed, None),
            10_900,
            &mut last_recovery_signal_at_ms,
            1_000,
        );
        assert_eq!(
            action,
            StartupProgressAction::Continue {
                transient_closed: true
            }
        );
    }

    #[test]
    fn closed_fails_after_recovery_window_expires() {
        let mut last_recovery_signal_at_ms = 10_000;
        let action = decide_startup_progress_action(
            &startup_progress(SessionPhase::Closed, Some("closed-final")),
            12_001,
            &mut last_recovery_signal_at_ms,
            2_000,
        );
        assert_eq!(
            action,
            StartupProgressAction::Fail("closed-final".to_string())
        );
    }

    #[test]
    fn recovering_phase_refreshes_recovery_signal_timestamp() {
        let mut last_recovery_signal_at_ms = 10_000;
        let action = decide_startup_progress_action(
            &startup_progress(SessionPhase::Recovering, None),
            11_234,
            &mut last_recovery_signal_at_ms,
            2_000,
        );
        assert_eq!(
            action,
            StartupProgressAction::Continue {
                transient_closed: false
            }
        );
        assert_eq!(last_recovery_signal_at_ms, 11_234);
    }

    #[test]
    fn closed_with_reconnect_signal_stays_transient_within_window() {
        let mut last_recovery_signal_at_ms = 1_000;
        let _ = decide_startup_progress_action(
            &startup_progress(
                SessionPhase::WaitingSessionReady,
                Some("networkLost reconnecting"),
            ),
            1_500,
            &mut last_recovery_signal_at_ms,
            900,
        );
        let action = decide_startup_progress_action(
            &startup_progress(SessionPhase::Closed, None),
            2_300,
            &mut last_recovery_signal_at_ms,
            1_000,
        );
        assert_eq!(
            action,
            StartupProgressAction::Continue {
                transient_closed: true
            }
        );
    }

    #[test]
    fn home_provisioning_startup_timeout_no_longer_triggers_recreate() {
        assert_eq!(
            decide_home_session_ready_recreate_retry(
                true,
                0,
                "streamingStartTimeout:sessionId=session-1",
                Some(SessionPhase::WaitingSessionReady),
                Some("Provisioning"),
                None,
                None,
            ),
            None
        );
    }

    #[test]
    fn home_provisioning_stall_timeout_no_longer_triggers_recreate() {
        assert_eq!(
            decide_home_session_ready_recreate_retry(
                true,
                0,
                "homeProvisioningStallTimeout:sessionId=session-1;elapsedMs=10000",
                Some(SessionPhase::WaitingSessionReady),
                Some("Provisioning"),
                None,
                None,
            ),
            None
        );
    }

    #[test]
    fn failed_server_never_registered_error_code_is_exhausted_immediately() {
        assert_eq!(
            decide_home_session_ready_recreate_retry(
                true,
                0,
                "streamingStartFailed",
                Some(SessionPhase::Failed),
                Some("Failed"),
                Some("ServerNeverRegistered"),
                None,
            ),
            Some(SessionReadyRetryDecision::Exhausted(
                SessionReadyRecreateRetryReason::WaitingForServerRegistration,
            ))
        );
    }

    #[test]
    fn failed_server_registration_error_is_exhausted_immediately() {
        assert_eq!(
            decide_home_session_ready_recreate_retry(
                true,
                0,
                "Agent : ServerNeverRegistered : Server never registered with service : State WaitingForServerToRegister",
                Some(SessionPhase::Failed),
                Some("Failed"),
                None,
                Some(
                    "Agent : ServerNeverRegistered : Server never registered with service : State WaitingForServerToRegister",
                ),
            ),
            Some(SessionReadyRetryDecision::Exhausted(
                SessionReadyRecreateRetryReason::WaitingForServerRegistration,
            ))
        );
    }

    #[test]
    fn waiting_for_server_registration_retry_signal_is_exhausted_bounded_retry() {
        assert_eq!(
            decide_home_session_ready_recreate_retry(
                true,
                0,
                "HTTP 500: Xccs : ErrorCallingWNS : Send command failed : State WaitingForServerToRegister",
                Some(SessionPhase::WaitingSessionReady),
                Some("Provisioning"),
                Some("ServerNeverRegistered"),
                Some("ServerNeverRegistered"),
            ),
            Some(SessionReadyRetryDecision::Exhausted(
                SessionReadyRecreateRetryReason::WaitingForServerRegistration,
            ))
        );
    }

    #[test]
    fn cleanup_terminal_state_only_accepts_closed_or_failed() {
        assert!(is_session_cleanup_terminal_state(Some("Closed")));
        assert!(is_session_cleanup_terminal_state(Some("Failed")));
        assert!(!is_session_cleanup_terminal_state(Some("Provisioning")));
        assert!(!is_session_cleanup_terminal_state(Some("ReadyToConnect")));
        assert!(!is_session_cleanup_terminal_state(None));
    }

    #[test]
    fn recreate_reused_session_only_fails_when_cleanup_did_not_settle() {
        assert!(should_fail_home_recreate_same_session(
            false,
            Some("session-1"),
            "session-1",
        ));
        assert!(!should_fail_home_recreate_same_session(
            true,
            Some("session-1"),
            "session-1",
        ));
        assert!(!should_fail_home_recreate_same_session(
            false,
            Some("session-1"),
            "session-2",
        ));
        assert!(!should_fail_home_recreate_same_session(
            false,
            None,
            "session-1",
        ));
    }

    #[test]
    fn non_home_provisioning_timeout_is_not_retryable() {
        assert_eq!(
            decide_home_session_ready_recreate_retry(
                false,
                0,
                "streamingStartTimeout:sessionId=session-1",
                Some(SessionPhase::WaitingSessionReady),
                Some("Provisioning"),
                None,
                None,
            ),
            None
        );
    }

    #[test]
    fn non_provisioning_state_is_not_retryable() {
        assert_eq!(
            decide_home_session_ready_recreate_retry(
                true,
                0,
                "streamingStartTimeout:sessionId=session-1",
                Some(SessionPhase::WaitingSessionReady),
                Some("ReadyToConnect"),
                None,
                None,
            ),
            None
        );
    }

    #[test]
    fn home_waiting_for_server_registration_is_exhausted_in_provisioning() {
        assert_eq!(
            decide_home_session_ready_recreate_retry(
                true,
                0,
                "HTTP 500: Xccs : ErrorCallingWNS : Send command failed : State WaitingForServerToRegister",
                Some(SessionPhase::Failed),
                Some("Provisioning"),
                Some("ServerNeverRegistered"),
                Some("ServerNeverRegistered"),
            ),
            Some(SessionReadyRetryDecision::Exhausted(
                SessionReadyRecreateRetryReason::WaitingForServerRegistration,
            ))
        );
    }

    #[test]
    fn home_server_registration_wait_timeout_only_triggers_after_threshold() {
        let runtime = SessionRuntimeSnapshot {
            stream_state: Some("Provisioning".to_string()),
            player_state: "pending".to_string(),
            queue: None,
            error_details: None,
        };
        let mut wait_started_at_ms = None;

        assert_eq!(
            evaluate_home_server_registration_wait_timeout(
                true,
                "console-1",
                "session-1",
                &runtime,
                10_000,
                &mut wait_started_at_ms,
            ),
            None
        );
        assert_eq!(wait_started_at_ms, Some(10_000));

        let error = evaluate_home_server_registration_wait_timeout(
            true,
            "console-1",
            "session-1",
            &runtime,
            40_000,
            &mut wait_started_at_ms,
        )
        .expect("home provisioning wait should stop after threshold");
        assert!(error.message.contains("homeServerRegistrationTimeout"));
        assert_eq!(
            error.startup_hint.as_ref().map(|hint| hint.kind.clone()),
            Some(SessionFlowStartupErrorKind::HostRegistrationRetryExhausted)
        );
        assert!(error
            .startup_hint
            .as_ref()
            .is_some_and(|hint| !hint.retryable));
    }

    #[test]
    fn home_server_registration_wait_timeout_resets_after_state_progresses() {
        let provisioning_runtime = SessionRuntimeSnapshot {
            stream_state: Some("Provisioning".to_string()),
            player_state: "pending".to_string(),
            queue: None,
            error_details: None,
        };
        let ready_runtime = SessionRuntimeSnapshot {
            stream_state: Some("ReadyToConnect".to_string()),
            player_state: "pending".to_string(),
            queue: None,
            error_details: None,
        };
        let mut wait_started_at_ms = None;

        let _ = evaluate_home_server_registration_wait_timeout(
            true,
            "console-1",
            "session-1",
            &provisioning_runtime,
            10_000,
            &mut wait_started_at_ms,
        );
        assert_eq!(wait_started_at_ms, Some(10_000));

        assert_eq!(
            evaluate_home_server_registration_wait_timeout(
                true,
                "console-1",
                "session-1",
                &ready_runtime,
                12_000,
                &mut wait_started_at_ms,
            ),
            None
        );
        assert_eq!(wait_started_at_ms, None);
    }

    #[test]
    fn home_server_registration_retry_exhausted_is_terminal_host_issue() {
        let error = build_home_session_ready_retry_exhausted_error(
            "console-1",
            SessionReadyRecreateRetryReason::WaitingForServerRegistration,
            1,
            1,
            SessionFlowError::message(
                "Agent : ServerNeverRegistered : Server never registered with service",
            ),
        );

        assert!(error.message.contains("homeSessionBoundedRetryExhausted"));
        assert!(error.message.contains("targetId=console-1"));
        assert!(error
            .message
            .contains("reason=waitingForServerRegistration"));
        assert_eq!(
            error.startup_hint.as_ref().map(|hint| hint.kind.clone()),
            Some(SessionFlowStartupErrorKind::HostRegistrationRetryExhausted)
        );
        assert_eq!(
            error.body.as_deref(),
            Some("Agent : ServerNeverRegistered : Server never registered with service")
        );
    }
}
