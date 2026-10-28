use crate::error::{AppError, AppResult};
use crate::mods::auth::AuthProviderRef;
use crate::mods::config::ConfigProviderRef;
use crate::mods::data::DataHostSummary;
use crate::mods::data::DataProviderRef;
use crate::mods::runtime_trace::RuntimeTraceRecorderRef;
use crate::mods::streaming::events;
use crate::mods::streaming::types::*;
use crate::mods::xbxengine::XbxEngineProviderRef;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use tauri::AppHandle;
use xbox_streaming::input::{
    SupportedInput as DomainSupportedInput, TitleCapabilities as DomainTitleCapabilities,
};
use xbox_streaming::policy::render::SuperResolutionRenderPreference;
use xbox_streaming::policy::{
    session::SessionAccessContext as DomainSessionAccessContext,
    InputCapabilityContext as DomainInputCapabilityContext,
};
use xbox_streaming::runtime::{
    RuntimeCapabilities as DomainRuntimeCapabilities, TurnContext as DomainTurnContext,
};
use xbox_streaming::{
    compile as compile_plan, decide_runtime_recovery, parse_session_access_context,
    project_render_plan, project_runtime_plan, project_session_capabilities,
    project_session_metadata, AnswerPayload as DomainAnswerPayload, AudioChannels,
    BitratePreference, CodecPreference, CompilerInput as DomainCompilerInput,
    Config as DomainStreamingConfig, Context as DomainContext,
    DisplayOptions as DomainDisplayOptions, FallbackTurnProvider, HostAddr as DomainHostAddr,
    IceCandidate as DomainIceCandidate, Plan as StreamingPlan, RemoteConsoleSnapshot, RuntimeFact,
    RuntimeLaunchState as DomainRuntimeLaunchState, RuntimePreference, SessionFlowError,
    SessionFlowProvider, SessionFlowService, SessionFlowStartupErrorHint as DomainStartupErrorHint,
    SessionFlowStartupErrorKind as DomainStartupErrorKind, SessionPhase as DomainSessionPhase,
    SessionProgressSnapshot, SessionStartupBoundedRetryReason as DomainStartupBoundedRetryReason,
    SessionStartupBoundedRetrySnapshot as DomainStartupBoundedRetrySnapshot,
    SessionStartupBoundedRetryStatus as DomainStartupBoundedRetryStatus, SessionStartupObserver,
    SessionStartupPhase as DomainStartupPhase,
    SessionStartupPhaseStatus as DomainStartupPhaseStatus, Target as DomainTarget, TurnServer,
};

#[path = "projection.rs"]
mod projection;
use projection::*;
#[path = "config_policy.rs"]
mod config_policy;
#[path = "context.rs"]
mod context;
use config_policy::*;

const CONSOLE_READY_SMARTGLASS_CACHE_MS: u64 = 2_000;

#[derive(Clone)]
pub struct StreamingService {
    app_handle: AppHandle,
    auth_provider: AuthProviderRef,
    config_provider: ConfigProviderRef,
    data_provider: DataProviderRef,
    xbxengine_provider: XbxEngineProviderRef,
    runtime_trace: RuntimeTraceRecorderRef,
    inner: Arc<StreamingServiceInner>,
}

struct StreamingServiceInner {
    flow: SessionFlowService<StreamingSessionSnapshot, TauriSessionFlowProvider>,
    fallback_turn_provider: tokio::sync::Mutex<FallbackTurnProvider>,
}

/// tauri 侧 flow adapter：仅负责提供凭证。
/// RFC: 策略与执行层已下沉 crate，adapter 彻底退化。
#[derive(Clone)]
struct TauriSessionFlowProvider {
    auth_provider: AuthProviderRef,
    data_provider: DataProviderRef,
    runtime_trace: RuntimeTraceRecorderRef,
    console_ready_smartglass_cache: Arc<tokio::sync::Mutex<Option<CachedHostSnapshot>>>,
}

#[derive(Clone)]
struct CachedHostSnapshot {
    recorded_at: std::time::Instant,
    hosts: Vec<DataHostSummary>,
}

struct StartupAttemptRecorder {
    app_handle: AppHandle,
    runtime_trace: RuntimeTraceRecorderRef,
    attempt_id: String,
    target_type: String,
    target_id: String,
    current_phase: Mutex<StreamingStartupPhase>,
    bounded_retry: Mutex<Option<StreamingStartupBoundedRetry>>,
}

impl StartupAttemptRecorder {
    fn new(
        app_handle: AppHandle,
        runtime_trace: RuntimeTraceRecorderRef,
        attempt_id: String,
        target_type: String,
        target_id: String,
    ) -> Self {
        Self {
            app_handle,
            runtime_trace,
            attempt_id,
            target_type,
            target_id,
            current_phase: Mutex::new(StreamingStartupPhase::ResolvingContext),
            bounded_retry: Mutex::new(None),
        }
    }

    fn emit(
        &self,
        phase: StreamingStartupPhase,
        status: StreamingStartupPhaseStatus,
        summary: impl Into<String>,
        details: Option<String>,
    ) {
        if !matches!(phase, StreamingStartupPhase::Failed) {
            if let Ok(mut current_phase) = self.current_phase.lock() {
                *current_phase = phase.clone();
            }
        }

        let event = StreamingStartupEvent {
            attempt_id: self.attempt_id.clone(),
            target_type: self.target_type.clone(),
            target_id: self.target_id.clone(),
            phase: phase.clone(),
            status: status.clone(),
            summary: summary.into(),
            details: details.clone(),
            bounded_retry: self.current_bounded_retry(),
            ts_ms: now_ms(),
        };
        self.runtime_trace.record_event(
            "streaming",
            "startupPhase",
            None,
            serde_json::json!({
                "attemptId": event.attempt_id,
                "targetType": event.target_type,
                "targetId": event.target_id,
                "phase": event.phase,
                "status": event.status,
                "summary": event.summary,
                "details": event.details,
                "boundedRetry": event.bounded_retry,
                "tsMs": event.ts_ms,
            }),
        );
        let _ = events::emit_startup_event(&self.app_handle, &event);
    }

    fn current_phase(&self) -> StreamingStartupPhase {
        self.current_phase
            .lock()
            .map(|phase| phase.clone())
            .unwrap_or(StreamingStartupPhase::Failed)
    }

    fn current_bounded_retry(&self) -> Option<StreamingStartupBoundedRetry> {
        self.bounded_retry
            .lock()
            .ok()
            .and_then(|state| state.clone())
    }

    fn set_bounded_retry(&self, bounded_retry: Option<StreamingStartupBoundedRetry>) {
        if let Ok(mut state) = self.bounded_retry.lock() {
            *state = bounded_retry;
        }
    }

    fn build_startup_error(&self, error: &SessionFlowError) -> StreamingStartupError {
        let phase = self.current_phase();
        let bounded_retry = self.current_bounded_retry();
        let error_kind = error
            .startup_hint
            .as_ref()
            .map(map_domain_startup_error_kind)
            .unwrap_or_else(|| classify_startup_error_kind_fallback(&phase, error));
        let user_message_key = startup_error_message_key(&error_kind);
        let diagnostic_summary = error
            .startup_hint
            .as_ref()
            .map(|hint| hint.diagnostic_summary.clone())
            .unwrap_or_else(|| build_startup_diagnostic_summary_fallback(&phase, error));
        let retryable = error
            .startup_hint
            .as_ref()
            .map(|hint| hint.retryable)
            .unwrap_or_else(|| is_startup_error_retryable_fallback(&error_kind, error));
        StreamingStartupError {
            attempt_id: self.attempt_id.clone(),
            phase: phase.clone(),
            error_kind: error_kind.clone(),
            user_message_key: user_message_key.to_string(),
            diagnostic_summary,
            raw_message: error.message.clone(),
            retryable,
            bounded_retry,
        }
    }
}

impl TauriSessionFlowProvider {
    async fn load_hosts_for_console_ready_trace(&self) -> Vec<DataHostSummary> {
        {
            let cache = self.console_ready_smartglass_cache.lock().await;
            if let Some(cache) = cache.as_ref() {
                if cache.recorded_at.elapsed().as_millis()
                    < CONSOLE_READY_SMARTGLASS_CACHE_MS as u128
                {
                    return cache.hosts.clone();
                }
            }
        }

        let hosts = self.data_provider.get_hosts().await.unwrap_or_default();
        let mut cache = self.console_ready_smartglass_cache.lock().await;
        *cache = Some(CachedHostSnapshot {
            recorded_at: std::time::Instant::now(),
            hosts: hosts.clone(),
        });
        hosts
    }
}

impl SessionStartupObserver for StartupAttemptRecorder {
    fn on_phase_event(
        &self,
        phase: DomainStartupPhase,
        status: DomainStartupPhaseStatus,
        details: Option<&str>,
    ) {
        let phase = map_startup_phase(phase);
        let status = map_startup_phase_status(status);
        self.emit(
            phase.clone(),
            status,
            startup_phase_summary(&phase, details),
            details.map(str::to_string),
        );
    }

    fn on_bounded_retry(
        &self,
        phase: DomainStartupPhase,
        bounded_retry: &DomainStartupBoundedRetrySnapshot,
    ) {
        let phase = map_startup_phase(phase);
        let bounded_retry = map_startup_bounded_retry(bounded_retry);
        self.set_bounded_retry(Some(bounded_retry.clone()));
        self.emit(
            phase.clone(),
            StreamingStartupPhaseStatus::Entered,
            startup_bounded_retry_summary(&phase, &bounded_retry),
            Some(build_startup_bounded_retry_details(&bounded_retry)),
        );
    }
}

#[async_trait::async_trait]
impl SessionFlowProvider for TauriSessionFlowProvider {
    fn on_session_lifecycle_event(
        &self,
        event: &str,
        session_id: Option<&str>,
        payload: serde_json::Value,
    ) {
        self.runtime_trace
            .record_event("streaming", event, session_id, payload);
    }

    async fn get_streaming_token(
        &self,
        target_type: &str,
    ) -> Result<serde_json::Value, SessionFlowError> {
        self.auth_provider
            .get_streaming_token(target_type)
            .map_err(|e| SessionFlowError::message(e.to_string()))?
            .ok_or_else(|| SessionFlowError::message("token missing"))
    }

    async fn transfer_token(&self) -> Result<String, SessionFlowError> {
        self.auth_provider
            .get_transfer_token()
            .await
            .map_err(|error| SessionFlowError::message(error.to_string()))
    }

    async fn power_on_console(&self, console_id: &str) -> Result<bool, SessionFlowError> {
        let result = self
            .data_provider
            .power_on_console(console_id)
            .await
            .map_err(|error| {
                self.runtime_trace.record_event(
                    "streaming",
                    "powerOnConsoleFailed",
                    None,
                    serde_json::json!({
                        "consoleId": console_id,
                        "error": error,
                    }),
                );
                SessionFlowError::message(error)
            })?;
        self.runtime_trace.record_event(
            "streaming",
            "powerOnConsoleResult",
            None,
            serde_json::json!({
                "consoleId": console_id,
                "accepted": result.accepted,
            }),
        );
        Ok(result.accepted)
    }

    async fn get_remote_consoles(&self) -> Result<Vec<RemoteConsoleSnapshot>, SessionFlowError> {
        let smartglass_hosts = self.load_hosts_for_console_ready_trace().await;
        let smartglass_ready_consoles = build_smartglass_ready_candidates(&smartglass_hosts);
        self.runtime_trace.record_snapshot(
            "streaming",
            "smartglassConsolesSnapshot",
            None,
            serde_json::json!({
                "count": smartglass_hosts.len(),
                "consoles": smartglass_hosts.iter().map(|console| {
                    serde_json::json!({
                        "id": console.id,
                        "deviceId": console.device_id,
                        "serverId": console.server_id,
                        "powerState": console.power_state,
                        "remoteManagementEnabled": console.remote_management_enabled,
                        "consoleStreamingEnabled": console.console_streaming_enabled,
                        "consoleAddrsCount": console.console_addrs.as_ref().map(|items| items.len()).unwrap_or(0),
                    })
                }).collect::<Vec<_>>(),
            }),
        );
        self.runtime_trace.record_snapshot(
            "streaming",
            "consoleReadySnapshot",
            None,
            build_console_ready_snapshot(&smartglass_hosts, &smartglass_ready_consoles),
        );
        Ok(smartglass_ready_consoles)
    }

    fn on_session_state_polled(
        &self,
        session_id: &str,
        target_type: &str,
        target_id: &str,
        state: Option<&str>,
        error_code: Option<&serde_json::Value>,
        error_message: Option<&str>,
    ) {
        self.runtime_trace.record_event(
            "streaming",
            "sessionStatePolled",
            None,
            serde_json::json!({
                "sessionId": session_id,
                "targetType": target_type,
                "targetId": target_id,
                "state": state,
                "errorCode": error_code,
                "errorMessage": error_message,
                "tsMs": now_ms(),
            }),
        );
    }

    fn on_session_state_poll_failed(
        &self,
        session_id: &str,
        target_type: &str,
        target_id: &str,
        error: &SessionFlowError,
    ) {
        self.runtime_trace.record_event(
            "streaming",
            "sessionStatePollFailed",
            None,
            serde_json::json!({
                "sessionId": session_id,
                "targetType": target_type,
                "targetId": target_id,
                "status": error.status,
                "message": error.message,
                "body": error.body,
                "tsMs": now_ms(),
            }),
        );
    }

    fn on_session_monitor_tick(
        &self,
        session_id: &str,
        target_type: &str,
        target_id: &str,
        progress: &SessionProgressSnapshot,
        stream_state: Option<&str>,
        player_state: &str,
        should_continue: bool,
        should_send_connect_token: bool,
    ) {
        self.runtime_trace.record_snapshot(
            "streaming",
            "sessionMonitorSnapshot",
            None,
            serde_json::json!({
                "sessionId": session_id,
                "targetType": target_type,
                "targetId": target_id,
                "phase": progress.phase,
                "runtimeLaunchState": progress.runtime_launch_state,
                "statusTextKey": progress.status_text_key,
                "streamState": stream_state,
                "playerState": player_state,
                "queueSeconds": progress.queue_seconds,
                "errorCode": progress.error_code,
                "errorMessage": progress.error_message,
                "shouldContinue": should_continue,
                "shouldSendConnectToken": should_send_connect_token,
                "tsMs": now_ms(),
            }),
        );
    }

    fn on_session_connect_token_result(
        &self,
        session_id: &str,
        target_type: &str,
        target_id: &str,
        status: &str,
        error: Option<&SessionFlowError>,
    ) {
        self.runtime_trace.record_event(
            "streaming",
            "sessionConnectToken",
            None,
            serde_json::json!({
                "sessionId": session_id,
                "targetType": target_type,
                "targetId": target_id,
                "status": status,
                "errorStatus": error.and_then(|value| value.status),
                "errorMessage": error.map(|value| value.message.clone()),
                "errorBody": error.and_then(|value| value.body.clone()),
                "tsMs": now_ms(),
            }),
        );
    }

    fn on_session_created(
        &self,
        session_id: &str,
        session_path: &str,
        target_type: &str,
        target_id: &str,
        recreate_from_session_id: Option<&str>,
    ) {
        self.runtime_trace.record_event(
            "streaming",
            "sessionCreated",
            Some(session_id),
            serde_json::json!({
                "sessionId": session_id,
                "sessionPath": session_path,
                "targetType": target_type,
                "targetId": target_id,
                "recreateFromSessionId": recreate_from_session_id,
                "reusedSessionId": recreate_from_session_id == Some(session_id),
                "tsMs": now_ms(),
            }),
        );
    }

    fn on_session_recreate_cleanup_result(
        &self,
        session_id: &str,
        target_type: &str,
        target_id: &str,
        status: &str,
        last_state: Option<&str>,
        error: Option<&SessionFlowError>,
    ) {
        self.runtime_trace.record_event(
            "streaming",
            "sessionRecreateCleanup",
            Some(session_id),
            serde_json::json!({
                "sessionId": session_id,
                "targetType": target_type,
                "targetId": target_id,
                "status": status,
                "lastState": last_state,
                "errorStatus": error.and_then(|value| value.status),
                "errorMessage": error.map(|value| value.message.clone()),
                "errorBody": error.and_then(|value| value.body.clone()),
                "tsMs": now_ms(),
            }),
        );
    }

    fn on_console_ready_wait_result(
        &self,
        target_type: &str,
        target_id: &str,
        status: &str,
        reason: &str,
        console: Option<&RemoteConsoleSnapshot>,
    ) {
        self.runtime_trace.record_event(
            "streaming",
            "consoleReadyWaitResult",
            None,
            serde_json::json!({
                "targetType": target_type,
                "targetId": target_id,
                "status": status,
                "reason": reason,
                "powerState": console.and_then(|value| value.power_state.clone()),
                "remoteManagementEnabled": console.and_then(|value| value.remote_management_enabled),
                "consoleStreamingEnabled": console.and_then(|value| value.console_streaming_enabled),
                "consoleAddrsCount": console.map(|value| value.console_addrs_count),
                "readySource": console.and_then(|value| value.ready_source.clone()),
                "tsMs": now_ms(),
            }),
        );
    }
}

impl StreamingService {
    pub fn new(
        app_handle: AppHandle,
        auth_provider: AuthProviderRef,
        config_provider: ConfigProviderRef,
        data_provider: DataProviderRef,
        xbxengine_provider: XbxEngineProviderRef,
        runtime_trace: RuntimeTraceRecorderRef,
    ) -> Self {
        let flow_provider = TauriSessionFlowProvider {
            auth_provider: auth_provider.clone(),
            data_provider: data_provider.clone(),
            runtime_trace: runtime_trace.clone(),
            console_ready_smartglass_cache: Arc::new(tokio::sync::Mutex::new(None)),
        };

        Self {
            app_handle,
            auth_provider,
            config_provider,
            data_provider,
            xbxengine_provider,
            runtime_trace,
            inner: Arc::new(StreamingServiceInner {
                flow: SessionFlowService::new(flow_provider),
                fallback_turn_provider: tokio::sync::Mutex::new(FallbackTurnProvider::new()),
            }),
        }
    }
}

#[async_trait::async_trait]
impl crate::mods::streaming::StreamingProvider for StreamingService {
    async fn start_session(
        &self,
        params: StreamingStartSessionParams,
    ) -> AppResult<StreamingStartSessionResult> {
        let attempt_id = params.attempt_id.clone();
        let scope = self
            .inner
            .flow
            .begin_startup(&attempt_id, &params.target_type, &params.target_id)
            .map_err(map_flow_error)?;
        let operation_id = format!("startup-{}", scope.generation());
        let started_at = std::time::Instant::now();
        self.runtime_trace.record_event(
            "streaming",
            "runtimeBuildInfo",
            None,
            serde_json::json!({
                "buildFingerprint": crate::mods::xbxengine::build_info::current_build_fingerprint(),
                "platform": std::env::consts::OS,
                "targetType": params.target_type,
            }),
        );
        self.runtime_trace.record_event("streaming", "operationStarted", None, serde_json::json!({
            "operationId": operation_id, "attemptId": attempt_id, "stage": "startup", "generation": scope.generation(),
        }));
        let result = async {
            let startup = StartupAttemptRecorder::new(
                self.app_handle.clone(),
                self.runtime_trace.clone(),
                params.attempt_id.clone(),
                params.target_type.clone(),
                params.target_id.clone(),
            );
            startup.emit(
                StreamingStartupPhase::ResolvingContext,
                StreamingStartupPhaseStatus::Entered,
                startup_phase_summary(&StreamingStartupPhase::ResolvingContext, None),
                None,
            );
            let (plan, compiler_context) = scope
                .run(
                    "resolvingContext",
                    Some(std::time::Duration::from_secs(90)),
                    async {
                        self.resolve_streaming_plan(&params.target_type, &params.target_id)
                            .await
                            .map_err(|error| SessionFlowError::message(error.to_string()))
                    },
                )
                .await
                .map_err(|error| {
                    let flow_error = SessionFlowError::message(error.to_string());
                    let startup_error = startup.build_startup_error(&flow_error);
                    startup.emit(
                        StreamingStartupPhase::Failed,
                        StreamingStartupPhaseStatus::Failed,
                        startup_error.diagnostic_summary.clone(),
                        Some(startup_error.raw_message.clone()),
                    );
                    AppError::streaming_detailed(
                        error.to_string(),
                        serde_json::to_value(&startup_error).unwrap_or(serde_json::Value::Null),
                    )
                })?;
            startup.emit(
                StreamingStartupPhase::ResolvingContext,
                StreamingStartupPhaseStatus::Succeeded,
                startup_phase_summary(
                    &StreamingStartupPhase::ResolvingContext,
                    Some("contextResolved"),
                ),
                Some("contextResolved".to_string()),
            );
            // home 启动期不再让 service 层 preflight 抢先失败，
            // 统一交给 flow 内已有的 wake/ready/create 重试主链裁决。
            // 执行快照会消费 plan，这里先投影出页面侧需要的稳定元数据。
            let metadata = project_session_metadata(&plan);
            let capabilities = project_session_capabilities(&compiler_context, &plan);
            let execution = match self
                .inner
                .flow
                .start_session_execution_with_scope(
                    plan,
                    project_runtime_plan,
                    project_render_plan,
                    Some(&startup),
                    &scope,
                )
                .await
            {
                Ok(execution) => execution,
                Err(error) => {
                    let startup_error = startup.build_startup_error(&error);
                    startup.emit(
                        StreamingStartupPhase::Failed,
                        StreamingStartupPhaseStatus::Failed,
                        startup_error.diagnostic_summary.clone(),
                        Some(startup_error.raw_message.clone()),
                    );
                    self.runtime_trace.record_event(
                        "streaming",
                        "sessionStartFailed",
                        None,
                        serde_json::json!({
                            "attemptId": params.attempt_id,
                            "targetType": params.target_type,
                            "targetId": params.target_id,
                            "startupError": startup_error,
                            "error": {
                                "message": error.message,
                                "status": error.status,
                                "body": error.body,
                            },
                        }),
                    );
                    return Err(AppError::streaming_detailed(
                        error.to_string(),
                        serde_json::to_value(&startup_error).unwrap_or(serde_json::Value::Null),
                    ));
                }
            };
            startup.emit(
                StreamingStartupPhase::StartingRuntime,
                StreamingStartupPhaseStatus::Entered,
                startup_phase_summary(&StreamingStartupPhase::StartingRuntime, None),
                Some(execution.session.id.clone()),
            );
            let mut runtime: crate::mods::streaming::types::StreamingRuntimeProjection =
                execution.runtime.into();
            runtime.vibration_strength = normalize_vibration_strength(
                &self
                    .config_provider
                    .get_streaming_config()
                    .vibration_strength,
            );
            let execution = StreamingSessionExecutionSnapshot {
                session: execution.session,
                runtime,
                render: execution.render.into(),
                metadata: metadata.into(),
                capabilities: capabilities.into(),
            };
            self.runtime_trace.record_state(
                "streaming",
                "sessionExecutionStarted",
                Some(&execution.session.id),
                &execution,
            );
            startup.emit(
                StreamingStartupPhase::Ready,
                StreamingStartupPhaseStatus::Succeeded,
                startup_phase_summary(&StreamingStartupPhase::Ready, None),
                Some(execution.session.id.clone()),
            );

            let progress = self
                .inner
                .flow
                .get_session_progress(&execution.session.id)
                .await
                .map(map_domain_progress_snapshot)
                .unwrap_or_else(|| {
                    build_fallback_progress_snapshot(
                        StreamingSessionProgressSnapshot::from_session_snapshot(&execution.session),
                    )
                });
            self.runtime_trace.record_state(
                "streaming",
                "sessionProgress",
                Some(&execution.session.id),
                &progress,
            );

            Ok(StreamingStartSessionResult {
                attempt_id: params.attempt_id,
                execution,
                progress,
            })
        }
        .await;
        self.inner.flow.finish_startup(&attempt_id);
        let error_message = result
            .as_ref()
            .err()
            .map(ToString::to_string)
            .unwrap_or_default();
        let event = if error_message.contains("operationTimeout") {
            "operationTimedOut"
        } else if error_message.contains("operationCancelled") {
            "operationCancelled"
        } else {
            "operationFinished"
        };
        self.runtime_trace.record_event("streaming", event, result.as_ref().ok().map(|started| started.execution.session.id.as_str()), serde_json::json!({
            "operationId": operation_id, "attemptId": attempt_id, "stage": "startup", "generation": scope.generation(),
            "result": if result.is_ok() { "success" } else { "failed" }, "elapsedMs": started_at.elapsed().as_millis() as u64,
        }));
        result
    }

    async fn cancel_startup(
        &self,
        params: StreamingCancelStartupParams,
    ) -> AppResult<StreamingCancelStartupResult> {
        self.inner
            .flow
            .cancel_startup(&params.attempt_id)
            .await
            .map_err(map_flow_error)?;
        self.runtime_trace.record_event(
            "streaming",
            "startupCancellationRequested",
            None,
            serde_json::json!({ "attemptId": params.attempt_id, "stage": "startup" }),
        );
        Ok(StreamingCancelStartupResult { cancelled: true })
    }

    async fn get_session_progress(
        &self,
        params: StreamingGetSessionProgressParams,
    ) -> AppResult<Option<StreamingSessionProgressSnapshot>> {
        Ok(self
            .inner
            .flow
            .get_session_progress(&params.session_id)
            .await
            .map(map_domain_progress_snapshot))
    }

    async fn close_session(
        &self,
        params: StreamingCloseSessionParams,
    ) -> AppResult<StreamingCloseSessionResult> {
        self.runtime_trace.record_event(
            "streaming",
            "closeSessionRequested",
            Some(&params.session_id),
            &params,
        );
        let closed = self
            .inner
            .flow
            .close_session(&params.session_id)
            .await
            .map_err(map_flow_error)?;
        self.runtime_trace.record_event(
            "streaming",
            "closeSessionResult",
            Some(&params.session_id),
            serde_json::json!({ "closed": closed }),
        );

        Ok(StreamingCloseSessionResult { closed })
    }

    async fn send_keepalive(&self, session_id: String) -> AppResult<bool> {
        self.runtime_trace.record_event(
            "streaming",
            "keepaliveRequested",
            Some(&session_id),
            serde_json::json!({}),
        );
        let result = self
            .inner
            .flow
            .send_keepalive(&session_id)
            .await
            .map_err(map_flow_error)?;
        self.runtime_trace.record_event(
            "streaming",
            "keepaliveResult",
            Some(&session_id),
            serde_json::json!({ "accepted": result }),
        );
        Ok(result)
    }

    async fn exchange_offer(
        &self,
        params: StreamingExchangeOfferParams,
    ) -> AppResult<StreamingExchangeOfferResult> {
        self.runtime_trace.record_event(
            "streaming",
            "exchangeOfferRequested",
            Some(&params.session_id),
            &params,
        );
        let answer = self
            .inner
            .flow
            .exchange_offer(
                &params.session_id,
                params.channel.as_deref(),
                &params.sdp,
                params.restart,
            )
            .await
            .map_err(map_flow_error)?;
        self.runtime_trace.record_event(
            "streaming",
            "exchangeOfferResult",
            Some(&params.session_id),
            serde_json::json!({
                "channel": params.channel,
                "restart": params.restart,
                "answer": from_domain_answer_payload(answer.clone()),
            }),
        );

        Ok(StreamingExchangeOfferResult {
            answer: from_domain_answer_payload(answer),
        })
    }

    async fn submit_ice(
        &self,
        params: StreamingSubmitIceParams,
    ) -> AppResult<StreamingSubmitIceResult> {
        self.runtime_trace.record_event(
            "streaming",
            "submitIceRequested",
            Some(&params.session_id),
            &params,
        );
        let local_candidates = params
            .candidate
            .iter()
            .map(to_domain_ice_candidate)
            .collect::<Vec<_>>();
        self.inner
            .flow
            .submit_ice(&params.session_id, &local_candidates, params.restart)
            .await
            .map_err(map_flow_error)?;
        self.runtime_trace.record_event(
            "streaming",
            "submitIceResult",
            Some(&params.session_id),
            serde_json::json!({ "accepted": true, "restart": params.restart }),
        );
        Ok(StreamingSubmitIceResult { accepted: true })
    }

    async fn poll_ice(&self, params: StreamingPollIceParams) -> AppResult<StreamingPollIceResult> {
        self.runtime_trace.record_event(
            "streaming",
            "pollIceRequested",
            Some(&params.session_id),
            &params,
        );
        let remote_candidates = self
            .inner
            .flow
            .poll_ice(&params.session_id, params.restart)
            .await
            .map_err(map_flow_error)?;
        self.runtime_trace.record_event(
            "streaming",
            "pollIceResult",
            Some(&params.session_id),
            serde_json::json!({
                "restart": params.restart,
                "candidates": remote_candidates,
            }),
        );

        Ok(StreamingPollIceResult {
            candidates: remote_candidates
                .into_iter()
                .map(from_domain_ice_candidate)
                .collect(),
        })
    }

    async fn list_active_sessions(
        &self,
        params: StreamingListActiveSessionsParams,
    ) -> AppResult<StreamingListActiveSessionsResult> {
        let result = self
            .inner
            .flow
            .list_active_sessions(params.target_type)
            .await;
        if result.used_default_target_type {
            log::warn!(
                "list_active_sessions: target_type missing, using default: {}",
                result.target_type
            );
        }

        Ok(StreamingListActiveSessionsResult {
            sessions: result.sessions,
        })
    }

    async fn decide_recovery(
        &self,
        params: StreamingDecideRecoveryParams,
    ) -> AppResult<StreamingDecideRecoveryResult> {
        let fact_value = serde_json::to_value(&params.fact).unwrap_or(serde_json::Value::Null);
        // 恢复判定统一在 crate session 内完成，tauri 仅做运行事实映射。
        let reason = match params.fact {
            StreamingRuntimeFact::TransportConnectionState { connection_state } => {
                decide_runtime_recovery(
                    RuntimeFact::TransportConnectionState(connection_state.as_str()),
                    params.is_closing,
                )
            }
            StreamingRuntimeFact::MediaHealth {
                connection_state,
                connected_elapsed_ms,
                inactivity_elapsed_ms,
            } => decide_runtime_recovery(
                RuntimeFact::MediaHealth {
                    connection_state: connection_state.as_str(),
                    connected_elapsed_ms,
                    inactivity_elapsed_ms,
                },
                params.is_closing,
            ),
            StreamingRuntimeFact::MediaStalled => {
                decide_runtime_recovery(RuntimeFact::MediaStalled, params.is_closing)
            }
        };
        let current_session = self
            .inner
            .flow
            .session_allows_recovery(&params.session_id)
            .await;
        let reason = reason
            .filter(|_| current_session)
            .map(|value| value.as_str().to_string());
        self.runtime_trace.record_decision(
            "streaming",
            "decideRecovery",
            Some(&params.session_id),
            serde_json::json!({
                "fact": fact_value,
                "isClosing": params.is_closing,
                "currentSession": current_session,
                "reason": reason,
            }),
        );
        Ok(StreamingDecideRecoveryResult {
            should_reconnect: reason.is_some(),
            reason,
        })
    }

    async fn shutdown(&self) {
        self.inner.flow.shutdown().await;
    }
}

fn map_flow_error(error: SessionFlowError) -> AppError {
    AppError::Streaming(error.to_string())
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn normalize_vibration_strength(value: &str) -> String {
    match value.trim() {
        "enhanced" => "enhanced".to_string(),
        "full" => "full".to_string(),
        _ => "realistic".to_string(),
    }
}

fn build_console_ready_snapshot(
    smartglass_hosts: &[DataHostSummary],
    smartglass_ready_consoles: &[RemoteConsoleSnapshot],
) -> serde_json::Value {
    serde_json::json!({
        "smartglassCount": smartglass_hosts.len(),
        "smartglassReadyCount": smartglass_ready_consoles.len(),
        "smartglassHosts": smartglass_hosts.iter().map(summarize_host_for_console_ready_trace).collect::<Vec<_>>(),
        "smartglassReadyConsoles": smartglass_ready_consoles
            .iter()
            .map(summarize_remote_console_for_ready_trace)
            .collect::<Vec<_>>(),
        "tsMs": now_ms(),
    })
}

fn build_smartglass_ready_candidates(
    smartglass_hosts: &[DataHostSummary],
) -> Vec<RemoteConsoleSnapshot> {
    let smartglass_index = build_host_index(smartglass_hosts);

    smartglass_index
        .into_iter()
        .map(|(identity, smartglass)| build_smartglass_ready_candidate(identity, smartglass))
        .collect()
}

fn build_host_index<'a>(hosts: &'a [DataHostSummary]) -> BTreeMap<String, &'a DataHostSummary> {
    let mut index = BTreeMap::new();
    for host in hosts {
        if let Some(identity) = host_identity(host) {
            index.entry(identity).or_insert(host);
        }
    }
    index
}

fn host_identity(host: &DataHostSummary) -> Option<String> {
    host.server_id
        .clone()
        .or_else(|| host.id.clone())
        .or_else(|| host.device_id.clone())
}

fn build_smartglass_ready_candidate(
    identity: String,
    smartglass: &DataHostSummary,
) -> RemoteConsoleSnapshot {
    RemoteConsoleSnapshot {
        id: smartglass.id.clone().or_else(|| Some(identity.clone())),
        device_id: smartglass.device_id.clone(),
        server_id: smartglass
            .server_id
            .clone()
            .or_else(|| smartglass.id.clone())
            .or_else(|| Some(identity)),
        power_state: smartglass.power_state.clone(),
        remote_management_enabled: smartglass.remote_management_enabled,
        console_streaming_enabled: smartglass.console_streaming_enabled,
        console_addrs_count: console_addrs_count(smartglass),
        ready_source: Some("smartglass".to_string()),
    }
}

fn console_addrs_count(host: &DataHostSummary) -> u32 {
    host.console_addrs
        .as_ref()
        .map(|items| items.len() as u32)
        .unwrap_or(0)
}

fn summarize_host_for_console_ready_trace(host: &DataHostSummary) -> serde_json::Value {
    serde_json::json!({
        "id": host.id,
        "deviceId": host.device_id,
        "serverId": host.server_id,
        "name": host.name,
        "deviceName": host.device_name,
        "powerState": host.power_state,
        "remoteManagementEnabled": host.remote_management_enabled,
        "consoleStreamingEnabled": host.console_streaming_enabled,
        "consoleAddrsCount": console_addrs_count(host),
    })
}

fn summarize_remote_console_for_ready_trace(console: &RemoteConsoleSnapshot) -> serde_json::Value {
    serde_json::json!({
        "id": console.id,
        "deviceId": console.device_id,
        "serverId": console.server_id,
        "powerState": console.power_state,
        "remoteManagementEnabled": console.remote_management_enabled,
        "consoleStreamingEnabled": console.console_streaming_enabled,
        "consoleAddrsCount": console.console_addrs_count,
        "readySource": console.ready_source,
    })
}

#[cfg(test)]
#[path = "service.test.rs"]
mod tests;
