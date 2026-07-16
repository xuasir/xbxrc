use crate::{
    auth_session_from_bundle, deserialize, normalize_force_region_ip, resolve_web_token_claims,
    AuthSession, XboxBridgeError,
};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use xbox_auth_flow::{AuthFlow, AuthFlowSeed, RefreshAndFinalizeInput};

static NEXT_HANDLE: AtomicU64 = AtomicU64::new(1);
static CLOUD_ACCESS: OnceLock<Mutex<HashMap<String, CloudAccessContext>>> = OnceLock::new();

#[derive(Debug, Clone, uniffi::Record)]
pub struct CloudAccessResult {
    pub auth_session: AuthSession,
    pub access_handle: String,
    pub account_id: String,
    pub region_host: String,
}

#[derive(Debug, Clone)]
pub(crate) struct CloudAccessContext {
    pub(crate) host: String,
    pub(crate) bearer_token: String,
    pub(crate) account_id: String,
}

#[uniffi::export(async_runtime = "tokio")]
pub async fn prepare_cloud_access(
    refresh_token: String,
    seed_json: String,
    force_region_ip: String,
) -> Result<CloudAccessResult, XboxBridgeError> {
    let seed: AuthFlowSeed = deserialize(&seed_json)?;
    let force_region_ip = normalize_force_region_ip(force_region_ip);
    let output = AuthFlow::new()
        .refresh_and_finalize(RefreshAndFinalizeInput {
            refresh_token,
            seed: seed.clone(),
            force_region_ip,
            include_streaming_tokens: true,
        })
        .await
        .map_err(|error| XboxBridgeError::Authentication(error.to_string()))?;

    let context = resolve_cloud_access_context(
        &output.auth_bundle.stream_tokens,
        &output.auth_bundle.web_token,
    )?;
    let auth_session = auth_session_from_bundle(output.auth_bundle, &seed)?;
    let access_handle = next_handle();
    let account_id = context.account_id.clone();
    let region_host = context.host.clone();

    cloud_access_registry()?.insert(access_handle.clone(), context);

    Ok(CloudAccessResult {
        auth_session,
        access_handle,
        account_id,
        region_host,
    })
}

#[uniffi::export]
pub fn release_cloud_access(access_handle: String) -> Result<(), XboxBridgeError> {
    cloud_access_registry()?.remove(access_handle.trim());
    Ok(())
}

pub(crate) fn load_cloud_access(
    access_handle: &str,
) -> Result<CloudAccessContext, XboxBridgeError> {
    cloud_access_registry()?
        .get(access_handle.trim())
        .cloned()
        .ok_or_else(|| XboxBridgeError::InvalidData("cloud access handle is invalid".to_string()))
}

fn resolve_cloud_access_context(
    stream_tokens: &Value,
    web_token: &Value,
) -> Result<CloudAccessContext, XboxBridgeError> {
    let token = stream_tokens
        .get("xCloudToken")
        .or_else(|| stream_tokens.get("xcloudToken"))
        .ok_or_else(|| {
            XboxBridgeError::Authentication(cloud_token_unavailable_message(stream_tokens))
        })?;
    let data = token.get("data").unwrap_or(token);
    let bearer_token = data
        .get("gsToken")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            XboxBridgeError::InvalidData("xCloud token is missing gsToken".to_string())
        })?;
    let regions = data
        .get("offeringSettings")
        .and_then(|value| value.get("regions"))
        .and_then(Value::as_array)
        .ok_or_else(|| {
            XboxBridgeError::InvalidData("xCloud token is missing regions".to_string())
        })?;
    let region = regions
        .iter()
        .find(|item| item.get("isDefault").and_then(Value::as_bool) == Some(true))
        .or_else(|| regions.first())
        .ok_or_else(|| XboxBridgeError::InvalidData("xCloud token has no region".to_string()))?;
    let base_uri = region
        .get("baseUri")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            XboxBridgeError::InvalidData("xCloud region is missing baseUri".to_string())
        })?;
    let host = base_uri
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/');
    if host.is_empty() {
        return Err(XboxBridgeError::InvalidData(
            "xCloud region host is empty".to_string(),
        ));
    }
    let claims = resolve_web_token_claims(web_token)?;
    let account_id = claims.xuid.unwrap_or(claims.uhs);

    Ok(CloudAccessContext {
        host: host.to_string(),
        bearer_token: bearer_token.to_string(),
        account_id,
    })
}

fn cloud_token_unavailable_message(stream_tokens: &Value) -> String {
    let diagnostics = stream_tokens
        .get("_diagnostics")
        .and_then(|value| value.get("xCloudToken"));
    let force_region_applied = diagnostics
        .and_then(|value| value.get("forceRegionApplied"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut parts = vec![format!(
        "xCloud token is unavailable; forceRegionApplied={force_region_applied}"
    )];
    for offering in ["xgpuweb", "xgpuwebf2p"] {
        let Some(detail) = diagnostics.and_then(|value| value.get(offering)) else {
            continue;
        };
        let error_kind = detail
            .get("errorKind")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let status_code = detail
            .get("statusCode")
            .and_then(Value::as_u64)
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_string());
        let timeout = detail
            .get("timeout")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let retriable = detail
            .get("retriable")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        parts.push(format!(
            "offering={offering},errorKind={error_kind},statusCode={status_code},timeout={timeout},retriable={retriable}"
        ));
    }
    parts.join("; ")
}

fn cloud_access_registry(
) -> Result<std::sync::MutexGuard<'static, HashMap<String, CloudAccessContext>>, XboxBridgeError> {
    CLOUD_ACCESS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|_| XboxBridgeError::InvalidData("cloud access registry is poisoned".to_string()))
}

fn next_handle() -> String {
    let sequence = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
    format!("cloud-{sequence:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn resolves_default_region_and_stable_xid() {
        let context = resolve_cloud_access_context(
            &json!({
                "xCloudToken": {
                    "data": {
                        "gsToken": "cloud-token",
                        "offeringSettings": {
                            "regions": [
                                { "baseUri": "https://first.example.com" },
                                {
                                    "isDefault": true,
                                    "baseUri": "https://default.example.com/"
                                }
                            ]
                        }
                    }
                }
            }),
            &json!({
                "data": {
                    "Token": "web-token",
                    "DisplayClaims": {
                        "xui": [{ "uhs": "volatile", "xid": "stable-xid" }]
                    }
                }
            }),
        )
        .expect("cloud access should resolve");

        assert_eq!(context.host, "default.example.com");
        assert_eq!(context.bearer_token, "cloud-token");
        assert_eq!(context.account_id, "stable-xid");
    }

    #[test]
    fn missing_cloud_token_is_rejected() {
        let result = resolve_cloud_access_context(
            &json!({
                "_diagnostics": {
                    "xCloudToken": {
                        "xgpuweb": {
                            "errorKind": "http",
                            "statusCode": 403,
                            "timeout": false,
                            "retriable": false
                        },
                        "xgpuwebf2p": {
                            "errorKind": "network",
                            "statusCode": null,
                            "timeout": true,
                            "retriable": true
                        },
                        "forceRegionApplied": false
                    }
                }
            }),
            &json!({}),
        );
        let error = result
            .expect_err("missing cloud token should fail")
            .to_string();
        assert!(error.contains("forceRegionApplied=false"));
        assert!(error.contains("offering=xgpuweb,errorKind=http,statusCode=403"));
        assert!(error.contains("offering=xgpuwebf2p,errorKind=network,statusCode=none"));
    }
}
