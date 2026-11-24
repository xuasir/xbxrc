//! 目标凭据续期使用共享 SISU/核心轮换编排，宿主只持有提交与代次。
use super::*;
use xbox_auth_flow::{
    renew_auth_tokens, AuthBundle, AuthFlowError, AuthFlowObserver, AuthRenewalError,
    AuthRenewalObserver, RenewAuthTokensInput,
};

struct DesktopAuthRenewal<'a> {
    service: &'a AuthService,
    trace: &'a AuthOperationTrace,
    generation: u64,
}

impl AuthFlowObserver for DesktopAuthRenewal<'_> {
    fn stage(&self, stage: &'static str, facts: Value) {
        self.trace.stage(stage, facts);
    }
}

#[async_trait]
impl AuthRenewalObserver for DesktopAuthRenewal<'_> {
    type Error = AppError;
    async fn checkpoint_credentials(
        &self,
        token: &xbox_webapi::OAuthTokenResponse,
    ) -> AppResult<()> {
        self.service
            .runtime
            .checkpoint_credentials(self.generation, || {
                self.service.persistence.persist_user_token(token)
            })
    }
}

impl AuthService {
    pub(super) async fn start_silent_flow_impl(
        &self,
        trace: &AuthOperationTrace,
        generation: u64,
        force_core_refresh: bool,
        scope: AuthTokenScope,
    ) -> AppResult<()> {
        let sisu = if self.persistence.can_reuse_sisu_user_token()? {
            Some(self.persistence.load_flow_sisu_token()?)
        } else {
            None
        };
        let user = self
            .persistence
            .get_user_token()?
            .map(|token| serde_json::from_value(serde_json::to_value(token)?))
            .transpose()?;
        let observer = DesktopAuthRenewal {
            service: self,
            trace,
            generation,
        };
        let bundle = Box::pin(renew_auth_tokens(
            &AuthFlow::new(),
            RenewAuthTokensInput {
                refresh_token: self.persistence.get_refresh_token()?.unwrap_or_default(),
                user_token: user,
                sisu_token: sisu,
                seed: self.persistence.load_auth_flow_seed()?,
                force_region_ip: self.config_provider.get_force_region_ip(),
                scope: scope.flow_scope(),
                force_core_refresh,
                refresh_token_rejected: self.runtime.is_refresh_token_rejected(),
            },
            &observer,
        ))
        .await
        .map_err(|error| match error {
            AuthRenewalError::Flow(error) => self.map_refresh_grant_error(error, generation),
            AuthRenewalError::Checkpoint(error) => error,
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
