use super::*;
use crate::error::AppResult;
use crate::mods::auth::{
    AuthProvider, AuthSessionReadyEvent, AuthState, CheckAuthResponse, LoginResponse,
};
use async_trait::async_trait;
use serde_json::json;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};

fn snapshot(token: &str, account: &str) -> Value {
    json!({"data":{"Token":token, "NotAfter":"2030-01-01T00:00:00Z", "DisplayClaims":{"xui":[{"uhs":account,"xid":account}]}}})
}

struct TestAuth {
    token: Mutex<Option<Value>>,
    check_error: Mutex<Option<WebApiError>>,
    checks: AtomicUsize,
    recoveries: AtomicUsize,
    stream_tokens: Mutex<Value>,
    stream_checks: Mutex<Vec<String>>,
    stream_recoveries: Mutex<Vec<(String, String)>>,
}
impl TestAuth {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            token: Mutex::new(Some(snapshot("old", "1"))),
            check_error: Mutex::new(None),
            checks: AtomicUsize::new(0),
            recoveries: AtomicUsize::new(0),
            stream_tokens: Mutex::new(json!({})),
            stream_checks: Mutex::new(Vec::new()),
            stream_recoveries: Mutex::new(Vec::new()),
        })
    }
}

#[async_trait]
impl AuthProvider for TestAuth {
    fn get_state(&self) -> AuthState {
        AuthState {
            provider: "xal".into(),
            is_authenticating: false,
            is_authenticated: self.token.lock().unwrap().is_some(),
            app_level: 0,
        }
    }
    fn get_active_session(&self) -> AppResult<Option<AuthSessionReadyEvent>> {
        let token = self.token.lock().unwrap().clone();
        let streams = self.stream_tokens.lock().unwrap().clone();
        Ok(token
            .filter(|_| streams.as_object().is_some_and(|tokens| !tokens.is_empty()))
            .map(|web_token| AuthSessionReadyEvent {
                provider: "xal".into(),
                app_level: 1,
                web_token,
                streaming_tokens: streams,
            }))
    }
    fn get_web_api_tokens(&self) -> AppResult<Option<(String, String)>> {
        Ok(self
            .token
            .lock()
            .unwrap()
            .as_ref()
            .and_then(resolve_web_token_claims)
            .map(|claims| (claims.uhs, claims.user_token)))
    }
    fn get_web_api_token_snapshot(&self) -> AppResult<Option<Value>> {
        Ok(self.token.lock().unwrap().clone())
    }
    async fn check_authentication(&self) -> AppResult<CheckAuthResponse> {
        self.checks.fetch_add(1, Ordering::SeqCst);
        if let Some(error) = self.check_error.lock().unwrap().clone() {
            return Err(crate::error::AppError::WebApi(error));
        }
        *self.token.lock().unwrap() = Some(snapshot("checked", "1"));
        Ok(CheckAuthResponse {
            provider: "xal".into(),
            started_silent_flow: true,
        })
    }
    async fn recover_web_api_session(&self, _: &str) -> AppResult<()> {
        self.recoveries.fetch_add(1, Ordering::SeqCst);
        if let Some(error) = self.check_error.lock().unwrap().clone() {
            return Err(crate::error::AppError::WebApi(error));
        }
        *self.token.lock().unwrap() = Some(snapshot("recovered", "1"));
        Ok(())
    }
    async fn get_streaming_token(&self, target: &str) -> AppResult<Option<Value>> {
        self.stream_checks.lock().unwrap().push(target.to_string());
        if let Some(error) = self.check_error.lock().unwrap().clone() {
            return Err(crate::error::AppError::WebApi(error));
        }
        let token = json!({"data":{"gsToken":format!("{target}-checked")}});
        self.stream_tokens.lock().unwrap()
            [AuthTokenScope::from_target(target).stream_key().unwrap()] = token.clone();
        Ok(Some(token))
    }
    async fn recover_streaming_token(
        &self,
        target: &str,
        rejected: &str,
        _credential_generation: Option<u64>,
    ) -> AppResult<Option<Value>> {
        self.stream_recoveries
            .lock()
            .unwrap()
            .push((target.to_string(), rejected.to_string()));
        if let Some(error) = self.check_error.lock().unwrap().clone() {
            return Err(crate::error::AppError::WebApi(error));
        }
        let token = json!({"data":{"gsToken":format!("{target}-recovered")}});
        self.stream_tokens.lock().unwrap()
            [AuthTokenScope::from_target(target).stream_key().unwrap()] = token.clone();
        Ok(Some(token))
    }
    async fn login(&self) -> AppResult<LoginResponse> {
        unimplemented!()
    }
    async fn get_transfer_token(&self) -> AppResult<String> {
        unimplemented!()
    }
    async fn clear_auth_cache(&self, _: &str) -> AppResult<()> {
        unimplemented!()
    }
    async fn logout(&self) -> AppResult<()> {
        unimplemented!()
    }
    async fn handle_oauth_callback(&self, _: &str) -> AppResult<()> {
        unimplemented!()
    }
    async fn cancel_pending_login(&self) {
        unimplemented!()
    }
    async fn reset_runtime_after_store_purge(&self) {
        unimplemented!()
    }
    fn mark_callback_processing(&self) -> AppResult<()> {
        unimplemented!()
    }
    fn unmark_callback_processing(&self) -> AppResult<()> {
        unimplemented!()
    }
}

#[tokio::test]
async fn web_only_context_preserves_account_claims_and_checks_missing_credentials() {
    let auth = TestAuth::new();
    let resolver = DataSessionResolver::new(auth.clone());
    let session = resolver
        .ensure_authenticated_session()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        resolve_web_token_claims(&session.web_token)
            .unwrap()
            .xid
            .as_deref(),
        Some("1")
    );
    assert_eq!(
        session.web_token["data"]["NotAfter"],
        "2030-01-01T00:00:00Z"
    );
    assert_eq!(auth.checks.load(Ordering::SeqCst), 0);
    *auth.token.lock().unwrap() = None;
    assert!(resolver
        .ensure_authenticated_session()
        .await
        .unwrap()
        .is_some());
    assert_eq!(auth.checks.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn unauthorized_query_recovers_once_and_retries_with_the_new_token() {
    let auth = TestAuth::new();
    let resolver = DataSessionResolver::new(auth.clone());
    let attempts = AtomicUsize::new(0);
    let result = resolver
        .run_query(|session| {
            attempts.fetch_add(1, Ordering::SeqCst);
            async move {
                if resolve_web_token_claims(&session.web_token)
                    .unwrap()
                    .user_token
                    == "old"
                {
                    Err(WebApiError::http(401, "private"))
                } else {
                    Ok(42)
                }
            }
        })
        .await
        .unwrap();
    assert_eq!(result, 42);
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
    assert_eq!(auth.recoveries.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_second_unauthorized_response_stops_after_one_recovery() {
    let auth = TestAuth::new();
    let resolver = DataSessionResolver::new(auth.clone());
    let attempts = AtomicUsize::new(0);
    let result: Result<(), _> = resolver
        .run_query(|_| {
            attempts.fetch_add(1, Ordering::SeqCst);
            async { Err(WebApiError::http(401, "private")) }
        })
        .await;
    assert_eq!(result.unwrap_err().to_status_code(), Some(401));
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
    assert_eq!(auth.recoveries.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn permission_rate_limit_and_network_errors_keep_their_category_without_auth_recovery() {
    for error in [
        WebApiError::http(403, "private"),
        WebApiError::http(429, "private"),
        WebApiError::network_kind(xbox_webapi::NetworkErrorKind::Dns),
    ] {
        let auth = TestAuth::new();
        let resolver = DataSessionResolver::new(auth.clone());
        let result: Result<(), _> = resolver
            .run_query(|_| {
                let error = error.clone();
                async move { Err(error) }
            })
            .await;
        assert_eq!(result.unwrap_err().safe_summary(), error.safe_summary());
        assert_eq!(auth.recoveries.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn late_result_is_discarded_after_account_switch_or_logout() {
    for next in [Some(snapshot("other", "2")), None] {
        let auth = TestAuth::new();
        let resolver = DataSessionResolver::new(auth.clone());
        let result = resolver
            .run_query(|_| {
                *auth.token.lock().unwrap() = next.clone();
                async { Ok("old-account-result") }
            })
            .await;
        assert!(matches!(result, Err(WebApiError::Parse { .. })));
    }
}

#[tokio::test]
async fn failed_credential_recovery_preserves_the_original_network_category() {
    let auth = TestAuth::new();
    *auth.token.lock().unwrap() = None;
    *auth.check_error.lock().unwrap() = Some(WebApiError::network_kind(
        xbox_webapi::NetworkErrorKind::Dns,
    ));
    let resolver = DataSessionResolver::new(auth);
    let error = resolver.ensure_authenticated_session().await.unwrap_err();
    assert!(error.contains("DNS"));
    let result: Result<(), _> = resolver.run_query(|_| async { Ok(()) }).await;
    assert_eq!(
        result.unwrap_err().network_error_kind(),
        Some(xbox_webapi::NetworkErrorKind::Dns)
    );
}

#[tokio::test]
async fn streaming_queries_ensure_and_recover_the_target_even_with_valid_web_credentials() {
    for scope in [AuthTokenScope::Home, AuthTokenScope::Cloud] {
        let auth = TestAuth::new();
        let other_key = if scope == AuthTokenScope::Home {
            "xCloudToken"
        } else {
            "xHomeToken"
        };
        auth.stream_tokens.lock().unwrap()[other_key] = json!({"gsToken": "other-target"});
        let resolver = DataSessionResolver::new(auth.clone());
        let attempts = AtomicUsize::new(0);
        let key = scope.stream_key().unwrap();
        let result = resolver
            .run_scoped_query(scope, |session| {
                attempts.fetch_add(1, Ordering::SeqCst);
                async move {
                    assert_eq!(
                        session.streaming_tokens[other_key]["gsToken"],
                        "other-target"
                    );
                    let token = session.streaming_tokens[key]["data"]["gsToken"]
                        .as_str()
                        .unwrap()
                        .to_string();
                    if token.ends_with("checked") {
                        Err(WebApiError::http(401, "rejected"))
                    } else {
                        Ok(token)
                    }
                }
            })
            .await
            .unwrap();
        assert_eq!(result, format!("{}-recovered", scope.as_str()));
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
        assert_eq!(&*auth.stream_checks.lock().unwrap(), &[scope.as_str()]);
        assert_eq!(
            &*auth.stream_recoveries.lock().unwrap(),
            &[(
                scope.as_str().to_string(),
                format!("{}-checked", scope.as_str())
            )]
        );
        assert_eq!(auth.recoveries.load(Ordering::SeqCst), 0);
        assert_eq!(auth.checks.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn cloud_query_stops_after_one_recovery_and_keeps_temporary_errors_typed() {
    let auth = TestAuth::new();
    let resolver = DataSessionResolver::new(auth.clone());
    let result: Result<(), _> = resolver
        .run_scoped_query(AuthTokenScope::Cloud, |_| async {
            Err(WebApiError::http(401, "rejected"))
        })
        .await;
    assert_eq!(result.unwrap_err().to_status_code(), Some(401));
    assert_eq!(auth.stream_recoveries.lock().unwrap().len(), 1);
    let error = WebApiError::network_kind(xbox_webapi::NetworkErrorKind::Timeout);
    *auth.check_error.lock().unwrap() = Some(error.clone());
    let result: Result<(), _> = resolver
        .run_scoped_query(AuthTokenScope::Cloud, |_| async { Ok(()) })
        .await;
    assert_eq!(result.unwrap_err().safe_summary(), error.safe_summary());
}
