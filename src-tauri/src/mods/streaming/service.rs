use crate::error::{AppError, AppResult};
use crate::mods::auth::AuthProviderRef;
use crate::mods::config::ConfigProviderRef;
use crate::mods::data::DataProviderRef;
use crate::mods::runtime_trace::RuntimeTraceRecorderRef;
use crate::mods::streaming::events;
use crate::mods::streaming::types::*;
use crate::mods::xbxengine::XbxEngineProviderRef;
use std::sync::{Arc, Mutex};
use tauri::AppHandle;
use xbox_streaming::input::{
    SupportedInput as DomainSupportedInput, TitleCapabilities as DomainTitleCapabilities,
};
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
    RuntimePreference, SessionFlowError, SessionFlowProvider, SessionFlowService,
    SessionProgressSnapshot, SessionStartupObserver, SessionStartupPhase as DomainStartupPhase,
    SessionStartupPhaseStatus as DomainStartupPhaseStatus, Target as DomainTarget, TurnServer,
};

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

#[derive(Default)]
struct ResolvedTitleCapabilitySnapshot {
    capabilities: DomainTitleCapabilities,
    input_capability: DomainInputCapabilityContext,
}

#[derive(Default)]
struct ResolvedRemotePlayContext {
    configuration_resolved: bool,
    remote_management_enabled: Option<bool>,
    console_streaming_enabled: Option<bool>,
    console_addrs: Vec<DomainHostAddr>,
}

const HOME_REMOTE_PLAY_PREFLIGHT_TIMEOUT_MS: u64 = 8_000;
const HOME_REMOTE_PLAY_PREFLIGHT_POLL_INTERVAL_MS: u64 = 1_000;

/// tauri 侧 flow adapter：仅负责提供凭证。
/// RFC: 策略与执行层已下沉 crate，adapter 彻底退化。
#[derive(Clone)]
struct TauriSessionFlowProvider {
    auth_provider: AuthProviderRef,
    data_provider: DataProviderRef,
    runtime_trace: RuntimeTraceRecorderRef,
}

struct StartupAttemptRecorder {
    app_handle: AppHandle,
    runtime_trace: RuntimeTraceRecorderRef,
    attempt_id: String,
    target_type: String,
    target_id: String,
    current_phase: Mutex<StreamingStartupPhase>,
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

    fn build_startup_error(&self, error: &SessionFlowError) -> StreamingStartupError {
        let phase = self.current_phase();
        let error_kind = classify_startup_error_kind(&phase, error);
        StreamingStartupError {
            attempt_id: self.attempt_id.clone(),
            phase: phase.clone(),
            error_kind: error_kind.clone(),
            user_message_key: startup_error_message_key(&error_kind, error).to_string(),
            diagnostic_summary: build_startup_diagnostic_summary(&phase, error),
            raw_message: error.message.clone(),
            retryable: is_startup_error_retryable(&error_kind, error),
        }
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
}

#[async_trait::async_trait]
impl SessionFlowProvider for TauriSessionFlowProvider {
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
        let consoles = self
            .data_provider
            .get_remote_consoles()
            .await
            .map_err(|error| {
                self.runtime_trace.record_event(
                    "streaming",
                    "remoteConsolesQueryFailed",
                    None,
                    serde_json::json!({
                        "error": error,
                    }),
                );
                SessionFlowError::message(error)
            })?;
        self.runtime_trace.record_snapshot(
            "streaming",
            "remoteConsolesSnapshot",
            None,
            serde_json::json!({
                "count": consoles.len(),
                "consoles": consoles.iter().map(|console| {
                    serde_json::json!({
                        "id": console.id,
                        "deviceId": console.device_id,
                        "serverId": console.server_id,
                        "powerState": console.power_state,
                        "consoleStreamingEnabled": console.console_streaming_enabled,
                    })
                }).collect::<Vec<_>>(),
            }),
        );
        Ok(consoles
            .into_iter()
            .map(|console| RemoteConsoleSnapshot {
                id: console.id,
                device_id: console.device_id,
                server_id: console.server_id,
                power_state: console.power_state,
                console_streaming_enabled: console.console_streaming_enabled,
            })
            .collect())
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

    async fn resolve_streaming_plan(
        &self,
        target_type: &str,
        target_id: &str,
    ) -> AppResult<(StreamingPlan, DomainContext)> {
        let target_type = StreamingTargetType::from_value(target_type);
        let target = if matches!(target_type, StreamingTargetType::Home) {
            DomainTarget::Home
        } else {
            DomainTarget::Cloud
        };

        let config_snapshot = self.config_provider.get_streaming_config();
        self.runtime_trace.record_snapshot(
            "streaming",
            "configSnapshot",
            None,
            serde_json::json!({
                "targetType": target_type.as_str(),
                "targetId": target_id,
                "config": config_snapshot,
            }),
        );

        let mut domain_config = DomainStreamingConfig::default();
        // RFC: 映射完整性。Facade 仅负责搬运原始值，不负责解释分辨率等字段语义。
        domain_config.update_from_raw_values(
            Some(config_snapshot.preferred_game_language.clone()),
            normalize_optional(&config_snapshot.force_region_ip),
            config_snapshot.ipv6,
            config_snapshot.resolution,
        );
        // xHome 分辨率独立于全局 resolution，避免本地主机串流被云串流默认值拖成 720p。
        domain_config.session.home_resolution =
            xbox_streaming::policy::config::parse_resolution_preference(
                config_snapshot.xhome_resolution,
            );
        apply_streaming_preferences(&mut domain_config, &config_snapshot);

        let token = self
            .auth_provider
            .get_streaming_token(target_type.as_str())
            .map_err(|error| AppError::Streaming(error.to_string()))?
            .ok_or_else(|| {
                AppError::Streaming(format!(
                    "Streaming token is unavailable for {}",
                    target_type.as_str()
                ))
            })?;

        let access_context = parse_session_access_context(&token)
            .map_err(|error| AppError::Streaming(error.to_string()))?;

        let context = self
            .build_domain_context(target, target_id, target_type.as_str(), access_context)
            .await;
        self.runtime_trace.record_snapshot(
            "streaming",
            "contextSnapshot",
            None,
            serde_json::json!({
                "targetType": target_type.as_str(),
                "targetId": target_id,
                "runtime": {
                    "browserWebrtc": context.runtime.browser_webrtc,
                    "rustOwned": context.runtime.rust_owned,
                    "preferBrowser": context.runtime.prefer_browser,
                },
                "remotePlay": {
                    "configurationResolved": context.remote_play.configuration_resolved,
                    "remoteManagementEnabled": context.remote_play.remote_management_enabled,
                    "consoleStreamingEnabled": context.remote_play.console_streaming_enabled,
                    "consoleAddrsCount": context.remote_play.console_addrs.len(),
                },
                "turn": {
                    "hasFallback": context.turn.fallback.is_some(),
                },
            }),
        );

        let compiler_context = context.clone();
        let output = compile_plan(DomainCompilerInput {
            config: domain_config,
            context,
        })
        .map_err(|error| AppError::Streaming(error.to_string()))?;
        self.runtime_trace.record_decision(
            "streaming",
            "compiledPlan",
            None,
            serde_json::json!({
                "targetType": target_type.as_str(),
                "targetId": target_id,
                "plan": output.plan,
            }),
        );

        log::info!(
            "streaming plan resolved: target={} target_id={} device_profile={:?} resolution={}x{} os={}",
            target_type.as_str(),
            target_id,
            output.plan.session.device.kind,
            output.plan.session.device.max_width,
            output.plan.session.device.max_height,
            output.plan.session.device.os_name,
        );

        Ok((output.plan, compiler_context))
    }

    async fn build_domain_context(
        &self,
        target: DomainTarget,
        target_id: &str,
        target_type: &str,
        session: DomainSessionAccessContext,
    ) -> DomainContext {
        let title_capability = self.resolve_title_capabilities(target, target_id).await;
        let remote_play = self.resolve_remote_play_context(target, target_id).await;

        DomainContext {
            target,
            target_id: target_id.to_string(),
            session,
            // input 用户配置面暂不开放，但 title 能力事实仍需来自真实数据源。
            input: title_capability.capabilities,
            input_capability: title_capability.input_capability,
            // runtime 能力按当前宿主真实 provider 状态计算，避免硬编码。
            runtime: resolve_runtime_capabilities(self.xbxengine_provider.is_runtime_available()),
            // xHome fallback TURN 由 provider 提供，失败时保守降级为 None。
            turn: DomainTurnContext {
                fallback: self.resolve_fallback_turn_server(target_type).await,
            },
            // remote play 地址注入来自 data provider；缺失时显式为空。
            remote_play: xbox_streaming::RemotePlayContext {
                configuration_resolved: remote_play.configuration_resolved,
                remote_management_enabled: remote_play.remote_management_enabled,
                console_streaming_enabled: remote_play.console_streaming_enabled,
                console_addrs: remote_play.console_addrs,
            },
        }
    }

    async fn resolve_fallback_turn_server(&self, target_type: &str) -> Option<TurnServer> {
        let mut provider = self.inner.fallback_turn_provider.lock().await;
        match provider.get_by_target_type(target_type).await {
            Ok(server) => server,
            Err(error) => {
                log::warn!(
                    "streaming fallback turn resolve failed for target={target_type}: {error}"
                );
                None
            }
        }
    }

    async fn resolve_title_capabilities(
        &self,
        target: DomainTarget,
        target_id: &str,
    ) -> ResolvedTitleCapabilitySnapshot {
        if target.is_home() {
            return ResolvedTitleCapabilitySnapshot::default();
        }

        let mut capabilities = resolve_min_title_capabilities(target_id);
        let mut input_capability = DomainInputCapabilityContext::default();
        match self
            .data_provider
            .get_streaming_title_input_config(target_id)
            .await
        {
            Ok(config) => {
                let parsed = parse_title_capabilities_from_input_config(&config.config);
                input_capability.input_config_resolved = true;
                input_capability.input_config_supports_mkb = parsed.has_mkb;
                input_capability.input_config_supports_touch = parsed.has_touch;
                input_capability.input_config_supports_native_touch = parsed.has_native_touch;
                apply_input_config_title_capabilities(&mut capabilities, parsed);
            }
            Err(error) => {
                log::warn!(
                    "streaming title capability resolve failed for title={target_id}: {error}"
                );
            }
        }
        ResolvedTitleCapabilitySnapshot {
            capabilities,
            input_capability,
        }
    }

    async fn resolve_remote_play_context(
        &self,
        target: DomainTarget,
        target_id: &str,
    ) -> ResolvedRemotePlayContext {
        if !target.is_home() || target_id.trim().is_empty() {
            return ResolvedRemotePlayContext::default();
        }

        let consoles = match self.data_provider.get_remote_consoles().await {
            Ok(value) => value,
            Err(error) => {
                log::warn!("streaming console addrs resolve failed: {error}");
                return ResolvedRemotePlayContext::default();
            }
        };

        let Some(console) = consoles.into_iter().find(|item| {
            item.server_id.as_deref() == Some(target_id)
                || item.id.as_deref() == Some(target_id)
                || item.device_id.as_deref() == Some(target_id)
        }) else {
            return ResolvedRemotePlayContext::default();
        };

        ResolvedRemotePlayContext {
            configuration_resolved: true,
            remote_management_enabled: console.remote_management_enabled,
            console_streaming_enabled: console.console_streaming_enabled,
            console_addrs: console
                .console_addrs
                .unwrap_or_default()
                .into_iter()
                .map(|item| DomainHostAddr {
                    ip: item.ip,
                    port: item.port,
                })
                .collect(),
        }
    }

    async fn preflight_home_remote_play_registration(
        &self,
        plan: &StreamingPlan,
        target_id: &str,
    ) -> Result<(), SessionFlowError> {
        if !plan.session.target.is_home() || !plan.session.schedule.require_console_ready {
            return Ok(());
        }

        let started_at_ms = now_ms();
        let timeout_ms = HOME_REMOTE_PLAY_PREFLIGHT_TIMEOUT_MS
            .min(plan.session.schedule.ready_timeout_ms)
            .max(HOME_REMOTE_PLAY_PREFLIGHT_POLL_INTERVAL_MS);
        let poll_interval_ms = HOME_REMOTE_PLAY_PREFLIGHT_POLL_INTERVAL_MS.max(
            plan.session
                .schedule
                .monitor_interval_ms
                .max(200)
                .min(HOME_REMOTE_PLAY_PREFLIGHT_POLL_INTERVAL_MS),
        );
        let mut attempt = 0u32;

        loop {
            attempt = attempt.saturating_add(1);
            let consoles = self
                .data_provider
                .get_remote_consoles()
                .await
                .map_err(|error| SessionFlowError::message(error.to_string()))?;
            let matched = consoles
                .iter()
                .find(|console| matches_remote_host_id(target_id, console));
            let elapsed_ms = now_ms().saturating_sub(started_at_ms);
            let ready = matched.is_some_and(is_home_remote_play_registered);

            self.runtime_trace.record_snapshot(
                "streaming",
                "homeRemotePlayPreflightSnapshot",
                None,
                serde_json::json!({
                    "targetId": target_id,
                    "attempt": attempt,
                    "elapsedMs": elapsed_ms,
                    "matched": matched.is_some(),
                    "powerState": matched.and_then(|console| console.power_state.clone()),
                    "remoteManagementEnabled": matched.and_then(|console| console.remote_management_enabled),
                    "consoleStreamingEnabled": matched.and_then(|console| console.console_streaming_enabled),
                    "consoleAddrsCount": matched
                        .and_then(|console| console.console_addrs.as_ref().map(|items| items.len()))
                        .unwrap_or(0),
                    "ready": ready,
                }),
            );

            if ready {
                return Ok(());
            }

            if elapsed_ms >= timeout_ms {
                if matched.is_some_and(|console| console.power_state.as_deref() == Some("On")) {
                    return Err(home_remote_play_not_ready_error(
                        target_id, matched, attempt, elapsed_ms,
                    ));
                }

                return Err(SessionFlowError::message(format!(
                    "remoteConsoleNotReady:targetId={target_id}"
                )));
            }

            tokio::time::sleep(std::time::Duration::from_millis(poll_interval_ms)).await;
        }
    }
}

fn resolve_runtime_capabilities(xbxengine_runtime_available: bool) -> DomainRuntimeCapabilities {
    DomainRuntimeCapabilities {
        browser_webrtc: true,
        rust_owned: xbxengine_runtime_available,
        native_mkb: false,
        touch_surface: false,
        prefer_browser: true,
    }
}

fn resolve_min_title_capabilities(target_id: &str) -> DomainTitleCapabilities {
    let mut capabilities = DomainTitleCapabilities::default();

    // 云游戏标题默认至少支持手柄；title 细粒度输入能力后续由 data provider 注入。
    capabilities
        .supported_inputs
        .push(DomainSupportedInput::Gamepad);
    capabilities.has_touch = true;

    // 仅在有合法 title id 时，才启用最小触控能力兜底，避免把异常路由误判成 touch title。
    if target_id.trim().parse::<u64>().is_ok() {
        capabilities.has_native_touch = true;
        capabilities
            .supported_inputs
            .push(DomainSupportedInput::NativeTouch);
    }

    capabilities
}

fn parse_title_capabilities_from_input_config(
    config: &serde_json::Value,
) -> DomainTitleCapabilities {
    let mut capabilities = DomainTitleCapabilities::default();
    let mut tokens = Vec::new();
    collect_string_tokens(config, &mut tokens);

    for token in tokens {
        let normalized = token.trim().to_ascii_lowercase();
        if normalized.is_empty() {
            continue;
        }

        if normalized.contains("native") && normalized.contains("touch") {
            capabilities.has_touch = true;
            capabilities.has_native_touch = true;
            push_supported_input(&mut capabilities, DomainSupportedInput::NativeTouch);
            continue;
        }
        if normalized.contains("touch") {
            capabilities.has_touch = true;
            push_supported_input(&mut capabilities, DomainSupportedInput::GenericTouch);
            continue;
        }
        if normalized == "mkb" || normalized.contains("mousekeyboard") {
            capabilities.has_mkb = true;
            push_supported_input(&mut capabilities, DomainSupportedInput::Mkb);
            continue;
        }
        if normalized.contains("keyboard") {
            capabilities.has_mkb = true;
            push_supported_input(&mut capabilities, DomainSupportedInput::Keyboard);
            continue;
        }
        if normalized.contains("mouse") {
            capabilities.has_mkb = true;
            push_supported_input(&mut capabilities, DomainSupportedInput::Mouse);
            continue;
        }
        if normalized.contains("gamepad") {
            push_supported_input(&mut capabilities, DomainSupportedInput::Gamepad);
        }
    }

    capabilities
}

fn collect_string_tokens(value: &serde_json::Value, output: &mut Vec<String>) {
    match value {
        serde_json::Value::String(text) => output.push(text.to_string()),
        serde_json::Value::Array(items) => {
            for item in items {
                collect_string_tokens(item, output);
            }
        }
        serde_json::Value::Object(map) => {
            for (key, item) in map {
                output.push(key.to_string());
                collect_string_tokens(item, output);
            }
        }
        _ => {}
    }
}

fn apply_input_config_title_capabilities(
    target: &mut DomainTitleCapabilities,
    source: DomainTitleCapabilities,
) {
    // inputconfigs 是标题输入能力的显式来源；一旦命中，touch/MKB 事实应以它为准，
    // 仅保留最小 gamepad 兜底，避免本地默认值把“官方未声明支持”的标题误判成 touch title。
    target.has_mkb = source.has_mkb;
    target.has_touch = source.has_touch;
    target.has_native_touch = source.has_native_touch;
    target
        .supported_inputs
        .retain(|item| matches!(item, DomainSupportedInput::Gamepad));

    for input in source.supported_inputs {
        push_supported_input(target, input);
    }
}

fn push_supported_input(capabilities: &mut DomainTitleCapabilities, input: DomainSupportedInput) {
    if !capabilities
        .supported_inputs
        .iter()
        .any(|item| item == &input)
    {
        capabilities.supported_inputs.push(input);
    }
}

#[async_trait::async_trait]
impl crate::mods::streaming::StreamingProvider for StreamingService {
    async fn start_session(
        &self,
        params: StreamingStartSessionParams,
    ) -> AppResult<StreamingStartSessionResult> {
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
        let (plan, compiler_context) = self
            .resolve_streaming_plan(&params.target_type, &params.target_id)
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
        self.preflight_home_remote_play_registration(&plan, &params.target_id)
            .await
            .map_err(|error| {
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
                AppError::streaming_detailed(
                    error.to_string(),
                    serde_json::to_value(&startup_error).unwrap_or(serde_json::Value::Null),
                )
            })?;
        // 执行快照会消费 plan，这里先投影出页面侧需要的稳定元数据。
        let metadata = project_session_metadata(&plan);
        let capabilities = project_session_capabilities(&compiler_context, &plan);
        let execution = match self
            .inner
            .flow
            .start_session_execution_with_observer(
                plan,
                project_runtime_plan,
                project_render_plan,
                Some(&startup),
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
            .map(Into::into)
            .unwrap_or_else(|| {
                StreamingSessionProgressSnapshot::from_session_snapshot(&execution.session)
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

    async fn get_session_progress(
        &self,
        params: StreamingGetSessionProgressParams,
    ) -> AppResult<Option<StreamingSessionProgressSnapshot>> {
        Ok(self
            .inner
            .flow
            .get_session_progress(&params.session_id)
            .await
            .map(Into::into))
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
        let reason = reason.map(|value| value.as_str().to_string());
        self.runtime_trace.record_decision(
            "streaming",
            "decideRecovery",
            Some(&params.session_id),
            serde_json::json!({
                "fact": fact_value,
                "isClosing": params.is_closing,
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

fn to_domain_ice_candidate(candidate: &StreamingIceCandidate) -> DomainIceCandidate {
    DomainIceCandidate {
        candidate: candidate.candidate.clone(),
        sdp_m_line_index: candidate.sdp_m_line_index,
        sdp_mid: candidate.sdp_mid.clone(),
        username_fragment: candidate.username_fragment.clone(),
        message_type: candidate.message_type.clone(),
    }
}

fn from_domain_ice_candidate(candidate: DomainIceCandidate) -> StreamingIceCandidate {
    StreamingIceCandidate {
        candidate: candidate.candidate,
        sdp_m_line_index: candidate.sdp_m_line_index,
        sdp_mid: candidate.sdp_mid,
        username_fragment: candidate.username_fragment,
        message_type: candidate.message_type,
    }
}

fn from_domain_answer_payload(payload: DomainAnswerPayload) -> StreamingAnswerPayload {
    StreamingAnswerPayload {
        sdp: payload.sdp,
        message_type: payload.message_type,
    }
}

fn normalize_optional(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn map_startup_phase(phase: DomainStartupPhase) -> StreamingStartupPhase {
    match phase {
        DomainStartupPhase::ResolvingContext => StreamingStartupPhase::ResolvingContext,
        DomainStartupPhase::WakingConsole => StreamingStartupPhase::WakingConsole,
        DomainStartupPhase::WaitingConsoleReady => StreamingStartupPhase::WaitingConsoleReady,
        DomainStartupPhase::CreatingSession => StreamingStartupPhase::CreatingSession,
        DomainStartupPhase::WaitingSessionReady => StreamingStartupPhase::WaitingSessionReady,
        DomainStartupPhase::StartingRuntime => StreamingStartupPhase::StartingRuntime,
        DomainStartupPhase::Ready => StreamingStartupPhase::Ready,
        DomainStartupPhase::Failed => StreamingStartupPhase::Failed,
    }
}

fn map_startup_phase_status(status: DomainStartupPhaseStatus) -> StreamingStartupPhaseStatus {
    match status {
        DomainStartupPhaseStatus::Entered => StreamingStartupPhaseStatus::Entered,
        DomainStartupPhaseStatus::Succeeded => StreamingStartupPhaseStatus::Succeeded,
        DomainStartupPhaseStatus::Failed => StreamingStartupPhaseStatus::Failed,
    }
}

fn startup_phase_summary(phase: &StreamingStartupPhase, details: Option<&str>) -> String {
    let details_suffix = details
        .filter(|value| !value.is_empty())
        .map(|value| format!(" ({value})"))
        .unwrap_or_default();
    match phase {
        StreamingStartupPhase::ResolvingContext => format!("resolvingContext{details_suffix}"),
        StreamingStartupPhase::WakingConsole => format!("wakingConsole{details_suffix}"),
        StreamingStartupPhase::WaitingConsoleReady => {
            format!("waitingConsoleReady{details_suffix}")
        }
        StreamingStartupPhase::CreatingSession => format!("creatingSession{details_suffix}"),
        StreamingStartupPhase::WaitingSessionReady => {
            format!("waitingSessionReady{details_suffix}")
        }
        StreamingStartupPhase::StartingRuntime => format!("startingRuntime{details_suffix}"),
        StreamingStartupPhase::Ready => format!("ready{details_suffix}"),
        StreamingStartupPhase::Failed => format!("failed{details_suffix}"),
    }
}

fn classify_startup_error_kind(
    phase: &StreamingStartupPhase,
    error: &SessionFlowError,
) -> StreamingStartupErrorKind {
    let normalized = error.message.to_ascii_lowercase();
    if is_home_remote_play_not_ready_message(&normalized) {
        return StreamingStartupErrorKind::HostRemotePlayUnavailable;
    }
    if normalized.contains("target missing") {
        return StreamingStartupErrorKind::Target;
    }
    if normalized.contains("auth") || normalized.contains("token missing") {
        return StreamingStartupErrorKind::Auth;
    }
    if normalized.contains("network error") {
        return StreamingStartupErrorKind::Network;
    }
    match phase {
        StreamingStartupPhase::ResolvingContext => StreamingStartupErrorKind::Unknown,
        StreamingStartupPhase::WakingConsole => StreamingStartupErrorKind::Wake,
        StreamingStartupPhase::WaitingConsoleReady => StreamingStartupErrorKind::ConsoleReady,
        StreamingStartupPhase::CreatingSession => StreamingStartupErrorKind::SessionCreate,
        StreamingStartupPhase::WaitingSessionReady => StreamingStartupErrorKind::SessionReady,
        StreamingStartupPhase::StartingRuntime => StreamingStartupErrorKind::Runtime,
        StreamingStartupPhase::Ready => StreamingStartupErrorKind::Runtime,
        StreamingStartupPhase::Failed => StreamingStartupErrorKind::Unknown,
    }
}

fn startup_error_message_key(
    kind: &StreamingStartupErrorKind,
    error: &SessionFlowError,
) -> &'static str {
    if is_home_remote_play_not_ready_message(&error.message.to_ascii_lowercase()) {
        return "streamPage.errors.hostRemotePlayUnavailable";
    }
    if is_remote_console_wake_circuit_open_message(&error.message) {
        return "streamPage.errors.hostRemotePlayUnavailable";
    }
    match kind {
        StreamingStartupErrorKind::Wake => "streamPage.errors.wakeFailed",
        StreamingStartupErrorKind::ConsoleReady => "streamPage.errors.consoleReadyFailed",
        StreamingStartupErrorKind::SessionCreate => "streamPage.errors.sessionCreateFailed",
        StreamingStartupErrorKind::SessionReady => "streamPage.errors.sessionReadyFailed",
        StreamingStartupErrorKind::Runtime => "streamPage.errors.runtimeStartFailed",
        StreamingStartupErrorKind::Network => "streamPage.errors.networkFailed",
        StreamingStartupErrorKind::Auth => "streamPage.errors.authFailed",
        StreamingStartupErrorKind::Target => "streamPage.errors.targetMissing",
        StreamingStartupErrorKind::HostRemotePlayUnavailable => {
            "streamPage.errors.hostRemotePlayUnavailable"
        }
        StreamingStartupErrorKind::Unknown => "streamPage.errors.unknown",
    }
}

fn build_startup_diagnostic_summary(
    phase: &StreamingStartupPhase,
    error: &SessionFlowError,
) -> String {
    if let Some(summary) = build_home_remote_play_not_ready_summary(&error.message) {
        return format!("phase={phase:?}; {summary}");
    }
    if let Some(summary) = build_remote_console_wake_circuit_summary(&error.message) {
        return format!("phase={phase:?}; {summary}");
    }
    let detail = error
        .body
        .as_deref()
        .filter(|value| !value.is_empty())
        .unwrap_or(&error.message);
    format!("phase={phase:?}; detail={detail}")
}

fn is_startup_error_retryable(kind: &StreamingStartupErrorKind, error: &SessionFlowError) -> bool {
    if is_home_remote_play_not_ready_message(&error.message.to_ascii_lowercase()) {
        return false;
    }
    if is_remote_console_wake_circuit_open_message(&error.message) {
        return false;
    }
    matches!(
        kind,
        StreamingStartupErrorKind::Wake
            | StreamingStartupErrorKind::ConsoleReady
            | StreamingStartupErrorKind::SessionCreate
            | StreamingStartupErrorKind::SessionReady
            | StreamingStartupErrorKind::Network
    ) || error.status.is_some_and(|status| status >= 500)
}

fn is_home_remote_play_not_ready_message(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    normalized.contains("homeremoteplaynotready")
}

fn is_remote_console_wake_circuit_open_message(message: &str) -> bool {
    message
        .to_ascii_lowercase()
        .contains("remoteconsolewakecircuitopen")
}

fn build_home_remote_play_not_ready_summary(message: &str) -> Option<String> {
    if !is_home_remote_play_not_ready_message(message) {
        return None;
    }

    let target_id = extract_preflight_field(message, "targetId").unwrap_or("unknown");
    let power_state = extract_preflight_field(message, "powerState").unwrap_or("unknown");
    let remote_management_enabled =
        extract_preflight_field(message, "remoteManagementEnabled").unwrap_or("unknown");
    let console_streaming_enabled =
        extract_preflight_field(message, "consoleStreamingEnabled").unwrap_or("unknown");
    let console_addrs_count =
        extract_preflight_field(message, "consoleAddrsCount").unwrap_or("unknown");
    let attempts = extract_preflight_field(message, "attempts").unwrap_or("unknown");
    let elapsed_ms = extract_preflight_field(message, "elapsedMs").unwrap_or("unknown");

    Some(format!(
        "targetId={target_id}; powerState={power_state}; remoteManagementEnabled={remote_management_enabled}; consoleStreamingEnabled={console_streaming_enabled}; consoleAddrsCount={console_addrs_count}; attempts={attempts}; elapsedMs={elapsed_ms}; hint=hostRemotePlayUnavailable"
    ))
}

fn build_remote_console_wake_circuit_summary(message: &str) -> Option<String> {
    if !is_remote_console_wake_circuit_open_message(message) {
        return None;
    }

    let target_id = extract_circuit_open_field(message, "targetId").unwrap_or("unknown");
    let power_state = extract_circuit_open_field(message, "powerState").unwrap_or("unknown");
    let wake_failure_count =
        extract_circuit_open_field(message, "wakeFailureCount").unwrap_or("unknown");
    Some(format!(
        "targetId={target_id}; powerState={power_state}; wakeFailureCount={wake_failure_count}; hint=hostRemotePlayUnavailable"
    ))
}

fn extract_preflight_field<'a>(message: &'a str, key: &str) -> Option<&'a str> {
    let prefix = format!("{key}=");
    message
        .split(';')
        .find_map(|segment| segment.trim().strip_prefix(&prefix))
}

fn extract_circuit_open_field<'a>(message: &'a str, key: &str) -> Option<&'a str> {
    let prefix = format!("{key}=");
    message
        .split(';')
        .find_map(|segment| segment.trim().strip_prefix(&prefix))
}

fn matches_remote_host_id(
    target_id: &str,
    console: &crate::mods::data::types::DataHostSummary,
) -> bool {
    console.server_id.as_deref() == Some(target_id)
        || console.id.as_deref() == Some(target_id)
        || console.device_id.as_deref() == Some(target_id)
}

fn is_home_remote_play_registered(console: &crate::mods::data::types::DataHostSummary) -> bool {
    console.power_state.as_deref() == Some("On")
        && (console.remote_management_enabled == Some(true)
            || console.console_streaming_enabled == Some(true))
}

fn home_remote_play_not_ready_error(
    target_id: &str,
    console: Option<&crate::mods::data::types::DataHostSummary>,
    attempts: u32,
    elapsed_ms: u64,
) -> SessionFlowError {
    let power_state = console
        .and_then(|console| console.power_state.as_deref())
        .unwrap_or("unknown");
    let remote_management_enabled =
        format_option_bool(console.and_then(|console| console.remote_management_enabled));
    let console_streaming_enabled =
        format_option_bool(console.and_then(|console| console.console_streaming_enabled));
    let console_addrs_count = console
        .and_then(|console| console.console_addrs.as_ref().map(|items| items.len()))
        .unwrap_or(0);

    SessionFlowError::message(format!(
        "homeRemotePlayNotReady:targetId={target_id};powerState={power_state};remoteManagementEnabled={remote_management_enabled};consoleStreamingEnabled={console_streaming_enabled};consoleAddrsCount={console_addrs_count};attempts={attempts};elapsedMs={elapsed_ms};hint=hostRemotePlayUnavailable"
    ))
}

fn format_option_bool(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "true",
        Some(false) => "false",
        None => "null",
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn apply_streaming_preferences(
    config: &mut DomainStreamingConfig,
    snapshot: &StreamingConfigSnapshot,
) {
    // 这里只做原始设置到 crate 偏好的搬运，不在 tauri 层解释执行策略。
    config.negotiation.video_codec = parse_codec_preference(&snapshot.codec);
    config.negotiation.home_video_bitrate =
        parse_bitrate_preference(&snapshot.xhome_bitrate_mode, snapshot.xhome_bitrate);
    config.negotiation.cloud_video_bitrate =
        parse_bitrate_preference(&snapshot.xcloud_bitrate_mode, snapshot.xcloud_bitrate);
    config.negotiation.audio_bitrate =
        parse_audio_bitrate_preference(&snapshot.audio_bitrate_mode, snapshot.audio_bitrate);
    config.negotiation.audio_channels = AudioChannels::Auto;

    config.input.polling_rate_hz = snapshot.polling_rate.clamp(1, u16::MAX as i64) as u16;
    config.input.vibration = snapshot.vibration;
    config.session.power_on = snapshot.power_on;

    config.runtime.mode = parse_runtime_preference(&snapshot.stream_runtime_mode);
    config.runtime.custom_turn = resolve_custom_turn(snapshot);
    config.runtime.home_fallback_turn = snapshot.xhome_turn_fallback;

    config.render.enable_audio_control = snapshot.enable_audio_control;
    config.render.video_format = normalize_optional(&snapshot.video_format);
    config.render.display_options = DomainDisplayOptions {
        sharpness: snapshot.display_options.sharpness,
        saturation: snapshot.display_options.saturation,
        contrast: snapshot.display_options.contrast,
        brightness: snapshot.display_options.brightness,
    };
}

fn parse_bitrate_preference(mode: &str, bitrate_mbps: i64) -> BitratePreference {
    if mode != "Custom" || bitrate_mbps <= 0 {
        return BitratePreference::Auto;
    }

    let kbps = bitrate_mbps.saturating_mul(1000);
    BitratePreference::CustomKbps {
        kbps: kbps.clamp(1, u32::MAX as i64) as u32,
    }
}

fn parse_audio_bitrate_preference(mode: &str, bitrate_kbps: i64) -> BitratePreference {
    if mode != "Custom" || bitrate_kbps <= 0 {
        return BitratePreference::Auto;
    }

    BitratePreference::CustomKbps {
        kbps: bitrate_kbps.clamp(1, u32::MAX as i64) as u32,
    }
}

fn parse_codec_preference(value: &str) -> CodecPreference {
    match value.trim() {
        "" => CodecPreference::Auto,
        "video/H264-420" => CodecPreference::H264Low,
        "video/H264-42e" => CodecPreference::H264Normal,
        "video/H264-4d" => CodecPreference::H264Main,
        "video/H264-64" => CodecPreference::H264High,
        mime_type => CodecPreference::MimeType {
            mime_type: mime_type.to_string(),
        },
    }
}

fn parse_runtime_preference(value: &str) -> RuntimePreference {
    match value.trim() {
        "webrtc-direct" => RuntimePreference::WebRtcDirect,
        "rust-owned" => RuntimePreference::RustOwned,
        _ => RuntimePreference::Auto,
    }
}

fn normalize_vibration_strength(value: &str) -> String {
    match value.trim() {
        "enhanced" => "enhanced".to_string(),
        "full" => "full".to_string(),
        _ => "realistic".to_string(),
    }
}

fn resolve_custom_turn(snapshot: &StreamingConfigSnapshot) -> Option<TurnServer> {
    Some(TurnServer {
        url: normalize_optional(&snapshot.server_url)?,
        username: normalize_optional(&snapshot.server_username)?,
        credential: normalize_optional(&snapshot.server_credential)?,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        apply_streaming_preferences, build_startup_diagnostic_summary, classify_startup_error_kind,
        is_startup_error_retryable, parse_audio_bitrate_preference, parse_bitrate_preference,
        parse_codec_preference, parse_runtime_preference, startup_error_message_key,
        SessionFlowError, StreamingStartupErrorKind, StreamingStartupPhase,
    };
    use crate::mods::streaming::types::{StreamingConfigSnapshot, StreamingDisplayOptionsValue};
    use xbox_streaming::{
        BitratePreference, CodecPreference, Config as DomainStreamingConfig, RuntimePreference,
    };

    #[test]
    fn maps_http_error_with_status_and_body() {
        let error = SessionFlowError::http(503, "HTTP 503: error", Some("body".to_string()));
        assert_eq!(error.status, Some(503));
        assert_eq!(error.body.as_deref(), Some("body"));
    }

    #[test]
    fn apply_streaming_preferences_maps_runtime_and_negotiation_fields() {
        let mut config = DomainStreamingConfig::default();
        let snapshot = StreamingConfigSnapshot {
            resolution: 1080,
            xhome_resolution: 1080,
            preferred_game_language: "en-US".to_string(),
            ipv6: false,
            force_region_ip: String::new(),
            xhome_bitrate_mode: "Custom".to_string(),
            xhome_bitrate: 35,
            xcloud_bitrate_mode: "Custom".to_string(),
            xcloud_bitrate: 18,
            audio_bitrate_mode: "Custom".to_string(),
            audio_bitrate: 2,
            codec: "video/H264-42e".to_string(),
            polling_rate: 333,
            vibration: false,
            vibration_strength: "enhanced".to_string(),
            stream_runtime_mode: "rust-owned".to_string(),
            power_on: true,
            server_url: "turn:example.test:3478".to_string(),
            server_username: "user".to_string(),
            server_credential: "secret".to_string(),
            xhome_turn_fallback: true,
            enable_audio_control: true,
            video_format: "Zoom".to_string(),
            display_options: StreamingDisplayOptionsValue {
                sharpness: 5,
                saturation: 110,
                contrast: 90,
                brightness: 105,
            },
        };

        apply_streaming_preferences(&mut config, &snapshot);

        assert_eq!(config.negotiation.video_codec, CodecPreference::H264Normal);
        assert_eq!(
            config.negotiation.home_video_bitrate,
            BitratePreference::CustomKbps { kbps: 35_000 }
        );
        assert_eq!(
            config.negotiation.cloud_video_bitrate,
            BitratePreference::CustomKbps { kbps: 18_000 }
        );
        assert_eq!(
            config.negotiation.audio_bitrate,
            BitratePreference::CustomKbps { kbps: 2 }
        );
        assert_eq!(config.input.polling_rate_hz, 333);
        assert!(!config.input.vibration);
        assert!(config.session.power_on);
        assert_eq!(config.runtime.mode, RuntimePreference::RustOwned);
        assert_eq!(config.runtime.home_fallback_turn, true);
        assert!(config.render.enable_audio_control);
        assert_eq!(config.render.video_format.as_deref(), Some("Zoom"));
        assert_eq!(config.render.display_options.sharpness, 5);
        assert_eq!(
            config
                .runtime
                .custom_turn
                .as_ref()
                .map(|turn| turn.url.as_str()),
            Some("turn:example.test:3478")
        );
    }

    #[test]
    fn home_remote_play_not_ready_maps_to_host_remote_play_unavailable() {
        let error = SessionFlowError::message(
            "homeRemotePlayNotReady:targetId=console-1;powerState=On;remoteManagementEnabled=null;consoleStreamingEnabled=null;consoleAddrsCount=0;attempts=3;elapsedMs=8000;hint=hostRemotePlayUnavailable",
        );

        let phase = StreamingStartupPhase::ResolvingContext;
        let kind = classify_startup_error_kind(&phase, &error);

        assert_eq!(kind, StreamingStartupErrorKind::HostRemotePlayUnavailable);
        assert_eq!(
            startup_error_message_key(&kind, &error),
            "streamPage.errors.hostRemotePlayUnavailable"
        );
        assert!(!is_startup_error_retryable(&kind, &error));
        assert!(build_startup_diagnostic_summary(&phase, &error)
            .contains("hint=hostRemotePlayUnavailable"));
    }

    #[test]
    fn parse_helpers_fall_back_to_auto_when_values_are_empty() {
        assert_eq!(
            parse_bitrate_preference("Auto", 20),
            BitratePreference::Auto
        );
        assert_eq!(
            parse_audio_bitrate_preference("Auto", 24),
            BitratePreference::Auto
        );
        assert_eq!(
            parse_audio_bitrate_preference("Custom", 24),
            BitratePreference::CustomKbps { kbps: 24 }
        );
        assert_eq!(parse_codec_preference(""), CodecPreference::Auto);
        assert_eq!(
            parse_codec_preference("video/H264-64"),
            CodecPreference::H264High
        );
        assert_eq!(
            parse_codec_preference("video/H264-4d"),
            CodecPreference::H264Main
        );
        assert_eq!(
            parse_codec_preference("video/H264-42e"),
            CodecPreference::H264Normal
        );
        assert_eq!(
            parse_codec_preference("video/H264-420"),
            CodecPreference::H264Low
        );
        assert_eq!(parse_runtime_preference(""), RuntimePreference::Auto);
    }
}
