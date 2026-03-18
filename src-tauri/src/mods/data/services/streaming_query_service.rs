use crate::mods::config::ConfigProviderRef;
use crate::mods::data::session_resolver::resolve_web_token_claims;
use crate::mods::data::types::{
    DataConsolePowerResult, DataHostAddr, DataHostSummary, DataSendTextResult, DataSessionContext,
    DataStreamingTitleInputConfig,
};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use reqwest::Client;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::net::IpAddr;
use uuid::Uuid;
use xbox_streaming::{
    compile as compile_plan, parse_session_access_context, CompilerInput as DomainCompilerInput,
    Config as DomainStreamingConfig, Context as DomainContext, Target as DomainTarget,
    WebApiSessionGateway,
};

const XBOX_CLIENT_VERSION: &str = "39.39.22001.0";

pub struct StreamingQueryService {
    client: Client,
    config_provider: ConfigProviderRef,
}

impl StreamingQueryService {
    pub fn new(config_provider: ConfigProviderRef) -> Self {
        Self {
            client: Client::new(),
            config_provider,
        }
    }

    pub async fn get_remote_consoles(
        &self,
        session: &DataSessionContext,
    ) -> Result<Vec<DataHostSummary>, String> {
        let Some(api) = self.create_home_session_api(session).await? else {
            return Ok(Vec::new());
        };

        let consoles = api
            .get_remote_consoles()
            .await
            .map_err(|error| error.to_string())?;

        let mut summaries = Vec::new();
        for console in consoles {
            if let Ok(mut summary) = serde_json::from_value::<DataHostSummary>(console.clone()) {
                if summary.console_addrs.is_none() {
                    summary.console_addrs = extract_console_addrs(&console);
                }
                summaries.push(summary);
            }
        }

        Ok(summaries)
    }

    pub async fn get_streaming_title_input_config(
        &self,
        session: &DataSessionContext,
        xbox_title_id: &str,
    ) -> Result<DataStreamingTitleInputConfig, String> {
        let Some(api) = self.create_home_session_api(session).await? else {
            return Ok(DataStreamingTitleInputConfig {
                xbox_title_id: xbox_title_id.to_string(),
                config: json!({}),
            });
        };

        let config = api
            .input_configs(xbox_title_id)
            .await
            .map_err(|error| error.to_string())?;

        Ok(DataStreamingTitleInputConfig {
            xbox_title_id: xbox_title_id.to_string(),
            config,
        })
    }

    pub async fn power_on_console(
        &self,
        session: &DataSessionContext,
        console_id: &str,
    ) -> Result<DataConsolePowerResult, String> {
        self.send_console_power_command(session, console_id, "WakeUp")
            .await
    }

    pub async fn power_off_console(
        &self,
        session: &DataSessionContext,
        console_id: &str,
    ) -> Result<DataConsolePowerResult, String> {
        self.send_console_power_command(session, console_id, "TurnOff")
            .await
    }

    pub async fn send_text_to_console(
        &self,
        session: &DataSessionContext,
        console_id: &str,
        text: &str,
    ) -> Result<DataSendTextResult, String> {
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
    ) -> Result<DataConsolePowerResult, String> {
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
    ) -> Result<bool, String> {
        let Some(claims) = resolve_web_token_claims(&session.web_token) else {
            return Ok(false);
        };

        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("XBL3.0 x={};{}", claims.uhs, claims.user_token))
                .map_err(|error| error.to_string())?,
        );
        headers.insert("Accept-Language", HeaderValue::from_static("en-US"));
        headers.insert(
            "skillplatform",
            HeaderValue::from_static("RemoteManagement"),
        );
        headers.insert("x-xbl-contract-version", HeaderValue::from_static("4"));
        headers.insert("x-xbl-client-name", HeaderValue::from_static("XboxApp"));
        headers.insert("x-xbl-client-type", HeaderValue::from_static("UWA"));
        headers.insert(
            "x-xbl-client-version",
            HeaderValue::from_static(XBOX_CLIENT_VERSION),
        );
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let body = json!({
            "destination": "Xbox",
            "type": command_type,
            "command": command,
            "sessionId": Uuid::new_v4().to_string(),
            "sourceId": "com.microsoft.smartglass",
            "parameters": parameters.unwrap_or_else(|| json!([])),
            "linkedXboxId": console_id
        });

        let response = self
            .client
            .post("https://xccs.xboxlive.com/commands")
            .headers(headers)
            .json(&body)
            .send()
            .await
            .map_err(|error| error.to_string())?;

        let status = response.status();
        if status.is_success() {
            return Ok(true);
        }

        // 保留 XCCS 原始响应体，避免把关键业务错误吞成简单的 accepted=false。
        let body = response.text().await.map_err(|error| error.to_string())?;
        let body = body.trim();
        if body.is_empty() {
            Err(format!("XCCS command failed: HTTP {}", status.as_u16()))
        } else {
            Err(body.to_string())
        }
    }

    async fn create_home_session_api(
        &self,
        session: &DataSessionContext,
    ) -> Result<Option<WebApiSessionGateway>, String> {
        // RFC: 拆除横向依赖。mods/data 直接扁平依赖 crate，不横向依赖 mods/streaming。
        let Some(token) = resolve_xhome_token(session) else {
            return Ok(None);
        };

        let access_context = parse_session_access_context(token).map_err(|e| e.to_string())?;
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
        .map_err(|e| e.to_string())?;

        log::info!(
            "xhome query gateway resolved: device_profile={:?} resolution={}x{} os={}",
            output.plan.session.device.kind,
            output.plan.session.device.max_width,
            output.plan.session.device.max_height,
            output.plan.session.device.os_name,
        );

        let streaming_token = xbox_streaming::session::access::StreamingToken::parse(token)
            .map_err(|e| e.to_string())?;

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

#[cfg(test)]
mod tests {
    use super::extract_console_addrs;
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
}
