use crate::mods::config::ConfigProviderRef;
use crate::mods::streaming::ice_normalizer::StreamingIceNormalizer;
use crate::mods::streaming::session_api::StreamingSessionApi;
use crate::mods::streaming::signaling_api::StreamingSignalingApi;
use crate::mods::streaming::types::StreamingConfigSnapshot;
use serde_json::Value;
use xbox_webapi::{SessionApi, SignalingApi};

pub struct StreamingApiProvider {
    config_provider: ConfigProviderRef,
}

impl StreamingApiProvider {
    pub fn new(config_provider: ConfigProviderRef) -> Self {
        Self { config_provider }
    }

    pub async fn create_session_api(
        &self,
        token: &Value,
        target_type: &str,
    ) -> Result<StreamingSessionApi, String> {
        let config = self.config_provider.get_streaming_config();
        let resolved = resolve_token(token, target_type, &config)?;
        let session_api = SessionApi::new(
            target_type.to_string(),
            resolved.base_url.clone(),
            resolved.gs_token.clone(),
            config.resolution,
        );

        Ok(StreamingSessionApi::new(
            target_type.to_string(),
            session_api,
            config.preferred_game_language,
            config.resolution,
        ))
    }

    pub async fn create_signaling_api(
        &self,
        token: &Value,
        target_type: &str,
    ) -> Result<StreamingSignalingApi, String> {
        let config = self.config_provider.get_streaming_config();
        let resolved = resolve_token(token, target_type, &config)?;
        let session_base_path = format!("{}/v5/sessions/{target_type}", resolved.base_url);
        let signaling_api = SignalingApi::new(session_base_path.clone(), resolved.gs_token);

        Ok(StreamingSignalingApi::new(
            session_base_path,
            signaling_api,
            StreamingIceNormalizer::new(config.ipv6),
        ))
    }
}

struct ResolvedStreamingToken {
    gs_token: String,
    base_url: String,
}

fn resolve_token(
    token: &Value,
    target_type: &str,
    config: &StreamingConfigSnapshot,
) -> Result<ResolvedStreamingToken, String> {
    let data = token.get("data").unwrap_or(token);

    let gs_token = data
        .get("gsToken")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Streaming gsToken is missing for {target_type}"))?
        .to_string();

    let regions = data
        .get("offeringSettings")
        .and_then(|value| value.get("regions"))
        .and_then(Value::as_array)
        .ok_or_else(|| format!("Streaming region is missing for {target_type}"))?;

    let selected_region = if !config.force_region_ip.is_empty() {
        regions.iter().find(|item| {
            item.get("baseUri")
                .and_then(Value::as_str)
                .map(|uri| uri.contains(&config.force_region_ip))
                .unwrap_or(false)
        })
    } else {
        None
    }
    .or_else(|| {
        regions
            .iter()
            .find(|item| item.get("isDefault").and_then(Value::as_bool) == Some(true))
    })
    .or_else(|| regions.first())
    .ok_or_else(|| format!("Streaming region is missing for {target_type}"))?;

    let base_uri = selected_region
        .get("baseUri")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Streaming region uri is missing for {target_type}"))?;

    let base_url = normalize_base_url(base_uri)
        .ok_or_else(|| format!("Streaming region uri is invalid for {target_type}: {base_uri}"))?;

    Ok(ResolvedStreamingToken { gs_token, base_url })
}

fn normalize_base_url(base_uri: &str) -> Option<String> {
    let trimmed = base_uri.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return None;
    }

    // 统一转成带协议的绝对 URL，避免 reqwest builder error。
    let normalized = if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    };

    if normalized == "https://" || normalized == "http://" {
        return None;
    }

    Some(normalized)
}
