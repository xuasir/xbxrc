use crate::mods::auth::AuthProviderRef;
use crate::mods::data::types::DataSessionContext;
use serde_json::{json, Value};

pub struct DataSessionResolver {
    auth_provider: AuthProviderRef,
}

impl DataSessionResolver {
    pub fn new(auth_provider: AuthProviderRef) -> Self {
        Self { auth_provider }
    }

    pub async fn ensure_authenticated_session(&self) -> Result<Option<DataSessionContext>, String> {
        if let Some(event) = self
            .auth_provider
            .get_active_session()
            .map_err(|error| error.to_string())?
        {
            return Ok(Some(DataSessionContext {
                provider: event.provider,
                app_level: event.app_level,
                streaming_tokens: event.streaming_tokens,
                web_token: event.web_token,
            }));
        }

        if let Some((uhs, token)) = self
            .auth_provider
            .get_web_api_tokens()
            .map_err(|error| error.to_string())?
        {
            return Ok(Some(self.build_web_only_session(&uhs, &token)));
        }

        let _ = self
            .auth_provider
            .check_authentication()
            .await
            .map_err(|error| error.to_string())?;

        if let Some(event) = self
            .auth_provider
            .get_active_session()
            .map_err(|error| error.to_string())?
        {
            return Ok(Some(DataSessionContext {
                provider: event.provider,
                app_level: event.app_level,
                streaming_tokens: event.streaming_tokens,
                web_token: event.web_token,
            }));
        }

        if let Some((uhs, token)) = self
            .auth_provider
            .get_web_api_tokens()
            .map_err(|error| error.to_string())?
        {
            return Ok(Some(self.build_web_only_session(&uhs, &token)));
        }

        Ok(None)
    }

    // 对齐迁移前语义：当 stream 快照暂不可用时，允许用 web token 先提供 profile/hosts 查询。
    fn build_web_only_session(&self, uhs: &str, token: &str) -> DataSessionContext {
        let state = self.auth_provider.get_state();
        DataSessionContext {
            provider: state.provider,
            app_level: 0,
            streaming_tokens: Value::Object(serde_json::Map::new()),
            web_token: json!({
                "data": {
                    "Token": token,
                    "DisplayClaims": {
                        "xui": [{ "uhs": uhs }]
                    }
                }
            }),
        }
    }
}

#[derive(Debug, Clone)]
pub struct WebTokenClaims {
    pub user_token: String,
    pub uhs: String,
}

// 从 webToken 快照中提取 xbox-webapi 初始化与 console 命令所需声明。
pub fn resolve_web_token_claims(raw: &Value) -> Option<WebTokenClaims> {
    let token = raw.get("data").unwrap_or(raw);

    let user_token = token.get("Token").and_then(|value| value.as_str())?;
    let uhs = token
        .get("DisplayClaims")
        .and_then(|value| value.get("xui"))
        .and_then(|value| value.as_array())
        .and_then(|xui| xui.first())
        .and_then(|value| value.get("uhs"))
        .and_then(|value| value.as_str())?;

    if user_token.is_empty() || uhs.is_empty() {
        return None;
    }

    Some(WebTokenClaims {
        user_token: user_token.to_string(),
        uhs: uhs.to_string(),
    })
}
