use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::mods::auth::repository::CoreTokenRepository;

const TOKEN_EXPIRY_SKEW_MS: i64 = 60 * 1000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidSessionSnapshot {
    pub app_level: u32,
    pub streaming_tokens: Value,
    pub web_token: Value,
}

pub struct AuthTokenRepository {
    core_repository: CoreTokenRepository,
}

impl AuthTokenRepository {
    pub fn new(core_repository: CoreTokenRepository) -> Self {
        Self { core_repository }
    }

    pub fn get_stream_tokens(&self) -> Result<Option<Value>, String> {
        self.core_repository.get_stream_tokens()
    }

    pub fn set_stream_tokens(&self, tokens: Value) -> Result<(), String> {
        self.core_repository.set_stream_tokens(tokens)
    }

    pub fn get_web_token(&self) -> Result<Option<Value>, String> {
        self.core_repository.get_web_token()
    }

    pub fn set_web_token(&self, token: Value) -> Result<(), String> {
        self.core_repository.set_web_token(token)
    }

    pub fn clear_ephemeral_tokens(&self) -> Result<(), String> {
        self.core_repository.clear_ephemeral_tokens()
    }

    pub fn clear_all_tokens(&self) -> Result<(), String> {
        self.core_repository.clear_all_tokens()
    }

    pub fn has_identity_token(&self) -> Result<bool, String> {
        Ok(self.core_repository.get_user_token()?.is_some())
    }

    pub fn get_cached_app_level(&self) -> Result<u32, String> {
        let stream_tokens = self.get_stream_tokens()?;
        let Some(stream_tokens) = stream_tokens else {
            return Ok(0);
        };

        if stream_tokens.get("xCloudToken").is_some() {
            return Ok(2);
        }
        if stream_tokens.get("xHomeToken").is_some() {
            return Ok(1);
        }
        Ok(0)
    }

    pub fn is_stream_token_valid(&self, token: Option<&Value>) -> bool {
        let Some(token) = token else {
            return false;
        };

        let create_time = token
            .get("_objectCreateTime")
            .and_then(|value| value.as_i64())
            .unwrap_or(0);

        let duration = token
            .get("data")
            .and_then(|data| data.get("durationInSeconds"))
            .and_then(|value| value.as_i64())
            .or_else(|| {
                token
                    .get("durationInSeconds")
                    .and_then(|value| value.as_i64())
            })
            .unwrap_or(0);

        if create_time <= 0 || duration <= 0 {
            return false;
        }

        let expires_at = create_time + duration * 1000;
        let now = chrono::Utc::now().timestamp_millis();
        expires_at - now > TOKEN_EXPIRY_SKEW_MS
    }

    pub fn is_web_token_valid(&self, token: Option<&Value>) -> bool {
        let Some(token) = token else {
            return false;
        };

        let not_after = token
            .get("data")
            .and_then(|data| data.get("NotAfter"))
            .and_then(|value| value.as_str())
            .or_else(|| token.get("NotAfter").and_then(|value| value.as_str()));

        let Some(not_after) = not_after else {
            return false;
        };

        let Ok(expires_at) = chrono::DateTime::parse_from_rfc3339(not_after) else {
            return false;
        };

        let now = chrono::Utc::now().timestamp_millis();
        expires_at.timestamp_millis() - now > TOKEN_EXPIRY_SKEW_MS
    }

    pub fn get_valid_session_snapshot(&self) -> Result<Option<ValidSessionSnapshot>, String> {
        let stream_tokens = self.get_stream_tokens()?;
        let web_token = self.get_web_token()?;

        let (Some(stream_tokens), Some(web_token)) = (stream_tokens, web_token) else {
            return Ok(None);
        };

        if !self.is_web_token_valid(Some(&web_token)) {
            return Ok(None);
        }

        let xhome_valid = self.is_stream_token_valid(stream_tokens.get("xHomeToken"));
        let xcloud_valid = self.is_stream_token_valid(stream_tokens.get("xCloudToken"));
        eprintln!(
            "[auth][token] snapshot validate web={} xhome={} xcloud={}",
            self.is_web_token_valid(Some(&web_token)),
            xhome_valid,
            xcloud_valid
        );

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
