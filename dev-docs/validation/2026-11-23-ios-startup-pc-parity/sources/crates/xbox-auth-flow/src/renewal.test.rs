use super::*;
use crate::FlowTokenDetails;
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

fn user(refresh: &str) -> OAuthTokenResponse {
    serde_json::from_value(json!({"token_type":"Bearer","expires_in":3600,"scope":"xbox","access_token":"access","refresh_token":refresh})).unwrap()
}

fn sisu() -> FlowSisuTokenData {
    let details = FlowTokenDetails {
        issue_instant: None,
        not_after: Some((chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339()),
        token: Some("sisu".into()),
        display_claims: json!({}),
    };
    FlowSisuTokenData {
        device_token: "device".into(),
        title_token: details.clone(),
        user_token: details.clone(),
        authorization_token: details,
    }
}

fn bundle(refresh: &str) -> AuthBundle {
    AuthBundle {
        user_token: user(refresh),
        sisu_token: sisu(),
        web_token: json!({}),
        stream_tokens: json!({}),
        app_level: 0,
        token_update_time: 0,
    }
}

fn input(scope: DownstreamTokenScope, reuse_sisu: bool) -> RenewAuthTokensInput {
    RenewAuthTokensInput {
        refresh_token: "old-refresh".into(),
        user_token: Some(user("old-refresh")),
        sisu_token: reuse_sisu.then(sisu),
        seed: AuthFlowSeed {
            private_jwk: json!({}),
        },
        force_region_ip: String::new(),
        scope,
        force_core_refresh: false,
        refresh_token_rejected: false,
    }
}

struct Backend {
    events: Arc<Mutex<Vec<String>>>,
    downstream: Mutex<VecDeque<Result<AuthBundle, AuthFlowError>>>,
    core: Mutex<VecDeque<Result<OAuthTokenResponse, AuthFlowError>>>,
    finalized: Mutex<VecDeque<Result<AuthBundle, AuthFlowError>>>,
}

impl Backend {
    fn new(
        events: Arc<Mutex<Vec<String>>>,
        downstream: Vec<Result<AuthBundle, AuthFlowError>>,
        finalized: Result<AuthBundle, AuthFlowError>,
    ) -> Self {
        Self {
            events,
            downstream: Mutex::new(downstream.into()),
            core: Mutex::new(vec![Ok(user("rotated-refresh"))].into()),
            finalized: Mutex::new(vec![finalized].into()),
        }
    }
}

#[async_trait]
impl AuthRenewalBackend for Backend {
    async fn downstream(
        &self,
        _: BuildDownstreamTokensInput,
        scope: DownstreamTokenScope,
        _: &dyn AuthFlowObserver,
    ) -> Result<AuthBundle, AuthFlowError> {
        self.events
            .lock()
            .unwrap()
            .push(format!("downstream:{}", scope.as_str()));
        self.downstream
            .lock()
            .unwrap()
            .pop_front()
            .expect("bounded downstream request")
    }
    async fn refresh_core(
        &self,
        _: &str,
        _: &dyn AuthFlowObserver,
    ) -> Result<OAuthTokenResponse, AuthFlowError> {
        self.events.lock().unwrap().push("oauth".into());
        self.core
            .lock()
            .unwrap()
            .pop_front()
            .expect("one core recovery")
    }
    async fn finalize_core(
        &self,
        _: &OAuthTokenResponse,
        _: &AuthFlowSeed,
        _: &str,
        scope: DownstreamTokenScope,
        _: &dyn AuthFlowObserver,
    ) -> Result<AuthBundle, AuthFlowError> {
        self.events
            .lock()
            .unwrap()
            .push(format!("finalize:{}", scope.as_str()));
        self.finalized
            .lock()
            .unwrap()
            .pop_front()
            .expect("one scoped finalization")
    }
}

#[derive(Default)]
struct Observer {
    events: Arc<Mutex<Vec<String>>>,
    persisted: Mutex<Option<String>>,
    checkpoint_fails: bool,
    stages: Mutex<Vec<Value>>,
}
impl AuthFlowObserver for Observer {
    fn stage(&self, stage: &'static str, facts: Value) {
        self.stages
            .lock()
            .unwrap()
            .push(json!({"stage":stage,"facts":facts}));
    }
}
#[async_trait]
impl AuthRenewalObserver for Observer {
    type Error = &'static str;
    async fn checkpoint_credentials(&self, token: &OAuthTokenResponse) -> Result<(), Self::Error> {
        self.events.lock().unwrap().push("checkpoint".into());
        if self.checkpoint_fails {
            return Err("storage");
        }
        *self.persisted.lock().unwrap() = Some(token.refresh_token.clone());
        Ok(())
    }
}

#[tokio::test]
async fn valid_sisu_refreshes_each_target_without_core_requests() {
    for scope in [
        DownstreamTokenScope::Web,
        DownstreamTokenScope::Home,
        DownstreamTokenScope::Cloud,
    ] {
        let observer = Observer::default();
        let backend = Backend::new(
            observer.events.clone(),
            vec![Ok(bundle("old-refresh"))],
            Ok(bundle("unused")),
        );
        renew_auth_tokens(&backend, input(scope, true), &observer)
            .await
            .unwrap();
        assert_eq!(
            *observer.events.lock().unwrap(),
            [format!("downstream:{}", scope.as_str())]
        );
        assert!(observer.persisted.lock().unwrap().is_none());
    }
}

#[tokio::test]
async fn downstream_401_recovers_core_once_and_checkpoints_before_target_requests() {
    let observer = Observer::default();
    let backend = Backend::new(
        observer.events.clone(),
        vec![Err(WebApiError::http(401, "secret").into())],
        Ok(bundle("rotated-refresh")),
    );
    renew_auth_tokens(&backend, input(DownstreamTokenScope::Home, true), &observer)
        .await
        .unwrap();
    assert_eq!(
        *observer.events.lock().unwrap(),
        ["downstream:home", "oauth", "checkpoint", "finalize:home"]
    );
    assert_eq!(
        observer.persisted.lock().unwrap().as_deref(),
        Some("rotated-refresh")
    );
}

#[tokio::test]
async fn cloud_diagnostic_401_recovers_once() {
    let observer = Observer::default();
    let mut rejected = bundle("old-refresh");
    rejected.stream_tokens =
        json!({"_diagnostics":{"xCloudToken":{"xgpuwebf2p":{"statusCode":401}}}});
    let backend = Backend::new(
        observer.events.clone(),
        vec![Ok(rejected)],
        Ok(bundle("rotated-refresh")),
    );
    renew_auth_tokens(
        &backend,
        input(DownstreamTokenScope::Cloud, true),
        &observer,
    )
    .await
    .unwrap();
    assert_eq!(
        *observer.events.lock().unwrap(),
        ["downstream:cloud", "oauth", "checkpoint", "finalize:cloud"]
    );
}

#[tokio::test]
async fn transient_downstream_failure_preserves_credentials_and_stops_core_recovery() {
    let observer = Observer::default();
    let backend = Backend::new(
        observer.events.clone(),
        vec![Err(WebApiError::network_kind(
            xbox_webapi::NetworkErrorKind::Timeout,
        )
        .into())],
        Ok(bundle("unused")),
    );
    let error = renew_auth_tokens(
        &backend,
        input(DownstreamTokenScope::Cloud, true),
        &observer,
    )
    .await
    .unwrap_err();
    assert!(matches!(error,AuthRenewalError::Flow(ref error) if error.is_retriable()));
    assert_eq!(*observer.events.lock().unwrap(), ["downstream:cloud"]);
}

#[tokio::test]
async fn core_rotation_survives_downstream_timeout() {
    let observer = Observer::default();
    let backend = Backend::new(
        observer.events.clone(),
        vec![],
        Err(WebApiError::network_kind(xbox_webapi::NetworkErrorKind::Timeout).into()),
    );
    let error = renew_auth_tokens(
        &backend,
        input(DownstreamTokenScope::Home, false),
        &observer,
    )
    .await
    .unwrap_err();
    assert!(matches!(error,AuthRenewalError::Flow(ref error) if error.is_retriable()));
    assert_eq!(
        observer.persisted.lock().unwrap().as_deref(),
        Some("rotated-refresh")
    );
    assert_eq!(
        *observer.events.lock().unwrap(),
        ["oauth", "checkpoint", "finalize:home"]
    );
}

#[tokio::test]
async fn checkpoint_failure_prevents_downstream_requests() {
    let observer = Observer {
        checkpoint_fails: true,
        ..Default::default()
    };
    let backend = Backend::new(observer.events.clone(), vec![], Ok(bundle("unused")));
    assert!(matches!(
        renew_auth_tokens(&backend, input(DownstreamTokenScope::Web, false), &observer).await,
        Err(AuthRenewalError::Checkpoint("storage"))
    ));
    assert_eq!(*observer.events.lock().unwrap(), ["oauth", "checkpoint"]);
}

#[tokio::test]
async fn forced_core_refresh_skips_a_valid_sisu_and_keeps_target_scope() {
    let observer = Observer::default();
    let backend = Backend::new(
        observer.events.clone(),
        vec![],
        Ok(bundle("rotated-refresh")),
    );
    let mut request = input(DownstreamTokenScope::Cloud, true);
    request.force_core_refresh = true;
    renew_auth_tokens(&backend, request, &observer)
        .await
        .unwrap();
    assert_eq!(
        *observer.events.lock().unwrap(),
        ["oauth", "checkpoint", "finalize:cloud"]
    );
}

#[tokio::test]
async fn rejected_refresh_credentials_require_login_without_network_work() {
    let observer = Observer::default();
    let backend = Backend::new(observer.events.clone(), vec![], Ok(bundle("unused")));
    let mut request = input(DownstreamTokenScope::Home, false);
    request.refresh_token_rejected = true;
    assert!(
        matches!(renew_auth_tokens(&backend,request,&observer).await,Err(AuthRenewalError::Flow(ref error)) if error.requires_interactive_login())
    );
    assert!(observer.events.lock().unwrap().is_empty());
}

#[tokio::test]
async fn request_telemetry_exposes_elapsed_classification_and_discards_raw_details() {
    let observer = Observer::default();
    let _: Result<(), _> = crate::telemetry::observe_request(Some(&observer), "webXsts", async {
        Err(WebApiError::http(
            503,
            "https://secret.example/token=private",
        ))
    })
    .await;
    let stages = observer.stages.lock().unwrap();
    assert_eq!(stages[0]["facts"]["requestStage"], "webXsts");
    assert_eq!(stages[1]["facts"]["statusCode"], 503);
    assert!(stages[1]["facts"]["elapsedMs"].is_u64());
    assert!(!serde_json::to_string(&*stages).unwrap().contains("secret"));
}
