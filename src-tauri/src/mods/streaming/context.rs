//! 宿主能力和策略上下文装配；会话执行继续由公共 flow 负责。
use super::*;

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

impl StreamingService {
    pub(super) async fn resolve_streaming_plan(
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
}

pub(super) fn resolve_runtime_capabilities(
    xbxengine_runtime_available: bool,
) -> DomainRuntimeCapabilities {
    DomainRuntimeCapabilities {
        browser_webrtc: true,
        rust_owned: xbxengine_runtime_available,
        native_mkb: false,
        touch_surface: false,
        prefer_browser: true,
    }
}

pub(super) fn resolve_min_title_capabilities(target_id: &str) -> DomainTitleCapabilities {
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

pub(super) fn parse_title_capabilities_from_input_config(
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

pub(super) fn apply_input_config_title_capabilities(
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
