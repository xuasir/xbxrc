use crate::mods::config::ConfigProviderRef;
use crate::mods::data::session_resolver::resolve_web_token_claims;
use crate::mods::data::types::{
    DataConsolePowerResult, DataHostAddr, DataHostSummary, DataSendTextResult, DataSessionContext,
    DataStreamingTitleInputConfig,
};
use crate::mods::runtime_trace::RuntimeTraceRecorderRef;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::net::IpAddr;
use xbox_streaming::{
    compile as compile_plan, parse_session_access_context, CompilerInput as DomainCompilerInput,
    Config as DomainStreamingConfig, Context as DomainContext, Target as DomainTarget,
    WebApiSessionGateway,
};
use xbox_webapi::{ConsoleApi, WebApiError};

pub struct StreamingQueryService {
    config_provider: ConfigProviderRef,
    runtime_trace: RuntimeTraceRecorderRef,
    discovery_cache: std::sync::Mutex<Option<DiscoveryCache>>,
}

struct DiscoveryCache {
    scope: u64,
    created: std::time::Instant,
    regions: std::collections::BTreeMap<String, String>,
}
fn discovery_scope(session: &DataSessionContext, token: &Value, route: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    resolve_web_token_claims(&session.web_token)
        .map(|claims| claims.account_scope().to_owned())
        .hash(&mut hash);
    token.to_string().hash(&mut hash);
    route.hash(&mut hash);
    hash.finish()
}

impl StreamingQueryService {
    pub fn new(config_provider: ConfigProviderRef, runtime_trace: RuntimeTraceRecorderRef) -> Self {
        Self {
            config_provider,
            runtime_trace,
            discovery_cache: std::sync::Mutex::new(None),
        }
    }

    pub fn get_streaming_regions(
        &self,
        session: &DataSessionContext,
        target: DomainTarget,
    ) -> Result<crate::mods::data::types::DataStreamingRegions, WebApiError> {
        let token = session
            .streaming_tokens
            .get(if target.is_home() {
                "xHomeToken"
            } else {
                "xCloudToken"
            })
            .ok_or_else(|| WebApiError::auth("请先登录 Xbox 账户"))?;
        let access = parse_session_access_context(token)
            .map_err(|_| WebApiError::parse("服务区域响应暂不可用"))?;
        let preferred_region_name = if target.is_home() {
            String::new()
        } else {
            self.config_provider
                .resolve_cloud_region_preference(&access.regions)
        };
        Ok(crate::mods::data::types::DataStreamingRegions {
            regions: access
                .regions
                .into_iter()
                .map(|region| crate::mods::data::types::DataStreamingRegion {
                    name: region.name,
                    display_name: region.display_name,
                    short_name: region.short_name,
                    continent: region.continent,
                    is_default: region.is_default,
                })
                .collect(),
            updated_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            preferred_region_name,
        })
    }

    pub async fn discover_remote_consoles<C: Fn() -> bool>(
        &self,
        session: &DataSessionContext,
        target: Option<&str>,
        current: C,
    ) -> Result<Vec<DataHostSummary>, WebApiError> {
        let Some(token) = resolve_xhome_token(session) else {
            return Ok(Vec::new());
        };
        let access = parse_session_access_context(token)
            .map_err(|_| WebApiError::parse("主机串流凭据响应暂不可用"))?;
        let config_snapshot = self.config_provider.get_streaming_config();
        let key = discovery_scope(session, token, &config_snapshot.force_region_ip);
        let cached = self.discovery_cache.lock().ok().and_then(|cache| {
            cache
                .as_ref()
                .filter(|cache| cache.scope == key && cache.created.elapsed().as_secs() < 300)
                .and_then(|cache| target.and_then(|target| cache.regions.get(target).cloned()))
        });
        let regions = xbox_streaming::session::discovery::ordered_home_regions(
            &access.regions,
            cached.as_deref(),
        );
        let streaming_token = xbox_streaming::StreamingToken::parse(token)
            .map_err(|_| WebApiError::parse("主机串流凭据响应暂不可用"))?;
        let result = xbox_streaming::session::discovery::discover_home(
            &regions,
            target,
            |region| {
                let mut config = DomainStreamingConfig::new_home_config(
                    config_snapshot.preferred_game_language.clone(),
                    String::new(),
                    config_snapshot.xhome_resolution,
                );
                config.session.preferred_region_name = Some(region.name.clone());
                let context = DomainContext {
                    target: DomainTarget::Home,
                    session: access.clone(),
                    ..Default::default()
                };
                let token = streaming_token.clone();
                async move {
                    let output = compile_plan(DomainCompilerInput { config, context })
                        .map_err(|_| WebApiError::parse("主机区域配置暂不可用"))?;
                    WebApiSessionGateway::new(output.plan, token)
                        .get_remote_consoles()
                        .await
                }
            },
            || {
                current()
                    && self.config_provider.get_force_region_ip() == config_snapshot.force_region_ip
            },
        )
        .await?;
        if !current()
            || self.config_provider.get_force_region_ip() != config_snapshot.force_region_ip
        {
            return Err(WebApiError::parse("主机发现状态已更新，请重新查询"));
        }
        let Some((region, consoles)) = result else {
            return Ok(Vec::new());
        };
        let mut summaries = Vec::new();
        let mut seen = BTreeSet::new();
        for console in consoles {
            if let Ok(mut summary) = serde_json::from_value::<DataHostSummary>(console.clone()) {
                let identity = summary
                    .server_id
                    .as_ref()
                    .or(summary.id.as_ref())
                    .or(summary.device_id.as_ref());
                if identity.is_none_or(|identity| {
                    identity.trim().is_empty() || !seen.insert(identity.clone())
                }) {
                    continue;
                }
                summary.console_addrs = extract_console_addrs(&console).or(summary.console_addrs);
                summary.discovery_region_name = Some(region.name.clone());
                summaries.push(summary);
            }
        }
        if let Ok(mut cache) = self.discovery_cache.lock() {
            if cache
                .as_ref()
                .is_none_or(|cache| cache.scope != key || cache.created.elapsed().as_secs() >= 300)
            {
                *cache = Some(DiscoveryCache {
                    scope: key,
                    created: std::time::Instant::now(),
                    regions: Default::default(),
                });
            }
            if let Some(cache) = cache.as_mut() {
                for summary in &summaries {
                    for identity in [&summary.server_id, &summary.id, &summary.device_id]
                        .into_iter()
                        .flatten()
                    {
                        cache.regions.insert(identity.clone(), region.name.clone());
                    }
                }
            }
        }
        self.runtime_trace.record_event("data", "remoteConsoleRegionDiscovered", None, json!({"regionName":region.name,"consoleCount":summaries.len(),"targeted":target.is_some()}));
        Ok(summaries)
    }

    pub async fn get_streaming_title_input_config(
        &self,
        session: &DataSessionContext,
        xbox_title_id: &str,
    ) -> Result<DataStreamingTitleInputConfig, WebApiError> {
        let Some(api) = self.create_home_session_api(session).await? else {
            return Ok(DataStreamingTitleInputConfig {
                xbox_title_id: xbox_title_id.to_string(),
                config: json!({}),
            });
        };

        let config = api.input_configs(xbox_title_id).await?;

        Ok(DataStreamingTitleInputConfig {
            xbox_title_id: xbox_title_id.to_string(),
            config,
        })
    }

    pub async fn power_on_console(
        &self,
        session: &DataSessionContext,
        console_id: &str,
    ) -> Result<DataConsolePowerResult, WebApiError> {
        self.send_console_power_command(session, console_id, "WakeUp")
            .await
    }

    pub async fn power_off_console(
        &self,
        session: &DataSessionContext,
        console_id: &str,
    ) -> Result<DataConsolePowerResult, WebApiError> {
        self.send_console_power_command(session, console_id, "TurnOff")
            .await
    }

    pub async fn send_text_to_console(
        &self,
        session: &DataSessionContext,
        console_id: &str,
        text: &str,
    ) -> Result<DataSendTextResult, WebApiError> {
        let accepted = self
            .send_console_command(
                session,
                console_id,
                "Shell",
                "InjectString",
                Some(json!([
                    {
                        "replacementString": text
                    }
                ])),
            )
            .await?;

        Ok(DataSendTextResult {
            console_id: console_id.to_string(),
            accepted,
        })
    }

    async fn send_console_power_command(
        &self,
        session: &DataSessionContext,
        console_id: &str,
        command: &str,
    ) -> Result<DataConsolePowerResult, WebApiError> {
        let accepted = self
            .send_console_command(session, console_id, "Power", command, None)
            .await?;

        Ok(DataConsolePowerResult {
            console_id: console_id.to_string(),
            accepted,
        })
    }

    async fn send_console_command(
        &self,
        session: &DataSessionContext,
        console_id: &str,
        command_type: &str,
        command: &str,
        parameters: Option<Value>,
    ) -> Result<bool, WebApiError> {
        let Some(claims) = resolve_web_token_claims(&session.web_token) else {
            return Ok(false);
        };

        ConsoleApi::new(claims.uhs, claims.user_token)
            .send_command(console_id, command_type, command, parameters)
            .await
            .map(|response| response.accepted)
            .map_err(|error| {
                log::warn!("[Data][consoleCommand] failed: {}", error.safe_summary());
                error
            })
    }

    async fn create_home_session_api(
        &self,
        session: &DataSessionContext,
    ) -> Result<Option<WebApiSessionGateway>, WebApiError> {
        // RFC: 拆除横向依赖。mods/data 直接扁平依赖 crate，不横向依赖 mods/streaming。
        let Some(token) = resolve_xhome_token(session) else {
            return Ok(None);
        };

        let access_context = parse_session_access_context(token)
            .map_err(|_| WebApiError::parse("主机串流凭据响应暂不可用，请重试"))?;
        let config_snapshot = self.config_provider.get_streaming_config();
        let domain_config = DomainStreamingConfig::new_home_config(
            config_snapshot.preferred_game_language.clone(),
            config_snapshot.force_region_ip.clone(),
            config_snapshot.xhome_resolution,
        );

        let context = DomainContext {
            target: DomainTarget::Home,
            target_id: String::new(),
            session: access_context,
            ..Default::default()
        };

        let output = compile_plan(DomainCompilerInput {
            config: domain_config,
            context,
        })
        .map_err(|_| WebApiError::parse("主机串流配置暂不可用，请检查设置"))?;

        log::info!(
            "xhome query gateway resolved: device_profile={:?} resolution={}x{} os={}",
            output.plan.session.device.kind,
            output.plan.session.device.max_width,
            output.plan.session.device.max_height,
            output.plan.session.device.os_name,
        );

        let streaming_token = xbox_streaming::session::access::StreamingToken::parse(token)
            .map_err(|_| WebApiError::parse("主机串流凭据响应暂不可用，请重试"))?;

        // 既然已经有 token 了，直接使用 new 构造。
        Ok(Some(WebApiSessionGateway::new(
            output.plan,
            streaming_token,
        )))
    }
}

fn resolve_xhome_token(session: &DataSessionContext) -> Option<&Value> {
    session
        .streaming_tokens
        .get("xHomeToken")
        .or_else(|| session.streaming_tokens.get("xhomeToken"))
}

fn extract_console_addrs(raw: &Value) -> Option<Vec<DataHostAddr>> {
    // 仅沿已知 streaming/endpoint/candidate 路径提取，避免全量递归误抓非媒体地址。
    let mut found = Vec::new();
    collect_console_addrs_from_known_paths(raw, 0, &mut found);
    if found.is_empty() {
        return None;
    }

    let mut dedup = BTreeSet::new();
    let mut result = Vec::new();
    for addr in found {
        let key = format!("{}:{}", addr.ip, addr.port);
        if dedup.insert(key) {
            result.push(addr);
        }
    }
    (!result.is_empty()).then_some(result)
}

fn collect_console_addrs_from_known_paths(
    value: &Value,
    depth: usize,
    output: &mut Vec<DataHostAddr>,
) {
    if depth > 6 {
        return;
    }

    let Some(map) = value.as_object() else {
        return;
    };

    for (key, child) in map {
        if is_addr_candidate_list_key(key) {
            collect_console_addrs_from_candidate_value(child, output);
            continue;
        }
        if is_addr_container_key(key) {
            match child {
                Value::Object(_) => {
                    collect_console_addrs_from_known_paths(child, depth + 1, output)
                }
                Value::Array(items) => {
                    for item in items {
                        collect_console_addrs_from_known_paths(item, depth + 1, output);
                    }
                }
                _ => {}
            }
        }
    }
}

fn collect_console_addrs_from_candidate_value(value: &Value, output: &mut Vec<DataHostAddr>) {
    match value {
        Value::Array(items) => {
            for item in items {
                if let Value::Object(map) = item {
                    if let Some(addr) = try_parse_console_addr(map) {
                        output.push(addr);
                    }
                }
            }
        }
        Value::Object(map) => {
            if let Some(addr) = try_parse_console_addr(map) {
                output.push(addr);
                return;
            }
            for nested in map.values() {
                if let Value::Object(candidate) = nested {
                    if let Some(addr) = try_parse_console_addr(candidate) {
                        output.push(addr);
                    }
                }
            }
        }
        _ => {}
    }
}

fn is_addr_candidate_list_key(key: &str) -> bool {
    matches!(
        key,
        "streamingEndpoints"
            | "streamingAddresses"
            | "streamingCandidates"
            | "connectionCandidates"
            | "endpointCandidates"
            | "remotePlayEndpoints"
            | "consoleStreamingEndpoints"
            | "consoleAddrs"
            | "consoleAddresses"
            | "endpoints"
    )
}

fn is_addr_container_key(key: &str) -> bool {
    matches!(
        key,
        "serverDetails"
            | "streaming"
            | "remotePlay"
            | "network"
            | "configuration"
            | "connection"
            | "ice"
    )
}

fn try_parse_console_addr(map: &serde_json::Map<String, Value>) -> Option<DataHostAddr> {
    let ip = map
        .get("ipAddress")
        .or_else(|| map.get("ip"))
        .or_else(|| map.get("address"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .filter(|value| value.parse::<IpAddr>().is_ok())?
        .to_string();

    let port = map
        .get("port")
        .or_else(|| map.get("portNumber"))
        .or_else(|| map.get("streamingPort"))
        .and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_str().and_then(|text| text.parse::<u64>().ok()))
        })
        .filter(|value| *value > 0 && *value <= u16::MAX as u64)? as u16;

    Some(DataHostAddr { ip, port })
}

fn describe_json_value(value: Option<&Value>) -> String {
    match value {
        None => "missing".to_string(),
        Some(Value::Null) => "null".to_string(),
        Some(Value::Bool(value)) => format!("bool:{value}"),
        Some(Value::Number(value)) => format!("number:{value}"),
        Some(Value::String(value)) => format!("string:{value}"),
        Some(Value::Array(items)) => format!("array(len={})", items.len()),
        Some(Value::Object(map)) => format!("object(len={})", map.len()),
    }
}

fn needs_remote_play_capability_review(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };

    !matches!(object.get("remoteManagementEnabled"), Some(Value::Bool(_)))
        || !matches!(object.get("consoleStreamingEnabled"), Some(Value::Bool(_)))
}

#[cfg(test)]
mod tests {
    use super::{describe_json_value, extract_console_addrs, needs_remote_play_capability_review};
    use serde_json::json;

    #[test]
    fn extracts_addrs_only_from_known_streaming_paths() {
        let payload = json!({
            "serverDetails": {
                "streamingEndpoints": [
                    { "ipAddress": "10.0.0.8", "port": 9002 }
                ]
            }
        });

        let addrs = extract_console_addrs(&payload).expect("should extract");
        assert_eq!(addrs.len(), 1);
        assert_eq!(addrs[0].ip, "10.0.0.8");
        assert_eq!(addrs[0].port, 9002);
    }

    #[test]
    fn ignores_unrelated_address_port_objects() {
        let payload = json!({
            "telemetry": {
                "networkProbe": { "address": "192.168.1.9", "port": 443 }
            },
            "meta": {
                "streamingPort": 9002
            }
        });

        assert!(extract_console_addrs(&payload).is_none());
    }

    #[test]
    fn deduplicates_same_ip_port() {
        let payload = json!({
            "remotePlay": {
                "endpointCandidates": [
                    { "ip": "10.0.0.10", "portNumber": 9002 },
                    { "ipAddress": "10.0.0.10", "streamingPort": "9002" }
                ]
            }
        });

        let addrs = extract_console_addrs(&payload).expect("should extract");
        assert_eq!(addrs.len(), 1);
    }

    #[test]
    fn describes_missing_and_null_values() {
        let payload = json!({
            "presentNull": null,
            "presentBool": true,
            "presentString": "ready"
        });

        assert_eq!(describe_json_value(payload.get("missing")), "missing");
        assert_eq!(describe_json_value(payload.get("presentNull")), "null");
        assert_eq!(describe_json_value(payload.get("presentBool")), "bool:true");
        assert_eq!(
            describe_json_value(payload.get("presentString")),
            "string:ready"
        );
    }

    #[test]
    fn flags_non_boolean_or_missing_capability_fields_for_review() {
        assert!(needs_remote_play_capability_review(&json!({})));
        assert!(needs_remote_play_capability_review(&json!({
            "remoteManagementEnabled": null,
            "consoleStreamingEnabled": true
        })));
        assert!(needs_remote_play_capability_review(&json!({
            "remoteManagementEnabled": "yes",
            "consoleStreamingEnabled": false
        })));
        assert!(!needs_remote_play_capability_review(&json!({
            "remoteManagementEnabled": true,
            "consoleStreamingEnabled": false
        })));
    }
}
