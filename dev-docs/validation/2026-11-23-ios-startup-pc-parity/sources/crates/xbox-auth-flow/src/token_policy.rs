//! PC 与 iOS 共用的凭据有效期、作用域与账户合并规则。
use crate::DownstreamTokenScope;
use base64::engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const TOKEN_EXPIRY_SKEW_MS: i64 = 60 * 1000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidSessionSnapshot {
    pub app_level: u32,
    pub streaming_tokens: Value,
    pub web_token: Value,
}

pub fn is_stream_token_valid_at(token: &Value, now_ms: i64) -> bool {
    extract_gs_token(token).is_some_and(|token| !token.trim().is_empty())
        && resolve_stream_token_expiry_ms(token)
            .is_some_and(|expiry| expiry.saturating_sub(now_ms) > TOKEN_EXPIRY_SKEW_MS)
}

pub fn resolve_valid_session_for(
    web_token: &Value,
    streaming_tokens: &Value,
    scope: DownstreamTokenScope,
    now_ms: i64,
) -> Option<ValidSessionSnapshot> {
    resolve_valid_web_api_tokens(web_token, now_ms)?;
    let home_valid = streaming_tokens
        .get("xHomeToken")
        .is_some_and(|token| is_stream_token_valid_at(token, now_ms));
    let cloud_valid = streaming_tokens
        .get("xCloudToken")
        .is_some_and(|token| is_stream_token_valid_at(token, now_ms));
    let scope_valid = match scope {
        DownstreamTokenScope::Web => true,
        DownstreamTokenScope::Home => home_valid,
        DownstreamTokenScope::Cloud => cloud_valid,
        DownstreamTokenScope::All => home_valid && cloud_valid,
    };
    scope_valid.then(|| ValidSessionSnapshot {
        web_token: web_token.clone(),
        streaming_tokens: streaming_tokens.clone(),
        app_level: if cloud_valid {
            2
        } else if home_valid {
            1
        } else {
            0
        },
    })
}

fn parse_i64_value(value: &Value) -> Option<i64> {
    if let Some(number) = value.as_i64() {
        return Some(number);
    }

    value.as_str().and_then(|raw| raw.parse::<i64>().ok())
}

fn extract_stream_duration_seconds(token: &Value) -> Option<i64> {
    token
        .get("data")
        .and_then(|data| data.get("durationInSeconds"))
        .and_then(parse_i64_value)
        .or_else(|| token.get("durationInSeconds").and_then(parse_i64_value))
        .or_else(|| {
            token
                .get("data")
                .and_then(|data| data.get("duration_in_seconds"))
                .and_then(parse_i64_value)
        })
}

fn extract_stream_create_time_ms(token: &Value) -> Option<i64> {
    token
        .get("_objectCreateTime")
        .and_then(parse_i64_value)
        .or_else(|| {
            token
                .get("data")
                .and_then(|data| data.get("_objectCreateTime"))
                .and_then(parse_i64_value)
        })
}

pub fn extract_gs_token(token: &Value) -> Option<&str> {
    token
        .get("data")
        .and_then(|data| data.get("gsToken"))
        .and_then(|value| value.as_str())
        .or_else(|| token.get("gsToken").and_then(|value| value.as_str()))
}

pub fn resolve_stream_token_expiry_ms(token: &Value) -> Option<i64> {
    let duration_expiry = if let (Some(create_time), Some(duration_seconds)) = (
        extract_stream_create_time_ms(token),
        extract_stream_duration_seconds(token),
    ) {
        if create_time > 0 && duration_seconds > 0 {
            Some(create_time.saturating_add(duration_seconds.saturating_mul(1000)))
        } else {
            None
        }
    } else {
        None
    };
    let jwt_expiry = extract_gs_token(token).and_then(|gs_token| {
        let payload_segment = gs_token.split('.').nth(1)?;
        let payload_bytes = URL_SAFE_NO_PAD
            .decode(payload_segment)
            .or_else(|_| URL_SAFE.decode(payload_segment))
            .ok()?;
        let payload: Value = serde_json::from_slice(&payload_bytes).ok()?;
        Some(
            payload
                .get("exp")
                .and_then(parse_i64_value)?
                .saturating_mul(1000),
        )
    });
    match (duration_expiry, jwt_expiry) {
        (Some(duration), Some(jwt)) => Some(duration.min(jwt)),
        (duration, jwt) => duration.or(jwt),
    }
}

pub fn same_web_account(previous: &Value, current: &Value) -> bool {
    fn claims(value: &Value) -> Option<(Option<&str>, &str)> {
        let claims = value
            .get("data")
            .unwrap_or(value)
            .get("DisplayClaims")?
            .get("xui")?
            .as_array()?
            .first()?;
        Some((
            claims
                .get("xid")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty()),
            claims.get("uhs")?.as_str()?.trim(),
        ))
    }
    claims(previous)
        .zip(claims(current))
        .is_some_and(
            |((old_xid, old_uhs), (new_xid, new_uhs))| match (old_xid, new_xid) {
                (Some(old), Some(new)) => old == new,
                _ => old_uhs == new_uhs,
            },
        )
}

pub fn merge_stream_tokens(
    previous: Option<Value>,
    incoming: &Value,
    scope: DownstreamTokenScope,
    same_account: bool,
) -> Value {
    let mut merged = if same_account {
        previous
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default()
    } else {
        serde_json::Map::new()
    };
    if let Some(key) = scope.stream_key() {
        merged.remove(key);
    }
    if scope == DownstreamTokenScope::Cloud {
        merged.remove("_diagnostics");
    }
    if let Some(incoming) = incoming.as_object() {
        merged.extend(incoming.clone());
    }
    Value::Object(merged)
}

pub fn cloud_token_unavailable_error(tokens: &Value) -> Option<xbox_webapi::WebApiError> {
    use xbox_webapi::{NetworkErrorKind, WebApiError};
    for offering in ["xgpuwebf2p", "xgpuweb"] {
        let diagnostic = &tokens["_diagnostics"]["xCloudToken"][offering];
        let kind = match diagnostic["networkErrorKind"].as_str() {
            Some("dns") => Some(NetworkErrorKind::Dns),
            Some("connect") => Some(NetworkErrorKind::Connect),
            Some("tls") => Some(NetworkErrorKind::Tls),
            Some("timeout") => Some(NetworkErrorKind::Timeout),
            Some("body") => Some(NetworkErrorKind::Body),
            Some("request") => Some(NetworkErrorKind::Request),
            _ => None,
        };
        if let Some(kind) = kind {
            return Some(WebApiError::network_kind(kind));
        }
        // 兼容已有 diagnostics 的 timeout 布尔字段，保留同一网络事实。
        if diagnostic["timeout"].as_bool() == Some(true) {
            return Some(WebApiError::network_kind(NetworkErrorKind::Timeout));
        }
        if let Some(status) = diagnostic["statusCode"]
            .as_u64()
            .filter(|status| (400..=599).contains(status))
        {
            return Some(
                match diagnostic["serviceCode"]
                    .as_str()
                    .and_then(xbox_webapi::sanitize_service_code)
                {
                    Some(code) => {
                        WebApiError::http_with_code(status as u16, code, "cloud access unavailable")
                    }
                    None => WebApiError::http(status as u16, "cloud access unavailable"),
                },
            );
        }
    }
    None
}

pub fn parse_datetime_to_timestamp_millis(input: &str) -> Option<i64> {
    if let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(input) {
        return Some(parsed.timestamp_millis());
    }

    if let Ok(parsed) = chrono::DateTime::parse_from_str(input, "%Y-%m-%dT%H:%M:%S%.f%z") {
        return Some(parsed.timestamp_millis());
    }

    if let Ok(parsed) = chrono::NaiveDateTime::parse_from_str(input, "%Y-%m-%dT%H:%M:%S%.f") {
        return Some(parsed.and_utc().timestamp_millis());
    }

    None
}

pub fn resolve_valid_web_api_tokens(token: &Value, now_ms: i64) -> Option<(String, String)> {
    let data = token.get("data").unwrap_or(token);
    let expires_at = data
        .get("NotAfter")
        .or_else(|| data.get("not_after"))
        .or_else(|| token.get("NotAfter"))
        .or_else(|| token.get("not_after"))
        .and_then(Value::as_str)
        .and_then(parse_datetime_to_timestamp_millis)?;
    // 给请求与重试预留同一份有效期余量，过期快照进入既有恢复流程。
    if expires_at.saturating_sub(now_ms) <= TOKEN_EXPIRY_SKEW_MS {
        return None;
    }
    let token = data.get("Token")?.as_str()?.trim();
    let uhs = data
        .get("DisplayClaims")?
        .get("xui")?
        .as_array()?
        .first()?
        .get("uhs")?
        .as_str()?
        .trim();
    if token.is_empty() || uhs.is_empty() {
        return None;
    }
    Some((uhs.to_string(), token.to_string()))
}

pub fn can_reuse_sisu_user_token(token: &crate::FlowSisuTokenData, now_ms: i64) -> bool {
    can_reuse_sisu_user_token_details(
        token.user_token.token.as_deref(),
        token.user_token.not_after.as_deref(),
        now_ms,
    )
}

pub fn can_reuse_sisu_user_token_details(
    token: Option<&str>,
    not_after: Option<&str>,
    now_ms: i64,
) -> bool {
    token.is_some_and(|value| !value.trim().is_empty())
        && not_after
            .and_then(parse_datetime_to_timestamp_millis)
            .is_some_and(|expiry| expiry.saturating_sub(now_ms) > TOKEN_EXPIRY_SKEW_MS)
}

#[cfg(test)]
#[path = "token_policy.test.rs"]
mod tests;
