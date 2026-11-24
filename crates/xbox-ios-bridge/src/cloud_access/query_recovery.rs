use super::*;
use crate::query_failure::credential_error;
use std::future::Future;
use xbox_webapi::{
    BearerRecoveryFuture, BearerTokenRecovery, HttpTransport, WebApiError, XcloudApi,
};

#[derive(Debug, Default)]
pub(super) struct QueryRecoveryState {
    last_failure: Option<(std::time::Instant, WebApiError)>,
}

struct QueryCredentialRecovery {
    handle: String,
    bound: StreamingAccessContext,
    state: Arc<tokio::sync::Mutex<QueryRecoveryState>>,
}

impl std::fmt::Debug for QueryCredentialRecovery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QueryCredentialRecovery")
            .finish_non_exhaustive()
    }
}

/// 普通读取保留公共查询预算，只把明确目标 401 交回原有凭据 owner。
pub(crate) fn query_xcloud_api(
    handle: &str,
    access: &StreamingAccessContext,
    region_host: Option<&str>,
) -> Result<XcloudApi, XboxBridgeError> {
    let recovery = QueryCredentialRecovery::new(handle, access)?;
    Ok(XcloudApi::with_transport(
        HttpTransport::for_queries().with_bearer_recovery(Arc::new(recovery)),
        region_host.unwrap_or(&access.host).to_owned(),
        access.bearer_token.clone(),
    ))
}

impl QueryCredentialRecovery {
    fn new(handle: &str, bound: &StreamingAccessContext) -> Result<Self, XboxBridgeError> {
        let registry = stream_access_registry()?;
        registry.validate_bound(handle.trim(), bound)?;
        if bound.target != Target::Cloud {
            return Err(XboxBridgeError::InvalidData(
                "streamAccessTargetMismatch".into(),
            ));
        }
        let state = Arc::clone(&registry.leases[handle.trim()].query_recovery);
        Ok(Self {
            handle: handle.trim().into(),
            bound: bound.clone(),
            state,
        })
    }

    fn current_access(&self) -> Result<StreamingAccessContext, XboxBridgeError> {
        let registry = stream_access_registry()?;
        registry
            .validate_bound(&self.handle, &self.bound)
            .map_err(|_| XboxBridgeError::Cancelled)?;
        // 已经发出的读取可以在短租约到期后完成，退出/替换仍由 bound 校验阻止。
        Ok(registry.leases[&self.handle]
            .context
            .clone()
            .unwrap_or_else(|| self.bound.clone()))
    }

    async fn recover_with<F, Fut>(&self, rejected: String, renew: F) -> Result<String, WebApiError>
    where
        F: FnOnce(StreamingAccessContext, String) -> Fut,
        Fut: Future<Output = Result<(StreamingAccessContext, AuthSession), XboxBridgeError>>,
    {
        let mut state = self.state.lock().await;
        let previous = self.current_access().map_err(credential_error)?;
        if previous.bearer_token != rejected {
            return Ok(previous.bearer_token);
        }
        if let Some((recorded, error)) = &state.last_failure {
            if recorded.elapsed() < std::time::Duration::from_secs(1) {
                return Err(error.clone());
            }
        }
        let (renewed, _) = match renew(previous.clone(), rejected).await {
            Ok(result) => result,
            Err(error) => {
                let error = credential_error(error);
                state.last_failure = Some((std::time::Instant::now(), error.clone()));
                return Err(error);
            }
        };
        let mut registry = stream_access_registry().map_err(credential_error)?;
        registry
            .validate_bound(&self.handle, &previous)
            .map_err(|_| credential_error(XboxBridgeError::Cancelled))?;
        if renewed.account_id != previous.account_id
            || renewed.target != previous.target
            || renewed.owner_generation != previous.owner_generation
        {
            return Err(credential_error(XboxBridgeError::Cancelled));
        }
        let token = renewed.bearer_token.clone();
        if token.trim().is_empty() {
            return Err(WebApiError::parse(
                "credential renewal returned an empty token",
            ));
        }
        let lease = registry
            .leases
            .get_mut(&self.handle)
            .expect("validated lease");
        lease.metadata.created_at_ms = renewed.created_at_ms;
        lease.metadata.expires_at_ms = renewed.expires_at_ms;
        lease.metadata.terminal = None;
        lease.context = Some(renewed);
        state.last_failure = None;
        Ok(token)
    }
}

impl BearerTokenRecovery for QueryCredentialRecovery {
    fn recover(&self, rejected_token: &str) -> BearerRecoveryFuture {
        let recovery = Self {
            handle: self.handle.clone(),
            bound: self.bound.clone(),
            state: Arc::clone(&self.state),
        };
        let rejected = rejected_token.to_owned();
        Box::pin(async move {
            recovery
                .recover_with(rejected, |access, rejected| async move {
                    renew_bound_access(&access, DownstreamTokenScope::Cloud, Some(rejected), true)
                        .await
                })
                .await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    fn fixture() -> (String, StreamingAccessContext, QueryCredentialRecovery) {
        let context =
            super::super::tests::fixture_context(&uuid::Uuid::new_v4().to_string(), Target::Cloud);
        let (handle, access) = register_test_stream_access(context);
        let recovery = QueryCredentialRecovery::new(&handle, &access).unwrap();
        (handle, access, recovery)
    }

    fn renewed(mut access: StreamingAccessContext) -> (StreamingAccessContext, AuthSession) {
        access.bearer_token = "new-token".into();
        let auth = AuthSession {
            refresh_token: "fixture".into(),
            seed_json: "{}".into(),
            web_token_json: "{}".into(),
            app_level: 0,
            token_cache_json: None,
        };
        (access, auth)
    }

    #[tokio::test]
    async fn concurrent_rejections_share_renewal_and_publish_latest_bound_access() {
        let (handle, access, recovery) = fixture();
        let calls = AtomicUsize::new(0);
        let calls = &calls;
        let renew = |current, rejected: String| async move {
            assert_eq!(rejected, "gs-token");
            calls.fetch_add(1, Ordering::SeqCst);
            tokio::task::yield_now().await;
            Ok(renewed(current))
        };
        let (first, second) = tokio::join!(
            recovery.recover_with(access.bearer_token.clone(), renew),
            recovery.recover_with(access.bearer_token, renew),
        );
        assert_eq!(first.unwrap(), "new-token");
        assert_eq!(second.unwrap(), "new-token");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            load_scoped_stream_access(&handle, Target::Cloud, None, None)
                .unwrap()
                .bearer_token,
            "new-token"
        );
        release_stream_access(handle).unwrap();
    }

    #[tokio::test]
    async fn temporary_failure_keeps_access_and_does_not_repeat_recovery() {
        let (handle, access, recovery) = fixture();
        let first = recovery
            .recover_with(access.bearer_token.clone(), |_, _| async {
                Err(XboxBridgeError::RetryableAuthentication("fixture".into()))
            })
            .await
            .unwrap_err();
        assert_eq!(first.to_status_code(), Some(503));
        let second = recovery
            .recover_with(access.bearer_token.clone(), |_, _| async {
                panic!("temporary failure cooldown must merge concurrent rejection")
            })
            .await
            .unwrap_err();
        assert_eq!(second.to_status_code(), Some(503));
        assert_eq!(
            recovery.current_access().unwrap().bearer_token,
            access.bearer_token
        );
        release_stream_access(handle).unwrap();
    }

    #[tokio::test]
    async fn revoked_and_changed_owners_reject_late_renewal() {
        let (handle, access, recovery) = fixture();
        let result = recovery
            .recover_with(access.bearer_token, |current, _| async {
                release_stream_access(handle.clone()).unwrap();
                Ok(renewed(current))
            })
            .await
            .unwrap_err();
        assert!(matches!(
            crate::query_failure::query_error(result),
            XboxBridgeError::Cancelled
        ));
        let (handle, access, recovery) = fixture();
        let result = recovery
            .recover_with(access.bearer_token, |mut current, _| async {
                current.account_id = "other-owner".into();
                Ok(renewed(current))
            })
            .await
            .unwrap_err();
        assert!(matches!(
            crate::query_failure::query_error(result),
            XboxBridgeError::Cancelled
        ));
        assert_eq!(recovery.current_access().unwrap().bearer_token, "gs-token");
        release_stream_access(handle).unwrap();
    }

    #[tokio::test]
    async fn cancelled_renewal_does_not_commit_and_releases_merge_lock() {
        let (handle, access, recovery) = fixture();
        let result = tokio::time::timeout(
            std::time::Duration::from_millis(1),
            recovery.recover_with(access.bearer_token.clone(), |_, _| std::future::pending()),
        )
        .await;
        assert!(result.is_err());
        assert_eq!(recovery.current_access().unwrap().bearer_token, "gs-token");
        assert_eq!(
            recovery
                .recover_with(access.bearer_token, |current, _| async {
                    Ok(renewed(current))
                })
                .await
                .unwrap(),
            "new-token"
        );
        release_stream_access(handle).unwrap();
    }
}
