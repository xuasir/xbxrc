use super::*;
use std::sync::Mutex;
use xbox_auth_flow::FlowTokenDetails;
use xbox_webapi::{NetworkErrorKind, WebApiError};

fn user(refresh: &str) -> OAuthTokenResponse {
    serde_json::from_value(json!({"token_type":"Bearer","expires_in":3600,"scope":"xbox","access_token":"oauth","refresh_token":refresh})).unwrap()
}
fn fixture_bundle(refresh: &str) -> AuthBundle {
    let now = chrono::Utc::now().timestamp_millis();
    let details = FlowTokenDetails {
        issue_instant: None,
        not_after: Some((chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339()),
        token: Some("sisu".into()),
        display_claims: json!({}),
    };
    AuthBundle {
        user_token: user(refresh),
        sisu_token: FlowSisuTokenData {
            device_token: "device".into(),
            title_token: details.clone(),
            user_token: details.clone(),
            authorization_token: details,
        },
        web_token: json!({"data":{"Token":"web","NotAfter":(chrono::Utc::now()+chrono::Duration::hours(1)).to_rfc3339(),"DisplayClaims":{"xui":[{"xid":"account","uhs":"uhs"}]}}}),
        stream_tokens: json!({"xHomeToken":{"_objectCreateTime":now,"data":{"gsToken":"home","durationInSeconds":3600}},"xCloudToken":{"_objectCreateTime":now,"data":{"gsToken":"cloud","durationInSeconds":3600}}}),
        app_level: 2,
        token_update_time: now as u64,
    }
}
fn fixture_session() -> AuthSession {
    session_from_bundle(
        fixture_bundle("old-refresh"),
        &AuthFlowSeed {
            private_jwk: json!({}),
        },
        String::new(),
    )
    .unwrap()
}
#[derive(Default)]
struct Observer {
    saved: Mutex<Vec<XboxAuthCheckpoint>>,
    stages: Mutex<Vec<XboxAuthStage>>,
    fail_checkpoint: bool,
}
#[async_trait]
impl XboxAuthObserver for Observer {
    async fn checkpoint(&self, update: XboxAuthCheckpoint) -> Result<(), XboxBridgeError> {
        if self.fail_checkpoint {
            return Err(XboxBridgeError::InvalidData("storage".into()));
        }
        self.saved.lock().unwrap().push(update);
        Ok(())
    }
    fn on_stage(&self, event: XboxAuthStage) {
        self.stages.lock().unwrap().push(event);
    }
}
#[derive(Default)]
struct Backend {
    requests: Mutex<Vec<String>>,
    fail_finalization: bool,
}
#[async_trait]
impl AuthRenewalBackend for Backend {
    async fn downstream(
        &self,
        input: xbox_auth_flow::BuildDownstreamTokensInput,
        scope: DownstreamTokenScope,
        _: &dyn AuthFlowObserver,
    ) -> Result<AuthBundle, xbox_auth_flow::AuthFlowError> {
        self.requests
            .lock()
            .unwrap()
            .push(format!("downstream:{}", scope.as_str()));
        let mut bundle = fixture_bundle(&input.user_token.refresh_token);
        bundle.stream_tokens = scope
            .stream_key()
            .map(|key| json!({key:bundle.stream_tokens[key].clone()}))
            .unwrap_or_else(|| json!({}));
        Ok(bundle)
    }
    async fn refresh_core(
        &self,
        _: &str,
        _: &dyn AuthFlowObserver,
    ) -> Result<OAuthTokenResponse, xbox_auth_flow::AuthFlowError> {
        self.requests.lock().unwrap().push("oauth".into());
        Ok(user("rotated-refresh"))
    }
    async fn finalize_core(
        &self,
        token: &OAuthTokenResponse,
        _: &AuthFlowSeed,
        _: &str,
        scope: DownstreamTokenScope,
        _: &dyn AuthFlowObserver,
    ) -> Result<AuthBundle, xbox_auth_flow::AuthFlowError> {
        self.requests
            .lock()
            .unwrap()
            .push(format!("finalize:{}", scope.as_str()));
        if self.fail_finalization {
            return Err(WebApiError::network_kind(NetworkErrorKind::Timeout).into());
        }
        let mut bundle = fixture_bundle(&token.refresh_token);
        bundle.stream_tokens = scope
            .stream_key()
            .map(|key| json!({key:bundle.stream_tokens[key].clone()}))
            .unwrap_or_else(|| json!({}));
        Ok(bundle)
    }
}

#[tokio::test]
async fn warm_web_home_and_cloud_credentials_issue_zero_auth_requests() {
    for scope in [
        DownstreamTokenScope::Web,
        DownstreamTokenScope::Home,
        DownstreamTokenScope::Cloud,
    ] {
        let backend = Backend::default();
        let observer = Observer::default();
        let session = fixture_session();
        let output = renew_auth_session_with(
            &backend,
            session.clone(),
            String::new(),
            scope,
            false,
            &observer,
        )
        .await
        .unwrap();
        assert_eq!(output, session);
        assert!(backend.requests.lock().unwrap().is_empty());
        assert!(observer.saved.lock().unwrap().is_empty());
        assert_eq!(
            observer.stages.lock().unwrap()[0].stage,
            "validCredentialsReused"
        );
    }
}

#[tokio::test]
async fn legacy_web_session_reuses_valid_web_credentials_before_target_migration() {
    let backend = Backend::default();
    let observer = Observer::default();
    let mut session = fixture_session();
    session.token_cache_json = None;
    renew_auth_session_with(
        &backend,
        session,
        String::new(),
        DownstreamTokenScope::Web,
        false,
        &observer,
    )
    .await
    .unwrap();
    assert!(backend.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn legacy_streaming_session_migrates_with_only_the_selected_target() {
    let backend = Backend::default();
    let observer = Observer::default();
    let mut session = fixture_session();
    session.token_cache_json = None;
    let output = renew_auth_session_with(
        &backend,
        session,
        String::new(),
        DownstreamTokenScope::Home,
        false,
        &observer,
    )
    .await
    .unwrap();
    assert_eq!(
        *backend.requests.lock().unwrap(),
        ["oauth", "finalize:home"]
    );
    let saved = observer.saved.lock().unwrap();
    assert_eq!(saved.len(), 2);
    assert!(saved[0].core_only);
    assert!(!saved[1].core_only);
    let cache = AuthTokenCache::from_session(&output).unwrap().unwrap();
    assert!(cache.stream_tokens.get("xHomeToken").is_some());
    assert!(cache.stream_tokens.get("xCloudToken").is_none());
}

#[tokio::test]
async fn expired_target_reuses_sisu_and_preserves_other_target() {
    let backend = Backend::default();
    let observer = Observer::default();
    let mut session = fixture_session();
    let mut cache = AuthTokenCache::from_session(&session).unwrap().unwrap();
    let cloud = cache.stream_tokens["xCloudToken"].clone();
    cache.stream_tokens["xHomeToken"]["_objectCreateTime"] = json!(1);
    session.token_cache_json = Some(serialize(&cache).unwrap());
    let output = renew_auth_session_with(
        &backend,
        session,
        String::new(),
        DownstreamTokenScope::Home,
        false,
        &observer,
    )
    .await
    .unwrap();
    assert_eq!(*backend.requests.lock().unwrap(), ["downstream:home"]);
    assert_eq!(
        AuthTokenCache::from_session(&output)
            .unwrap()
            .unwrap()
            .stream_tokens["xCloudToken"],
        cloud
    );
}

#[tokio::test]
async fn authorization_region_change_refreshes_cloud_and_retains_home() {
    let backend = Backend::default();
    let observer = Observer::default();
    let output = renew_auth_session_with(
        &backend,
        fixture_session(),
        "192.0.2.1".into(),
        DownstreamTokenScope::Cloud,
        false,
        &observer,
    )
    .await
    .unwrap();
    assert_eq!(*backend.requests.lock().unwrap(), ["downstream:cloud"]);
    let cache = AuthTokenCache::from_session(&output).unwrap().unwrap();
    assert_eq!(cache.cloud_force_region_ip, "192.0.2.1");
    assert!(cache.stream_tokens.get("xHomeToken").is_some());
}

#[tokio::test]
async fn core_checkpoint_retains_new_refresh_token_after_timeout() {
    let backend = Backend {
        fail_finalization: true,
        ..Default::default()
    };
    let observer = Observer::default();
    let error = renew_auth_session_with(
        &backend,
        fixture_session(),
        String::new(),
        DownstreamTokenScope::Cloud,
        true,
        &observer,
    )
    .await
    .unwrap_err();
    assert!(
        matches!(error,XboxBridgeError::Query { ref failure } if failure.kind==crate::XboxQueryFailureKind::Timeout)
    );
    let saved = observer.saved.lock().unwrap();
    assert_eq!(saved.len(), 2);
    assert_eq!(saved[1].auth_session.refresh_token, "rotated-refresh");
    let cache = AuthTokenCache::from_session(&saved[1].auth_session)
        .unwrap()
        .unwrap();
    assert!(cache.requires_core_refresh(DownstreamTokenScope::Cloud));
    assert!(!cache.requires_core_refresh(DownstreamTokenScope::Home));
    assert!(cache.sisu_token.is_some());
}

#[tokio::test]
async fn checkpoint_failure_stops_target_finalization() {
    let backend = Backend::default();
    let observer = Observer {
        fail_checkpoint: true,
        ..Default::default()
    };
    let mut session = fixture_session();
    let mut cache = AuthTokenCache::from_session(&session).unwrap().unwrap();
    cache.sisu_token = None;
    cache.stream_tokens["xHomeToken"]["_objectCreateTime"] = json!(1);
    session.token_cache_json = Some(serialize(&cache).unwrap());
    assert!(renew_auth_session_with(
        &backend,
        session,
        String::new(),
        DownstreamTokenScope::Home,
        false,
        &observer
    )
    .await
    .is_err());
    assert_eq!(*backend.requests.lock().unwrap(), ["oauth"]);
}

#[test]
fn rejected_token_comparison_reuses_newer_cached_credentials() {
    let session = fixture_session();
    assert!(token_was_rejected(&session, DownstreamTokenScope::Cloud, Some("cloud")).unwrap());
    assert!(
        !token_was_rejected(&session, DownstreamTokenScope::Cloud, Some("older-cloud")).unwrap()
    );
    assert!(token_was_rejected(&session, DownstreamTokenScope::Web, Some("web")).unwrap());
}

#[tokio::test]
async fn failed_cloud_recovery_keeps_home_warm_and_cloud_on_core_recovery() {
    let backend = Backend {
        fail_finalization: true,
        ..Default::default()
    };
    let observer = Observer::default();
    assert!(renew_auth_session_with(
        &backend,
        fixture_session(),
        String::new(),
        DownstreamTokenScope::Cloud,
        true,
        &observer
    )
    .await
    .is_err());
    let checkpoint = observer
        .saved
        .lock()
        .unwrap()
        .last()
        .unwrap()
        .auth_session
        .clone();
    let home_backend = Backend::default();
    let home_observer = Observer::default();
    renew_auth_session_with(
        &home_backend,
        checkpoint.clone(),
        String::new(),
        DownstreamTokenScope::Home,
        false,
        &home_observer,
    )
    .await
    .unwrap();
    assert!(home_backend.requests.lock().unwrap().is_empty());
    let cloud_backend = Backend::default();
    let cloud_observer = Observer::default();
    let renewed = renew_auth_session_with(
        &cloud_backend,
        checkpoint,
        String::new(),
        DownstreamTokenScope::Cloud,
        false,
        &cloud_observer,
    )
    .await
    .unwrap();
    assert_eq!(
        *cloud_backend.requests.lock().unwrap(),
        ["oauth", "finalize:cloud"]
    );
    assert!(!AuthTokenCache::from_session(&renewed)
        .unwrap()
        .unwrap()
        .requires_core_refresh(DownstreamTokenScope::Cloud));
}

#[tokio::test]
async fn rejected_scope_checkpoint_failure_prevents_oauth_requests() {
    let backend = Backend::default();
    let observer = Observer {
        fail_checkpoint: true,
        ..Default::default()
    };
    assert!(renew_auth_session_with(
        &backend,
        fixture_session(),
        String::new(),
        DownstreamTokenScope::Cloud,
        true,
        &observer
    )
    .await
    .is_err());
    assert!(backend.requests.lock().unwrap().is_empty());
}
