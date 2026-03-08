use crate::mods::config::ConfigProviderRef;
use crate::mods::streaming::http_client::StreamingHttpClient;
use crate::mods::streaming::ice_normalizer::StreamingIceNormalizer;
use crate::mods::streaming::session_api::StreamingSessionApi;
use crate::mods::streaming::signaling_api::StreamingSignalingApi;
use crate::mods::streaming::types::StreamingConfigSnapshot;
use serde_json::Value;

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
        let (host, gs_token) = resolve_host_and_token(token, target_type, &config)?;
        let http_client = StreamingHttpClient::new(host, gs_token);

        Ok(StreamingSessionApi::new(
            target_type.to_string(),
            http_client,
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
        let (host, gs_token) = resolve_host_and_token(token, target_type, &config)?;
        let http_client = StreamingHttpClient::new(host, gs_token);

        Ok(StreamingSignalingApi::new(
            format!("/v5/sessions/{target_type}"),
            http_client,
            StreamingIceNormalizer::new(config.ipv6),
        ))
    }
}

fn resolve_host_and_token(
    token: &Value,
    target_type: &str,
    config: &StreamingConfigSnapshot,
) -> Result<(String, String), String> {
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

    let host = base_uri
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/')
        .to_string();

    if host.is_empty() {
        return Err(format!("Streaming region host is empty for {target_type}"));
    }

    Ok((host, gs_token))
}
