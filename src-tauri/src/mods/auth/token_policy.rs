use base64::engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::mods::auth::storage_repository::AuthStorageRepository;
use crate::mods::auth::types::SisuTokenData;

const TOKEN_EXPIRY_SKEW_MS: i64 = 60 * 1000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidSessionSnapshot {
    pub app_level: u32,
    pub streaming_tokens: Value,
    pub web_token: Value,
}

pub struct AuthTokenPolicy {
    storage_repository: AuthStorageRepository,
}

impl AuthTokenPolicy {
    pub fn new(storage_repository: AuthStorageRepository) -> Self {
        Self { storage_repository }
    }

    pub fn get_stream_tokens(&self) -> Result<Option<Value>, String> {
        self.storage_repository.get_stream_tokens()
    }

    pub fn set_stream_tokens(&self, tokens: Value) -> Result<(), String> {
        self.storage_repository.set_stream_tokens(tokens)
    }

    pub fn get_web_token(&self) -> Result<Option<Value>, String> {
        self.storage_repository.get_web_token()
    }

    pub fn set_web_token(&self, token: Value) -> Result<(), String> {
        self.storage_repository.set_web_token(token)
    }

    pub fn clear_ephemeral_tokens(&self) -> Result<(), String> {
        self.storage_repository.clear_ephemeral_tokens()
    }

    pub fn get_cached_app_level(&self) -> Result<u32, String> {
        let stream_tokens = self.get_stream_tokens()?;
        let Some(stream_tokens) = stream_tokens else {
            return Ok(0);
        };

        if stream_tokens
            .get("xCloudToken")
            .is_some_and(|value| !value.is_null())
        {
            return Ok(2);
        }
        if stream_tokens
            .get("xHomeToken")
            .is_some_and(|value| !value.is_null())
        {
            return Ok(1);
        }
        Ok(0)
    }

    pub fn is_stream_token_valid(&self, token: Option<&Value>) -> bool {
        let Some(token) = token else {
            return false;
        };
        if token.is_null() {
            return false;
        }

        let Some(expires_at) = resolve_stream_token_expiry_ms(token) else {
            return false;
        };

        let now = chrono::Utc::now().timestamp_millis();
        expires_at - now > TOKEN_EXPIRY_SKEW_MS
    }

    pub fn is_web_token_valid(&self, token: Option<&Value>) -> bool {
        token.is_some_and(|token| {
            resolve_valid_web_api_tokens(token, chrono::Utc::now().timestamp_millis()).is_some()
        })
    }

    pub fn get_valid_session_snapshot(&self) -> Result<Option<ValidSessionSnapshot>, String> {
        let stream_tokens = self.get_stream_tokens()?;
        let web_token = self.get_web_token()?;

        let (Some(stream_tokens), Some(web_token)) = (stream_tokens, web_token) else {
            return Ok(None);
        };

        let web_valid = self.is_web_token_valid(Some(&web_token));
        if !web_valid {
            return Ok(None);
        }

        let xhome_valid = self.is_stream_token_valid(stream_tokens.get("xHomeToken"));
        let xcloud_valid = self.is_stream_token_valid(stream_tokens.get("xCloudToken"));

        if !xhome_valid && !xcloud_valid {
            return Ok(None);
        }

        Ok(Some(ValidSessionSnapshot {
            app_level: if xcloud_valid { 2 } else { 1 },
            streaming_tokens: stream_tokens,
            web_token,
        }))
    }
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

fn extract_gs_token(token: &Value) -> Option<&str> {
    token
        .get("data")
        .and_then(|data| data.get("gsToken"))
        .and_then(|value| value.as_str())
        .or_else(|| token.get("gsToken").and_then(|value| value.as_str()))
}

fn resolve_stream_token_expiry_ms(token: &Value) -> Option<i64> {
    if let (Some(create_time), Some(duration_seconds)) = (
        extract_stream_create_time_ms(token),
        extract_stream_duration_seconds(token),
    ) {
        if create_time > 0 && duration_seconds > 0 {
            return Some(create_time.saturating_add(duration_seconds.saturating_mul(1000)));
        }
    }

    let gs_token = extract_gs_token(token)?;
    let payload_segment = gs_token.split('.').nth(1)?;
    let payload_bytes = URL_SAFE_NO_PAD
        .decode(payload_segment)
        .or_else(|_| URL_SAFE.decode(payload_segment))
        .ok()?;
    let payload: Value = serde_json::from_slice(&payload_bytes).ok()?;
    let exp_seconds = payload.get("exp").and_then(parse_i64_value)?;
    Some(exp_seconds.saturating_mul(1000))
}

fn parse_datetime_to_timestamp_millis(input: &str) -> Option<i64> {
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

pub(super) fn resolve_valid_web_api_tokens(token: &Value, now_ms: i64) -> Option<(String, String)> {
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

pub(super) fn can_reuse_sisu_user_token(token: &SisuTokenData, now_ms: i64) -> bool {
    token
        .user_token
        .token
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
        && token
            .user_token
            .not_after
            .as_deref()
            .and_then(parse_datetime_to_timestamp_millis)
            .is_some_and(|expiry| expiry.saturating_sub(now_ms) > TOKEN_EXPIRY_SKEW_MS)
}

#[cfg(test)]
#[path = "token_policy.test.rs"]
mod tests;
