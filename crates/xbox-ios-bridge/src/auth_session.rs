//! iOS 安全存储投影；凭据判定与续期协议消费 PC 共用的 Rust 实现。
use crate::{deserialize, serialize, AuthSession, XboxBridgeError, XboxQueryFailure};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use xbox_auth_flow::token_policy::{
    merge_stream_tokens, resolve_valid_session_for, same_web_account,
};
use xbox_auth_flow::{
    renew_auth_tokens, AuthBundle, AuthFlow, AuthFlowObserver, AuthFlowSeed, AuthRenewalBackend,
    AuthRenewalError, AuthRenewalObserver, DownstreamTokenScope, FlowSisuTokenData,
    RenewAuthTokensInput,
};
use xbox_webapi::OAuthTokenResponse;

#[derive(Debug, Clone, Copy, uniffi::Enum)]
pub enum XboxAuthScope {
    Web,
    Home,
    Cloud,
}
impl XboxAuthScope {
    pub(crate) fn flow_scope(self) -> DownstreamTokenScope {
        match self {
            Self::Web => DownstreamTokenScope::Web,
            Self::Home => DownstreamTokenScope::Home,
            Self::Cloud => DownstreamTokenScope::Cloud,
        }
    }
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct XboxAuthCheckpoint {
    pub previous_refresh_token: String,
    pub auth_session: AuthSession,
    pub core_only: bool,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct XboxAuthStage {
    pub stage: String,
    pub request_stage: Option<String>,
    pub scope: Option<String>,
    pub elapsed_ms: Option<u64>,
    pub skip_core_refresh: Option<bool>,
    pub network_error_kind: Option<String>,
    pub failure: Option<XboxQueryFailure>,
}

#[uniffi::export(foreign)]
#[async_trait]
pub trait XboxAuthObserver: Send + Sync {
    async fn checkpoint(&self, update: XboxAuthCheckpoint) -> Result<(), XboxBridgeError>;
    fn on_stage(&self, event: XboxAuthStage);
}

/// 会话续期回到同一个安全存储 owner，复用宿主已有认证事务与当前缓存。
#[uniffi::export(foreign)]
#[async_trait]
pub trait XboxStreamingAuthProvider: Send + Sync {
    async fn ensure_session(
        &self,
        target_type: String,
        rejected_token: Option<String>,
    ) -> Result<AuthSession, XboxBridgeError>;
}

pub(crate) fn token_was_rejected(
    session: &AuthSession,
    scope: DownstreamTokenScope,
    rejected: Option<&str>,
) -> Result<bool, XboxBridgeError> {
    let Some(rejected) = rejected else {
        return Ok(false);
    };
    let web: Value = deserialize(&session.web_token_json)?;
    let cache = AuthTokenCache::from_session(session)?;
    let current = match scope.stream_key() {
        Some(key) => cache
            .as_ref()
            .and_then(|cache| cache.stream_tokens.get(key))
            .and_then(xbox_auth_flow::token_policy::extract_gs_token),
        None => web
            .get("data")
            .unwrap_or(&web)
            .get("Token")
            .and_then(Value::as_str),
    };
    Ok(current == Some(rejected))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AuthTokenCache {
    pub(crate) schema_version: u8,
    pub(crate) user_token: Option<OAuthTokenResponse>,
    pub(crate) sisu_token: Option<FlowSisuTokenData>,
    pub(crate) stream_tokens: Value,
    pub(crate) cloud_force_region_ip: String,
    pub(crate) force_core_refresh: bool,
    #[serde(default)]
    pub(crate) core_refresh_scopes: Vec<String>,
}

impl AuthTokenCache {
    fn empty() -> Self {
        Self {
            schema_version: 1,
            user_token: None,
            sisu_token: None,
            stream_tokens: json!({}),
            cloud_force_region_ip: String::new(),
            force_core_refresh: false,
            core_refresh_scopes: Vec::new(),
        }
    }

    fn requires_core_refresh(&self, scope: DownstreamTokenScope) -> bool {
        self.force_core_refresh
            || self
                .core_refresh_scopes
                .iter()
                .any(|pending| pending == scope.as_str())
    }

    fn require_core_refresh(&mut self, scope: DownstreamTokenScope) {
        if scope == DownstreamTokenScope::Web {
            self.force_core_refresh = true;
        } else if !self
            .core_refresh_scopes
            .iter()
            .any(|pending| pending == scope.as_str())
        {
            self.core_refresh_scopes.push(scope.as_str().into());
        }
    }

    pub(crate) fn from_session(session: &AuthSession) -> Result<Option<Self>, XboxBridgeError> {
        let Some(raw) = &session.token_cache_json else {
            return Ok(None);
        };
        let cache: Self = deserialize(raw)?;
        if cache.schema_version != 1 {
            return Ok(None);
        }
        if cache
            .user_token
            .as_ref()
            .is_some_and(|token| token.refresh_token != session.refresh_token)
        {
            return Err(XboxBridgeError::InvalidData(
                "credentialCacheRevisionMismatch".into(),
            ));
        }
        Ok(Some(cache))
    }
}

pub(crate) fn session_from_bundle(
    bundle: AuthBundle,
    seed: &AuthFlowSeed,
    cloud_force_region_ip: String,
) -> Result<AuthSession, XboxBridgeError> {
    let cache = AuthTokenCache {
        schema_version: 1,
        user_token: Some(bundle.user_token.clone()),
        sisu_token: Some(bundle.sisu_token),
        stream_tokens: bundle.stream_tokens,
        cloud_force_region_ip,
        force_core_refresh: false,
        core_refresh_scopes: Vec::new(),
    };
    Ok(AuthSession {
        refresh_token: bundle.user_token.refresh_token,
        seed_json: serialize(seed)?,
        web_token_json: serialize(&bundle.web_token)?,
        app_level: bundle.app_level,
        token_cache_json: Some(serialize(&cache)?),
    })
}

struct BridgeAuthRenewal<'a> {
    host: &'a dyn XboxAuthObserver,
    current: &'a AuthSession,
    cache: Option<&'a AuthTokenCache>,
    scope: DownstreamTokenScope,
}
impl AuthFlowObserver for BridgeAuthRenewal<'_> {
    fn stage(&self, stage: &'static str, facts: Value) {
        self.host.on_stage(XboxAuthStage {
            stage: stage.into(),
            request_stage: facts["requestStage"].as_str().map(str::to_owned),
            scope: facts["scope"].as_str().map(str::to_owned),
            elapsed_ms: facts["elapsedMs"].as_u64(),
            skip_core_refresh: facts["skipCoreRefresh"].as_bool(),
            // 固定分类穿过 FFI；底层 source 正文留在 HTTP 边界内。
            network_error_kind: facts["networkErrorKind"]
                .as_str()
                .filter(|kind| {
                    matches!(
                        *kind,
                        "dns" | "connect" | "tls" | "timeout" | "body" | "request" | "unknown"
                    )
                })
                .map(str::to_owned),
            failure: facts
                .get("errorKind")
                .map(|_| XboxQueryFailure::from_safe_summary(&facts)),
        });
    }
}
#[async_trait]
impl AuthRenewalObserver for BridgeAuthRenewal<'_> {
    type Error = XboxBridgeError;
    async fn checkpoint_credentials(
        &self,
        token: &OAuthTokenResponse,
    ) -> Result<(), XboxBridgeError> {
        let mut cache = self.cache.cloned().unwrap_or_else(AuthTokenCache::empty);
        cache.user_token = Some(token.clone());
        // 保留其他目标的有效缓存；当前目标的核心恢复在下游成功后解除。
        cache.require_core_refresh(self.scope);
        let mut session = self.current.clone();
        session.refresh_token = token.refresh_token.clone();
        session.token_cache_json = Some(serialize(&cache)?);
        self.host
            .checkpoint(XboxAuthCheckpoint {
                previous_refresh_token: self.current.refresh_token.clone(),
                auth_session: session,
                core_only: true,
            })
            .await
    }
}

pub(crate) async fn renew_auth_session(
    current: AuthSession,
    force_region_ip: String,
    scope: DownstreamTokenScope,
    force_core_refresh: bool,
    observer: Arc<dyn XboxAuthObserver>,
) -> Result<AuthSession, XboxBridgeError> {
    Box::pin(renew_auth_session_with(
        &AuthFlow::new(),
        current,
        force_region_ip,
        scope,
        force_core_refresh,
        observer.as_ref(),
    ))
    .await
}

async fn renew_auth_session_with<B: AuthRenewalBackend>(
    backend: &B,
    mut current: AuthSession,
    force_region_ip: String,
    scope: DownstreamTokenScope,
    force_core_refresh: bool,
    host: &dyn XboxAuthObserver,
) -> Result<AuthSession, XboxBridgeError> {
    let mut cache = AuthTokenCache::from_session(&current)?;
    let web: Value = deserialize(&current.web_token_json)?;
    if force_core_refresh
        && !cache
            .as_ref()
            .is_some_and(|cache| cache.requires_core_refresh(scope))
    {
        let pending = cache.get_or_insert_with(AuthTokenCache::empty);
        pending.require_core_refresh(scope);
        current.token_cache_json = Some(serialize(pending)?);
        // 与 PC 的拒绝状态一致：OAuth 请求失败后的重试继续恢复当前目标。
        host.checkpoint(XboxAuthCheckpoint {
            previous_refresh_token: current.refresh_token.clone(),
            auth_session: current.clone(),
            core_only: true,
        })
        .await?;
    }
    let force_core_refresh = force_core_refresh
        || cache
            .as_ref()
            .is_some_and(|cache| cache.requires_core_refresh(scope));
    let region_changed = scope == DownstreamTokenScope::Cloud
        && cache
            .as_ref()
            .is_some_and(|cache| cache.cloud_force_region_ip != force_region_ip);
    if !force_core_refresh && !region_changed {
        let empty_streams = json!({});
        if let Some(snapshot) = resolve_valid_session_for(
            &web,
            cache
                .as_ref()
                .map(|cache| &cache.stream_tokens)
                .unwrap_or(&empty_streams),
            scope,
            chrono::Utc::now().timestamp_millis(),
        ) {
            host.on_stage(XboxAuthStage {
                stage: "validCredentialsReused".into(),
                request_stage: None,
                scope: Some(scope.as_str().into()),
                elapsed_ms: None,
                skip_core_refresh: Some(true),
                network_error_kind: None,
                failure: None,
            });
            return Ok(AuthSession {
                app_level: snapshot.app_level,
                ..current
            });
        }
    }
    let seed: AuthFlowSeed = deserialize(&current.seed_json)?;
    let observer = BridgeAuthRenewal {
        host,
        current: &current,
        cache: cache.as_ref(),
        scope,
    };
    let mut bundle = Box::pin(renew_auth_tokens(
        backend,
        RenewAuthTokensInput {
            refresh_token: current.refresh_token.clone(),
            user_token: cache.as_ref().and_then(|cache| cache.user_token.clone()),
            sisu_token: cache.as_ref().and_then(|cache| cache.sisu_token.clone()),
            seed: seed.clone(),
            force_region_ip: force_region_ip.clone(),
            scope,
            force_core_refresh,
            refresh_token_rejected: false,
        },
        &observer,
    ))
    .await
    .map_err(|error| match error {
        AuthRenewalError::Flow(error) => crate::authentication_error(error),
        AuthRenewalError::Checkpoint(error) => error,
    })?;
    let same_account = same_web_account(&web, &bundle.web_token);
    bundle.stream_tokens = merge_stream_tokens(
        cache.as_ref().map(|cache| cache.stream_tokens.clone()),
        &bundle.stream_tokens,
        scope,
        same_account,
    );
    bundle.app_level = resolve_valid_session_for(
        &bundle.web_token,
        &bundle.stream_tokens,
        DownstreamTokenScope::Web,
        chrono::Utc::now().timestamp_millis(),
    )
    .map(|snapshot| snapshot.app_level)
    .unwrap_or(bundle.app_level);
    let cloud_route = if scope == DownstreamTokenScope::Cloud {
        force_region_ip
    } else {
        cache
            .as_ref()
            .map(|cache| cache.cloud_force_region_ip.clone())
            .unwrap_or_default()
    };
    let mut session = session_from_bundle(bundle, &seed, cloud_route)?;
    if same_account {
        let mut renewed_cache =
            AuthTokenCache::from_session(&session)?.expect("bundle cache is present");
        renewed_cache.core_refresh_scopes = cache
            .as_ref()
            .map(|cache| {
                cache
                    .core_refresh_scopes
                    .iter()
                    .filter(|pending| pending.as_str() != scope.as_str())
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        session.token_cache_json = Some(serialize(&renewed_cache)?);
    }
    // 完整目标续期也先落盘，随后访问解析失败时仍保存 Web/SISU 与诊断结果。
    host.checkpoint(XboxAuthCheckpoint {
        previous_refresh_token: current.refresh_token,
        auth_session: session.clone(),
        core_only: false,
    })
    .await?;
    Ok(session)
}

#[cfg(test)]
#[path = "auth_session.test.rs"]
mod tests;
