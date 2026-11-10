use crate::mods::auth::{AuthProviderRef, AuthTokenScope};
use crate::mods::data::types::DataSessionContext;
use serde_json::Value;
use xbox_webapi::WebApiError;

#[derive(Clone)]
pub struct DataSessionResolver {
    auth_provider: AuthProviderRef,
}

impl DataSessionResolver {
    pub fn new(auth_provider: AuthProviderRef) -> Self {
        Self { auth_provider }
    }

    pub async fn ensure_authenticated_session(&self) -> Result<Option<DataSessionContext>, String> {
        self.ensure_session()
            .await
            .map_err(|error| super::services::personal_service::user_error(&error))
    }

    pub(crate) async fn ensure_session(&self) -> Result<Option<DataSessionContext>, WebApiError> {
        self.ensure_session_for(AuthTokenScope::Web).await
    }

    pub(crate) async fn ensure_session_for(
        &self,
        scope: AuthTokenScope,
    ) -> Result<Option<DataSessionContext>, WebApiError> {
        if scope != AuthTokenScope::Web {
            let target = scope.as_str();
            let token = self
                .auth_provider
                .get_streaming_token(target)
                .await
                .map_err(auth_query_error)?;
            return match token {
                Some(_) => self.current_session(),
                None => Ok(None),
            };
        }
        if let Some(session) = self.current_session()? {
            return Ok(Some(session));
        }
        self.auth_provider
            .check_web_authentication()
            .await
            .map_err(auth_query_error)?;
        self.current_session()
    }

    pub fn current_session(&self) -> Result<Option<DataSessionContext>, WebApiError> {
        if let Some(event) = self
            .auth_provider
            .get_active_session()
            .map_err(auth_query_error)?
        {
            return Ok(Some(DataSessionContext {
                provider: event.provider,
                app_level: event.app_level,
                streaming_tokens: event.streaming_tokens,
                web_token: event.web_token,
            }));
        }

        if let Some(web_token) = self
            .auth_provider
            .get_web_api_token_snapshot()
            .map_err(auth_query_error)?
        {
            return Ok(Some(DataSessionContext {
                provider: self.auth_provider.get_state().provider,
                app_level: 0,
                streaming_tokens: Value::Object(serde_json::Map::new()),
                web_token,
            }));
        }

        Ok(None)
    }

    /// 明确 401 拒绝后恢复一次；整个过程保持原账户边界。
    pub async fn run_query<T, F, Fut>(&self, request: F) -> Result<T, WebApiError>
    where
        F: Fn(DataSessionContext) -> Fut,
        Fut: std::future::Future<Output = Result<T, WebApiError>>,
    {
        self.run_scoped_query(AuthTokenScope::Web, request).await
    }

    pub(crate) async fn run_scoped_query<T, F, Fut>(
        &self,
        scope: AuthTokenScope,
        request: F,
    ) -> Result<T, WebApiError>
    where
        F: Fn(DataSessionContext) -> Fut,
        Fut: std::future::Future<Output = Result<T, WebApiError>>,
    {
        let mut session = self
            .ensure_session_for(scope)
            .await?
            .ok_or_else(|| WebApiError::auth("请先登录 Xbox 账户"))?;
        let claims = resolve_web_token_claims(&session.web_token)
            .ok_or_else(|| WebApiError::auth("登录信息暂不可用，请重新登录"))?;
        let mut result = request(session.clone()).await;
        if result
            .as_ref()
            .err()
            .is_some_and(|error| error.to_status_code() == Some(401))
        {
            match scope.stream_key() {
                Some(key) => {
                    let rejected = session
                        .streaming_tokens
                        .get(key)
                        .and_then(|token| token.get("data").unwrap_or(token).get("gsToken"))
                        .and_then(Value::as_str)
                        .ok_or_else(|| WebApiError::parse("串流凭据响应暂不可用，请重试"))?;
                    self.auth_provider
                        .recover_streaming_token(
                            scope.as_str(),
                            rejected,
                            session.streaming_tokens[key]["_authGeneration"].as_u64(),
                        )
                        .await
                        .map_err(auth_query_error)?;
                }
                None => self
                    .auth_provider
                    .recover_web_api_session(&claims.user_token)
                    .await
                    .map_err(auth_query_error)?,
            }
            session = self
                .current_session()?
                .ok_or_else(|| WebApiError::auth("登录已过期，请重新登录"))?;
            let current = resolve_web_token_claims(&session.web_token)
                .ok_or_else(|| WebApiError::auth("登录信息暂不可用，请重新登录"))?;
            if !same_account(&claims, &current) {
                return Err(WebApiError::parse("登录状态已更新，请重新查询"));
            }
            result = request(session.clone()).await;
        }
        let current = if scope.stream_key().is_some() {
            self.is_current_streaming_session(&session, scope)
        } else {
            self.is_current_session(&session)
        };
        if !current {
            return Err(WebApiError::parse("登录状态已更新，请重新查询"));
        }
        result
    }

    pub fn is_current_streaming_session(
        &self,
        session: &DataSessionContext,
        scope: AuthTokenScope,
    ) -> bool {
        self.is_current_session(session)
            && self
                .current_session()
                .ok()
                .flatten()
                .is_some_and(|current| {
                    scope.stream_key().is_some_and(|key| {
                        current.streaming_tokens.get(key) == session.streaming_tokens.get(key)
                    })
                })
    }

    pub fn is_current_session(&self, session: &DataSessionContext) -> bool {
        let previous = resolve_web_token_claims(&session.web_token);
        let current = self
            .current_session()
            .ok()
            .flatten()
            .and_then(|session| resolve_web_token_claims(&session.web_token));
        previous
            .zip(current)
            .is_some_and(|(previous, current)| same_account(&previous, &current))
    }
}

fn auth_query_error(error: crate::error::AppError) -> WebApiError {
    match error {
        crate::error::AppError::WebApi(error) => error,
        crate::error::AppError::Auth(_) => WebApiError::auth("登录续期暂不可用，请选择重新登录"),
        _ => WebApiError::parse("认证续期暂不可用，请重试"),
    }
}

fn same_account(previous: &WebTokenClaims, current: &WebTokenClaims) -> bool {
    match (&previous.xid, &current.xid) {
        (Some(previous), Some(current)) => previous == current,
        _ => previous.uhs == current.uhs,
    }
}

#[derive(Debug, Clone)]
pub struct WebTokenClaims {
    pub user_token: String,
    pub uhs: String,
    pub xid: Option<String>,
}

impl WebTokenClaims {
    pub fn account_scope(&self) -> &str {
        self.xid.as_deref().unwrap_or(&self.uhs)
    }
}

// 从 webToken 快照中提取 xbox-webapi 初始化与 console 命令所需声明。
pub fn resolve_web_token_claims(raw: &Value) -> Option<WebTokenClaims> {
    let token = raw.get("data").unwrap_or(raw);

    let user_token = token.get("Token").and_then(|value| value.as_str())?.trim();
    let xui = token
        .get("DisplayClaims")
        .and_then(|value| value.get("xui"))
        .and_then(|value| value.as_array())
        .and_then(|xui| xui.first())?;

    let uhs = xui.get("uhs").and_then(|value| value.as_str())?.trim();
    let xid = xui
        .get("xid")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    if user_token.is_empty() || uhs.is_empty() {
        return None;
    }

    Some(WebTokenClaims {
        user_token: user_token.to_string(),
        uhs: uhs.to_string(),
        xid,
    })
}

#[cfg(test)]
#[path = "session_resolver.test.rs"]
mod tests;
