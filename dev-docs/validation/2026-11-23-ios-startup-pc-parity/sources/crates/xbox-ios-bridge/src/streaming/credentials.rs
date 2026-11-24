//! 会话绑定凭据、单次续期与安全存储提交的宿主投影。
use crate::cloud_access::{
    renew_bound_stream_access, renew_bound_web_access, validate_bound_stream_access,
    StreamingAccessContext,
};
use crate::{AuthSession, XboxBridgeError};
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;
use xbox_webapi::transport::{BearerRecoveryFuture, BearerTokenRecovery, HttpTransport};
use xbox_webapi::WebApiError;

struct StreamingCredentialRecovery {
    credentials: Arc<SessionCredentials>,
    target: String,
    owner_generation: u64,
}

impl std::fmt::Debug for StreamingCredentialRecovery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamingCredentialRecovery")
            .field("target", &self.target)
            .finish_non_exhaustive()
    }
}

impl BearerTokenRecovery for StreamingCredentialRecovery {
    fn recover(&self, rejected_token: &str) -> BearerRecoveryFuture {
        let credentials = Arc::clone(&self.credentials);
        let target = self.target.clone();
        let generation = self.owner_generation;
        let rejected = rejected_token.to_owned();
        Box::pin(async move {
            credentials
                .access(&target, Some(&rejected), Some(generation))
                .await
                .map(|access| access.bearer_token)
        })
    }
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct XboxStreamingAuthUpdate {
    pub revision: u64,
    pub account_id: String,
    pub owner_generation: u64,
    pub previous_refresh_token: String,
    pub auth_session: AuthSession,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cloud_access::register_test_stream_access;
    use std::sync::atomic::AtomicUsize;
    use xbox_streaming::{policy::session::SessionAccessContext, Target};

    fn fixture() -> SessionCredentials {
        let (handle, access) = register_test_stream_access(StreamingAccessContext {
            host: "cloud.example.com".into(),
            bearer_token: "old-token".into(),
            account_id: uuid::Uuid::new_v4().to_string(),
            refresh_token: "old-refresh".into(),
            auth_seed: None,
            auth_session: None,
            auth_provider: None,
            target: Target::Cloud,
            session_access: SessionAccessContext::default(),
            force_region_ip: String::new(),
            web_uhs: "uhs".into(),
            web_token: "web".into(),
            owner_generation: 0,
            created_at_ms: 0,
            expires_at_ms: u64::MAX,
            fallback_turn: None,
        });
        SessionCredentials::new(handle, access)
    }

    fn renewed(
        mut access: StreamingAccessContext,
        token: &str,
    ) -> (StreamingAccessContext, AuthSession) {
        access.bearer_token = token.into();
        access.refresh_token = format!("{token}-refresh");
        access.expires_at_ms = u64::MAX;
        let auth = AuthSession {
            refresh_token: access.refresh_token.clone(),
            seed_json: "seed".into(),
            web_token_json: "web".into(),
            app_level: 2,
            token_cache_json: None,
        };
        (access, auth)
    }

    #[tokio::test]
    async fn concurrent_rejections_share_one_renewal_and_reuse_latest_bearer() {
        let credentials = fixture();
        let calls = AtomicUsize::new(0);
        let renew = |access, _| async {
            calls.fetch_add(1, Ordering::Relaxed);
            tokio::task::yield_now().await;
            Ok(renewed(access, "new-token"))
        };
        let (first, second) = tokio::join!(
            credentials.access_with("cloud", Some("old-token"), None, renew),
            credentials.access_with("cloud", Some("old-token"), None, renew)
        );
        assert_eq!(first.unwrap().bearer_token, "new-token");
        assert_eq!(second.unwrap().bearer_token, "new-token");
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        let update = credentials.pending_update().await.unwrap();
        assert_eq!(update.revision, 1);
        assert_eq!(update.previous_refresh_token, "old-refresh");
    }

    #[tokio::test]
    async fn retryable_failure_keeps_credentials_and_shares_pc_backoff() {
        let credentials = fixture();
        let error = credentials
            .access_with("cloud", Some("old-token"), None, |_, _| async {
                Err(XboxBridgeError::RetryableAuthentication(
                    "private error".into(),
                ))
            })
            .await
            .unwrap_err();
        assert_eq!(error.to_status_code(), Some(503));
        assert!(!error.to_string().contains("private error"));
        assert!(credentials.pending_update().await.is_none());
        let cached_error = credentials
            .access_with("cloud", Some("old-token"), None, |_, _| async {
                panic!("PC backoff must reuse the previous failure")
            })
            .await
            .unwrap_err();
        assert_eq!(cached_error.safe_summary(), error.safe_summary());
        assert_eq!(
            credentials.state.lock().await.access.bearer_token,
            "old-token"
        );
    }

    #[tokio::test]
    async fn revocation_rejects_late_renewal_before_commit() {
        let credentials = fixture();
        let error = credentials
            .access_with("cloud", Some("old-token"), None, |access, _| async {
                credentials.revoke();
                Ok(renewed(access, "late-token"))
            })
            .await
            .unwrap_err();
        assert_eq!(error.to_status_code(), Some(401));
        assert!(credentials.pending_update().await.is_none());
    }

    #[tokio::test]
    async fn scope_and_account_changes_are_rejected_before_commit() {
        let credentials = fixture();
        for (target, generation) in [("home", None), ("cloud", Some(u64::MAX))] {
            assert!(credentials
                .access_with(target, Some("old-token"), generation, |_, _| async {
                    panic!("scope must be checked before renewal")
                })
                .await
                .is_err());
        }
        assert!(credentials
            .access_with("cloud", Some("old-token"), None, |access, _| async {
                let (mut access, auth) = renewed(access, "new-token");
                access.account_id = "other-account".into();
                Ok((access, auth))
            })
            .await
            .is_err());
        assert!(credentials.pending_update().await.is_none());
    }

    #[tokio::test]
    async fn pending_update_preserves_persistence_base_until_exact_revision_ack() {
        let credentials = fixture();
        credentials
            .access_with("cloud", Some("old-token"), None, |access, _| async {
                Ok(renewed(access, "first"))
            })
            .await
            .unwrap();
        credentials
            .access_with("cloud", Some("first"), None, |access, _| async {
                Ok(renewed(access, "second"))
            })
            .await
            .unwrap();
        credentials.acknowledge(1).await;
        let update = credentials.pending_update().await.unwrap();
        assert_eq!(update.revision, 2);
        assert_eq!(update.previous_refresh_token, "old-refresh");
        assert_eq!(update.auth_session.refresh_token, "second-refresh");
        credentials.acknowledge(2).await;
        assert!(credentials.pending_update().await.is_none());
    }

    #[tokio::test]
    async fn acknowledgement_rebases_a_newer_update_created_during_host_save() {
        let credentials = fixture();
        credentials
            .access_with("cloud", Some("old-token"), None, |access, _| async {
                Ok(renewed(access, "first"))
            })
            .await
            .unwrap();
        assert_eq!(credentials.pending_update().await.unwrap().revision, 1);
        credentials
            .access_with("cloud", Some("first"), None, |access, _| async {
                Ok(renewed(access, "second"))
            })
            .await
            .unwrap();
        credentials.acknowledge(1).await;
        let update = credentials.pending_update().await.unwrap();
        assert_eq!(update.revision, 2);
        assert_eq!(update.previous_refresh_token, "first-refresh");
        assert_eq!(update.auth_session.refresh_token, "second-refresh");
    }

    #[tokio::test]
    async fn expiring_credentials_renew_proactively_and_host_sync_preserves_owner() {
        let credentials = fixture();
        credentials.state.lock().await.access.expires_at_ms = 0;
        credentials
            .access_with("cloud", None, None, |access, _| async {
                Ok(renewed(access, "first"))
            })
            .await
            .unwrap();
        let update = credentials.pending_update().await.unwrap();
        let auth = AuthSession {
            refresh_token: "host-refresh".into(), seed_json: r#"{"private_jwk":{}}"#.into(),
            web_token_json: serde_json::json!({"data":{"Token":"host-web","DisplayClaims":{"xui":[{"xid":update.account_id,"uhs":"uhs"}]}}}).to_string(), app_level: 2, token_cache_json: None,
        };
        credentials
            .synchronize(update.revision, auth)
            .await
            .unwrap();
        let current = credentials
            .access_with("cloud", None, None, |_, _| async {
                panic!("valid token should reuse synchronized host credentials")
            })
            .await
            .unwrap();
        assert_eq!(current.refresh_token, "host-refresh");
        assert_eq!(current.web_token, "host-web");
        assert_eq!(current.bearer_token, "first");
        assert!(credentials.pending_update().await.is_none());
    }

    #[tokio::test]
    async fn cleanup_retry_reuses_last_resource_token_and_releases_update_publication() {
        let credentials = fixture();
        credentials.begin_cleanup();
        credentials.revoke();
        let current = credentials
            .access_with("cloud", None, None, |_, _| async {
                panic!("closed cleanup should reuse the last resource token")
            })
            .await
            .unwrap();
        assert_eq!(current.bearer_token, "old-token");
        assert!(credentials.pending_update().await.is_none());
        assert!(credentials
            .access_with("home", None, None, |_, _| async {
                panic!("wrong cleanup target")
            })
            .await
            .is_err());
        assert!(credentials
            .access_with("cloud", Some("old-token"), None, |_, _| async {
                panic!("closed cleanup cannot renew")
            })
            .await
            .is_err());
    }
}

struct CredentialState {
    access: StreamingAccessContext,
    revision: u64,
    pending: Option<XboxStreamingAuthUpdate>,
    delivered: Option<(u64, String)>,
    rejected_token: Option<String>,
    last_failure: Option<(std::time::Instant, WebApiError)>,
}

pub(super) struct SessionCredentials {
    handle: String,
    active: AtomicBool,
    cleanup: AtomicBool,
    state: Mutex<CredentialState>,
}

impl SessionCredentials {
    pub(super) fn handle(&self) -> &str {
        &self.handle
    }
    pub(super) fn transport(self: &Arc<Self>, access: &StreamingAccessContext) -> HttpTransport {
        HttpTransport::new().with_bearer_recovery(Arc::new(StreamingCredentialRecovery {
            credentials: Arc::clone(self),
            target: access.target.as_str().to_owned(),
            owner_generation: access.owner_generation,
        }))
    }
    pub(super) fn new(handle: String, access: StreamingAccessContext) -> Self {
        Self {
            handle,
            active: AtomicBool::new(true),
            cleanup: AtomicBool::new(false),
            state: Mutex::new(CredentialState {
                access,
                revision: 0,
                pending: None,
                delivered: None,
                rejected_token: None,
                last_failure: None,
            }),
        }
    }

    pub(super) fn revoke(&self) {
        self.active.store(false, Ordering::Release);
    }

    pub(super) fn begin_cleanup(&self) {
        self.cleanup.store(true, Ordering::Release);
    }

    fn validate(&self, access: &StreamingAccessContext) -> Result<(), WebApiError> {
        if !self.active.load(Ordering::Acquire) {
            return Err(WebApiError::http(401, "streaming credentials revoked"));
        }
        validate_bound_stream_access(&self.handle, access)
            .map_err(|_| WebApiError::http(401, "streaming credential owner changed"))
    }

    pub(super) async fn access(
        &self,
        target: &str,
        rejected: Option<&str>,
        generation: Option<u64>,
    ) -> Result<StreamingAccessContext, WebApiError> {
        self.access_with(
            target,
            rejected,
            generation,
            |access, rejected| async move { renew_bound_stream_access(&access, rejected).await },
        )
        .await
    }

    async fn access_with<F, Fut>(
        &self,
        target: &str,
        rejected: Option<&str>,
        generation: Option<u64>,
        renew: F,
    ) -> Result<StreamingAccessContext, WebApiError>
    where
        F: FnOnce(StreamingAccessContext, Option<String>) -> Fut,
        Fut: Future<Output = Result<(StreamingAccessContext, AuthSession), XboxBridgeError>>,
    {
        // 同一会话的401和主动续期共享锁，后到请求直接消费已更新凭据。
        let mut state = self.state.lock().await;
        // 公共 flow 已撤销业务请求；后台 stop 重试仅复用该资源的最后凭据。
        if self.cleanup.load(Ordering::Acquire) && !self.active.load(Ordering::Acquire) {
            if target != state.access.target.as_str() {
                return Err(WebApiError::http(401, "cleanup credential target changed"));
            }
            if rejected.is_some() {
                return Err(WebApiError::http(
                    401,
                    "cleanup credentials require a live owner",
                ));
            }
            return Ok(state.access.clone());
        }
        self.validate(&state.access)?;
        if target != state.access.target.as_str()
            || generation.is_some_and(|value| value != state.access.owner_generation)
        {
            return Err(WebApiError::http(401, "streaming credential scope changed"));
        }
        if rejected.is_some_and(|token| token == state.access.bearer_token) {
            state.rejected_token = Some(state.access.bearer_token.clone());
        }
        let expired = state.access.expires_at_ms <= super::now_ms_f64() as u64 + 60_000;
        let needs_renewal = expired
            || state.rejected_token.as_deref() == Some(state.access.bearer_token.as_str())
            || match rejected {
                Some(token) => token == state.access.bearer_token,
                None => expired,
            };
        if needs_renewal {
            let previous = state.access.clone();
            if let Some((recorded, error)) = &state.last_failure {
                if recorded.elapsed() < std::time::Duration::from_secs(1) {
                    return Err(error.clone());
                }
            }
            let result = renew(previous.clone(), state.rejected_token.clone()).await;
            let (renewed, auth_session) = match result {
                Ok(result) => result,
                Err(error) => {
                    let error = credential_error(error);
                    state.last_failure = Some((std::time::Instant::now(), error.clone()));
                    return Err(error);
                }
            };
            self.commit_renewal(&mut state, previous, renewed, auth_session)?;
            state.rejected_token = None;
            state.last_failure = None;
        }
        Ok(state.access.clone())
    }

    fn commit_renewal(
        &self,
        state: &mut CredentialState,
        previous: StreamingAccessContext,
        renewed: StreamingAccessContext,
        auth_session: AuthSession,
    ) -> Result<(), WebApiError> {
        // 退出、替换访问句柄和迟到网络响应在提交前重新核验。
        self.validate(&previous)?;
        if renewed.account_id != previous.account_id
            || renewed.target != previous.target
            || renewed.owner_generation != previous.owner_generation
        {
            return Err(WebApiError::http(
                401,
                "streaming credential renewal owner changed",
            ));
        }
        state.revision = state.revision.saturating_add(1);
        let previous_refresh_token = state
            .pending
            .as_ref()
            .map(|pending| pending.previous_refresh_token.clone())
            .unwrap_or(previous.refresh_token);
        state.pending = Some(XboxStreamingAuthUpdate {
            revision: state.revision,
            account_id: renewed.account_id.clone(),
            owner_generation: renewed.owner_generation,
            previous_refresh_token,
            auth_session,
        });
        state.access = renewed;
        Ok(())
    }

    async fn recover_web_access(
        &self,
        rejected: &str,
    ) -> Result<StreamingAccessContext, WebApiError> {
        let mut state = self.state.lock().await;
        self.validate(&state.access)?;
        if state.access.web_token != rejected {
            return Ok(state.access.clone());
        }
        let previous = state.access.clone();
        let (renewed, auth) = Box::pin(renew_bound_web_access(&previous, rejected.into()))
            .await
            .map_err(credential_error)?;
        self.commit_renewal(&mut state, previous, renewed, auth)?;
        Ok(state.access.clone())
    }

    /// Smartglass/唤醒与 PC 的 Web 查询一致，401 回到同一 owner 恢复一次。
    pub(super) async fn run_web_query<T, F, Fut>(&self, request: F) -> Result<T, WebApiError>
    where
        F: Fn(StreamingAccessContext) -> Fut,
        Fut: Future<Output = Result<T, WebApiError>>,
    {
        let target = self.state.lock().await.access.target;
        let mut access = self.access(target.as_str(), None, None).await?;
        let mut result = request(access.clone()).await;
        if result
            .as_ref()
            .is_err_and(|error| error.to_status_code() == Some(401))
        {
            access = Box::pin(self.recover_web_access(&access.web_token)).await?;
            result = request(access.clone()).await;
        }
        self.validate(&access)?;
        result
    }

    pub(super) async fn pending_update(&self) -> Option<XboxStreamingAuthUpdate> {
        let mut state = self.state.lock().await;
        self.validate(&state.access).ok()?;
        state.delivered = state
            .pending
            .as_ref()
            .map(|update| (update.revision, update.auth_session.refresh_token.clone()));
        state.pending.clone()
    }

    pub(super) async fn acknowledge(&self, revision: u64) {
        let mut state = self.state.lock().await;
        if state
            .pending
            .as_ref()
            .is_some_and(|update| update.revision == revision)
        {
            state.pending = None;
        } else if let Some((delivered_revision, token)) = state.delivered.clone() {
            if delivered_revision == revision {
                if let Some(pending) = &mut state.pending {
                    pending.previous_refresh_token = token;
                }
            }
        }
        state.delivered = None;
    }

    pub(super) async fn synchronize(
        &self,
        revision: u64,
        auth: AuthSession,
    ) -> Result<(), WebApiError> {
        let mut state = self.state.lock().await;
        self.validate(&state.access)?;
        if state.pending.is_none() {
            return Ok(());
        }
        let web: serde_json::Value = crate::deserialize(&auth.web_token_json)
            .map_err(|_| WebApiError::http(401, "host credentials invalid"))?;
        let claims = crate::resolve_web_token_claims(&web)
            .map_err(|_| WebApiError::http(401, "host credential owner unavailable"))?;
        let account = claims.xuid.as_deref().unwrap_or(&claims.uhs);
        if account != state.access.account_id || auth.refresh_token.trim().is_empty() {
            return Err(WebApiError::http(401, "host credential owner changed"));
        }
        let seed = crate::deserialize(&auth.seed_json)
            .map_err(|_| WebApiError::http(401, "host credential seed invalid"))?;
        if let Some(pending) = &mut state.pending {
            if pending.revision != revision {
                pending.previous_refresh_token = auth.refresh_token;
                state.delivered = None;
                return Ok(());
            }
        }
        state.access.auth_session = Some(auth.clone());
        state.access.refresh_token = auth.refresh_token;
        state.access.auth_seed = Some(seed);
        state.access.web_uhs = claims.uhs;
        state.access.web_token = claims.token;
        state.pending = None;
        state.delivered = None;
        Ok(())
    }
}

fn credential_error(error: XboxBridgeError) -> WebApiError {
    match error {
        XboxBridgeError::Query { failure } => match failure.kind {
            crate::XboxQueryFailureKind::Timeout => {
                WebApiError::network_kind(xbox_webapi::NetworkErrorKind::Timeout)
            }
            crate::XboxQueryFailureKind::Network => {
                WebApiError::network_kind(xbox_webapi::NetworkErrorKind::Connect)
            }
            _ if failure.status_code.is_some() => WebApiError::http(
                failure.status_code.unwrap(),
                "streaming credential renewal unavailable",
            ),
            crate::XboxQueryFailureKind::Authentication => {
                WebApiError::http(401, "streaming credential renewal rejected")
            }
            _ => WebApiError::parse("streaming credential response unavailable"),
        },
        XboxBridgeError::RetryableAuthentication(_) => {
            WebApiError::http(503, "streaming credential renewal temporarily unavailable")
        }
        XboxBridgeError::Authentication(_) => {
            WebApiError::http(401, "streaming credential renewal rejected")
        }
        XboxBridgeError::Cancelled => WebApiError::parse("operationCancelled:stage=authRenewal"),
        _ => WebApiError::parse("streaming credential response unavailable"),
    }
}
