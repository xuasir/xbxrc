use crate::mods::auth::storage_repository::AuthStorageRepository;
use crate::mods::auth::types::{AuthTokenScope, SisuTokenData};
#[cfg(test)]
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde_json::Value;
pub use xbox_auth_flow::token_policy::cloud_token_unavailable_error;
pub use xbox_auth_flow::token_policy::ValidSessionSnapshot;
pub(super) use xbox_auth_flow::token_policy::{
    extract_gs_token, is_stream_token_valid_at, resolve_valid_web_api_tokens, same_web_account,
};
#[cfg(test)]
use xbox_auth_flow::token_policy::{
    parse_datetime_to_timestamp_millis, resolve_stream_token_expiry_ms,
};

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
        token.is_some_and(|token| {
            is_stream_token_valid_at(token, chrono::Utc::now().timestamp_millis())
        })
    }

    pub fn is_web_token_valid(&self, token: Option<&Value>) -> bool {
        token.is_some_and(|token| {
            resolve_valid_web_api_tokens(token, chrono::Utc::now().timestamp_millis()).is_some()
        })
    }

    pub fn get_valid_session_snapshot(&self) -> Result<Option<ValidSessionSnapshot>, String> {
        Ok(self
            .get_valid_session_for(AuthTokenScope::Web)?
            .filter(|snapshot| snapshot.app_level > 0))
    }

    pub fn get_valid_session_for(
        &self,
        scope: AuthTokenScope,
    ) -> Result<Option<ValidSessionSnapshot>, String> {
        let Some(web_token) = self.get_web_token()? else {
            return Ok(None);
        };
        let streaming_tokens = self
            .get_stream_tokens()?
            .unwrap_or_else(|| serde_json::json!({}));
        Ok(resolve_valid_session_for(
            &web_token,
            &streaming_tokens,
            scope,
            chrono::Utc::now().timestamp_millis(),
        ))
    }
}

fn resolve_valid_session_for(
    web: &Value,
    streams: &Value,
    scope: AuthTokenScope,
    now_ms: i64,
) -> Option<ValidSessionSnapshot> {
    xbox_auth_flow::token_policy::resolve_valid_session_for(
        web,
        streams,
        scope.flow_scope(),
        now_ms,
    )
}

pub(super) fn merge_stream_tokens(
    previous: Option<Value>,
    incoming: &Value,
    scope: AuthTokenScope,
    same_account: bool,
) -> Value {
    xbox_auth_flow::token_policy::merge_stream_tokens(
        previous,
        incoming,
        scope.flow_scope(),
        same_account,
    )
}

pub(super) fn can_reuse_sisu_user_token(token: &SisuTokenData, now_ms: i64) -> bool {
    xbox_auth_flow::token_policy::can_reuse_sisu_user_token_details(
        token.user_token.token.as_deref(),
        token.user_token.not_after.as_deref(),
        now_ms,
    )
}

#[cfg(test)]
#[path = "token_policy.test.rs"]
mod tests;
