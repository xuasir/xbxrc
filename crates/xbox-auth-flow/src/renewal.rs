//! 两端共用目标续期、SISU 复用和一次核心恢复，持久化由宿主完成。
use crate::telemetry::flow_error_facts;
use crate::token_policy::{can_reuse_sisu_user_token, cloud_token_unavailable_error};
use crate::{
    AuthBundle, AuthFlowError, AuthFlowObserver, AuthFlowSeed, BuildDownstreamTokensInput,
    DownstreamTokenScope, FlowSisuTokenData,
};
use async_trait::async_trait;
use serde_json::json;
use xbox_webapi::{OAuthTokenResponse, WebApiError};

pub struct RenewAuthTokensInput {
    pub refresh_token: String,
    pub user_token: Option<OAuthTokenResponse>,
    pub sisu_token: Option<FlowSisuTokenData>,
    pub seed: AuthFlowSeed,
    pub force_region_ip: String,
    pub scope: DownstreamTokenScope,
    pub force_core_refresh: bool,
    pub refresh_token_rejected: bool,
}

#[derive(Debug)]
pub enum AuthRenewalError<E> {
    Flow(AuthFlowError),
    Checkpoint(E),
}

#[async_trait]
pub trait AuthRenewalObserver: AuthFlowObserver {
    type Error: Send;
    async fn checkpoint_credentials(&self, token: &OAuthTokenResponse) -> Result<(), Self::Error>;
}

#[async_trait]
pub trait AuthRenewalBackend: Send + Sync {
    async fn downstream(
        &self,
        input: BuildDownstreamTokensInput,
        scope: DownstreamTokenScope,
        observer: &dyn AuthFlowObserver,
    ) -> Result<AuthBundle, AuthFlowError>;
    async fn refresh_core(
        &self,
        refresh_token: &str,
        observer: &dyn AuthFlowObserver,
    ) -> Result<OAuthTokenResponse, AuthFlowError>;
    async fn finalize_core(
        &self,
        user: &OAuthTokenResponse,
        seed: &AuthFlowSeed,
        force_region_ip: &str,
        scope: DownstreamTokenScope,
        observer: &dyn AuthFlowObserver,
    ) -> Result<AuthBundle, AuthFlowError>;
}

pub async fn renew_auth_tokens<B: AuthRenewalBackend, O: AuthRenewalObserver>(
    backend: &B,
    input: RenewAuthTokensInput,
    observer: &O,
) -> Result<AuthBundle, AuthRenewalError<O::Error>> {
    let reuse_sisu = !input.force_core_refresh
        && input.sisu_token.as_ref().is_some_and(|token| {
            can_reuse_sisu_user_token(token, chrono::Utc::now().timestamp_millis())
        })
        && input.user_token.is_some();
    let scope = input.scope;
    observer.stage(
        "refreshPolicy",
        json!({ "scope": scope.as_str(), "skipCoreRefresh": reuse_sisu }),
    );
    if reuse_sisu {
        observer.stage(
            "downstreamRefreshStarted",
            json!({ "scope": scope.as_str() }),
        );
        let result = backend
            .downstream(
                BuildDownstreamTokensInput {
                    user_token: input.user_token.unwrap(),
                    sisu_token: input.sisu_token.unwrap(),
                    seed: input.seed.clone(),
                    force_region_ip: input.force_region_ip.clone(),
                },
                scope,
                observer,
            )
            .await;
        match result {
            Ok(bundle)
                if scope != DownstreamTokenScope::Cloud
                    || !cloud_token_unavailable_error(&bundle.stream_tokens)
                        .is_some_and(|error| error.to_status_code() == Some(401)) =>
            {
                return Ok(bundle)
            }
            Ok(_) => observer.stage(
                "downstreamTokenRejected",
                json!({ "statusCode": 401, "scope": scope.as_str() }),
            ),
            Err(error)
                if error
                    .upstream_error()
                    .is_some_and(|error| error.to_status_code() == Some(401)) =>
            {
                observer.stage("downstreamTokenRejected", flow_error_facts(&error))
            }
            Err(error) => {
                observer.stage("refreshFailed", flow_error_facts(&error));
                return Err(AuthRenewalError::Flow(error));
            }
        }
        observer.stage(
            "coreRefreshRetryStarted",
            json!({ "scope": scope.as_str(), "retryLimit": 1 }),
        );
    }
    if input.refresh_token_rejected || input.refresh_token.trim().is_empty() {
        return Err(AuthRenewalError::Flow(AuthFlowError::RefreshGrant(
            WebApiError::http(401, "refresh credentials unavailable"),
        )));
    }
    observer.stage(
        "coreAndDownstreamRefreshStarted",
        json!({ "scope": scope.as_str() }),
    );
    let user = backend
        .refresh_core(&input.refresh_token, observer)
        .await
        .map_err(|error| {
            observer.stage("refreshFailed", flow_error_facts(&error));
            AuthRenewalError::Flow(error)
        })?;
    observer
        .checkpoint_credentials(&user)
        .await
        .map_err(AuthRenewalError::Checkpoint)?;
    observer.stage(
        "coreCredentialsPersistCompleted",
        json!({ "scope": scope.as_str() }),
    );
    backend
        .finalize_core(&user, &input.seed, &input.force_region_ip, scope, observer)
        .await
        .map_err(|error| {
            observer.stage("refreshFailed", flow_error_facts(&error));
            AuthRenewalError::Flow(error)
        })
}

#[cfg(test)]
#[path = "renewal.test.rs"]
mod tests;
