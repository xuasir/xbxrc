//! 目标凭据续期共用 SISU 复用、核心轮换落盘和有界恢复流程。
use super::*;
use xbox_auth_flow::{AuthBundle, AuthFlowError};

impl AuthService {
    pub(super) async fn start_silent_flow_impl(
        &self,
        trace: &AuthOperationTrace,
        generation: u64,
        force_core_refresh: bool,
        scope: AuthTokenScope,
    ) -> AppResult<()> {
        let reuse_sisu = !force_core_refresh && self.persistence.can_reuse_sisu_user_token()?;
        trace.stage(
            "refreshPolicy",
            json!({ "scope": scope.as_str(), "skipCoreRefresh": reuse_sisu }),
        );
        if reuse_sisu {
            let user_token = self
                .persistence
                .get_user_token()?
                .ok_or_else(|| AppError::Auth("登录续期凭据暂不可用，请选择重新登录".into()))?;
            let input = BuildDownstreamTokensInput {
                user_token: serde_json::from_value(serde_json::to_value(user_token)?)?,
                sisu_token: self.persistence.load_flow_sisu_token()?,
                seed: self.persistence.load_auth_flow_seed()?,
                force_region_ip: self.config_provider.get_force_region_ip(),
            };
            trace.stage(
                "downstreamRefreshStarted",
                json!({ "scope": scope.as_str() }),
            );
            let result = AuthFlow::new()
                .build_downstream_tokens_for(input, scope.flow_scope())
                .await;
            match result {
                Ok(output) if !cloud_unauthorized(&output.auth_bundle, scope) => {
                    return self.persist_renewed_bundle(
                        output.auth_bundle,
                        scope,
                        generation,
                        trace,
                    );
                }
                Ok(_) => {}
                Err(error)
                    if error
                        .upstream_error()
                        .is_some_and(|error| error.to_status_code() == Some(401)) =>
                {
                    trace.stage("downstreamTokenRejected", flow_error_summary(&error));
                }
                Err(error) => {
                    trace.stage("refreshFailed", flow_error_summary(&error));
                    return Err(auth_flow_app_error(error));
                }
            }
            // 下游拒绝有效期内的 SISU 后，使用 refresh grant 恢复一次。
            trace.stage(
                "coreRefreshRetryStarted",
                json!({ "scope": scope.as_str(), "retryLimit": 1 }),
            );
        }
        self.refresh_core_tokens(scope, generation, trace).await
    }

    async fn refresh_core_tokens(
        &self,
        scope: AuthTokenScope,
        generation: u64,
        trace: &AuthOperationTrace,
    ) -> AppResult<()> {
        if self.runtime.is_refresh_token_rejected() {
            return Err(AppError::Auth("登录续期凭据已失效，请选择重新登录".into()));
        }
        let refresh_token = self
            .persistence
            .get_refresh_token()?
            .ok_or_else(|| AppError::Auth("登录续期凭据暂不可用，请选择重新登录".into()))?;
        let seed = self.persistence.load_auth_flow_seed()?;
        let flow = AuthFlow::new();
        trace.stage(
            "coreAndDownstreamRefreshStarted",
            json!({ "scope": scope.as_str() }),
        );
        let user_token = flow
            .refresh_user_token(&refresh_token)
            .await
            .map_err(|error| {
                trace.stage("refreshFailed", flow_error_summary(&error));
                self.map_refresh_grant_error(error, generation)
            })?;
        self.runtime.checkpoint_credentials(generation, || {
            self.persistence.persist_user_token(&user_token)
        })?;
        trace.stage(
            "coreCredentialsPersistCompleted",
            json!({ "scope": scope.as_str() }),
        );
        let bundle = flow
            .finalize_user_token_for(
                &user_token,
                &seed,
                &self.config_provider.get_force_region_ip(),
                scope.flow_scope(),
            )
            .await
            .map_err(|error| {
                trace.stage("refreshFailed", flow_error_summary(&error));
                auth_flow_app_error(error)
            })?;
        self.persist_renewed_bundle(bundle, scope, generation, trace)
    }

    fn persist_renewed_bundle(
        &self,
        bundle: AuthBundle,
        scope: AuthTokenScope,
        generation: u64,
        trace: &AuthOperationTrace,
    ) -> AppResult<()> {
        trace.stage(
            "authBundlePersistStarted",
            token_summary(&bundle.stream_tokens),
        );
        self.runtime
            .commit_scoped_authentication(generation, bundle.app_level, scope, || {
                self.persistence.persist_refreshed_bundle(&bundle, scope)
            })?;
        trace.stage(
            "authBundlePersistCompleted",
            json!({ "scope": scope.as_str(), "appLevel": self.get_state().app_level }),
        );
        Ok(())
    }

    pub(super) fn map_refresh_grant_error(
        &self,
        error: AuthFlowError,
        generation: u64,
    ) -> AppError {
        if error.requires_interactive_login() {
            if let Err(error) = self.runtime.reject_refresh_token(generation) {
                return error;
            }
        }
        auth_flow_app_error(error)
    }
}

fn cloud_unauthorized(bundle: &AuthBundle, scope: AuthTokenScope) -> bool {
    scope == AuthTokenScope::Cloud
        && cloud_token_unavailable_error(&bundle.stream_tokens)
            .is_some_and(|error| error.to_status_code() == Some(401))
}
