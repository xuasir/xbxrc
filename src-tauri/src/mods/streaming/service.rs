use crate::error::{AppError, AppResult};
use crate::mods::auth::AuthProviderRef;
use crate::mods::config::ConfigProviderRef;
use crate::mods::data::DataProviderRef;
use crate::mods::streaming::types::*;
use crate::mods::xbxengine::XbxEngineProviderRef;
use std::sync::Arc;
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
    Target as DomainTarget, TurnServer,
};

#[derive(Clone)]
pub struct StreamingService {
    auth_provider: AuthProviderRef,
    config_provider: ConfigProviderRef,
    data_provider: DataProviderRef,
    xbxengine_provider: XbxEngineProviderRef,
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

/// tauri 侧 flow adapter：仅负责提供凭证。
/// RFC: 策略与执行层已下沉 crate，adapter 彻底退化。
#[derive(Clone)]
struct TauriSessionFlowProvider {
    auth_provider: AuthProviderRef,
    data_provider: DataProviderRef,
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
            .map_err(SessionFlowError::message)?;
        Ok(result.accepted)
    }

    async fn get_remote_consoles(&self) -> Result<Vec<RemoteConsoleSnapshot>, SessionFlowError> {
        let consoles = self
            .data_provider
            .get_remote_consoles()
            .await
            .map_err(SessionFlowError::message)?;
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
}

impl StreamingService {
    pub fn new(
        auth_provider: AuthProviderRef,
        config_provider: ConfigProviderRef,
        data_provider: DataProviderRef,
        xbxengine_provider: XbxEngineProviderRef,
    ) -> Self {
        let flow_provider = TauriSessionFlowProvider {
            auth_provider: auth_provider.clone(),
            data_provider: data_provider.clone(),
        };

        Self {
            auth_provider,
            config_provider,
            data_provider,
            xbxengine_provider,
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

        let compiler_context = context.clone();
        let output = compile_plan(DomainCompilerInput {
            config: domain_config,
            context,
        })
        .map_err(|error| AppError::Streaming(error.to_string()))?;

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
        let (plan, compiler_context) = self
            .resolve_streaming_plan(&params.target_type, &params.target_id)
            .await?;
        // 执行快照会消费 plan，这里先投影出页面侧需要的稳定元数据。
        let metadata = project_session_metadata(&plan);
        let capabilities = project_session_capabilities(&compiler_context, &plan);
        let execution = self
            .inner
            .flow
            .start_session_execution(plan, project_runtime_plan, project_render_plan)
            .await
            .map_err(map_flow_error)?;
        let execution = StreamingSessionExecutionSnapshot {
            session: execution.session,
            runtime: execution.runtime.into(),
            render: execution.render.into(),
            metadata: metadata.into(),
            capabilities: capabilities.into(),
        };

        let progress = self
            .inner
            .flow
            .get_session_progress(&execution.session.id)
            .await
            .map(Into::into)
            .unwrap_or_else(|| {
                StreamingSessionProgressSnapshot::from_session_snapshot(&execution.session)
            });

        Ok(StreamingStartSessionResult {
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
        let closed = self
            .inner
            .flow
            .close_session(&params.session_id)
            .await
            .map_err(map_flow_error)?;

        Ok(StreamingCloseSessionResult { closed })
    }

    async fn exchange_offer(
        &self,
        params: StreamingExchangeOfferParams,
    ) -> AppResult<StreamingExchangeOfferResult> {
        let answer = self
            .inner
            .flow
            .exchange_offer(&params.session_id, params.channel.as_deref(), &params.sdp)
            .await
            .map_err(map_flow_error)?;

        Ok(StreamingExchangeOfferResult {
            answer: from_domain_answer_payload(answer),
        })
    }

    async fn exchange_ice(
        &self,
        params: StreamingExchangeIceParams,
    ) -> AppResult<StreamingExchangeIceResult> {
        let local_candidates = params
            .candidate
            .iter()
            .map(to_domain_ice_candidate)
            .collect::<Vec<_>>();
        let remote_candidates = self
            .inner
            .flow
            .exchange_ice(&params.session_id, &local_candidates)
            .await
            .map_err(map_flow_error)?;

        Ok(StreamingExchangeIceResult {
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
        parse_bitrate_preference(&snapshot.audio_bitrate_mode, snapshot.audio_bitrate);
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

fn parse_codec_preference(value: &str) -> CodecPreference {
    match value.trim() {
        "" => CodecPreference::Auto,
        "video/H264-420" => CodecPreference::H264Low,
        "video/H264-42e" => CodecPreference::H264Normal,
        "video/H264-4d" => CodecPreference::H264High,
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
        apply_streaming_preferences, parse_bitrate_preference, parse_codec_preference,
        parse_runtime_preference,
    };
    use crate::mods::streaming::types::{StreamingConfigSnapshot, StreamingDisplayOptionsValue};
    use xbox_streaming::SessionFlowError;
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
            BitratePreference::CustomKbps { kbps: 2_000 }
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
    fn parse_helpers_fall_back_to_auto_when_values_are_empty() {
        assert_eq!(
            parse_bitrate_preference("Auto", 20),
            BitratePreference::Auto
        );
        assert_eq!(parse_codec_preference(""), CodecPreference::Auto);
        assert_eq!(parse_runtime_preference(""), RuntimePreference::Auto);
    }
}
