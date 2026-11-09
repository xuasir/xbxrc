use crate::error::{AppError, AppResult};
use crate::mods::auth::storage_repository::AuthStorageRepository;
use crate::mods::auth::token_policy::{
    can_reuse_sisu_user_token, merge_stream_tokens, resolve_valid_web_api_tokens, same_web_account,
    AuthTokenPolicy, ValidSessionSnapshot,
};
use crate::mods::auth::types::{AuthTokenScope, SisuTokenData, UserTokenData};
use serde_json::Value;
use tauri::AppHandle;
use xbox_auth_flow::{AuthBundle, AuthFlowSeed, FlowSisuTokenData};

pub struct AuthPersistenceService {
    storage_repository: AuthStorageRepository,
    token_policy: AuthTokenPolicy,
}

impl AuthPersistenceService {
    pub fn new(app_handle: AppHandle) -> Self {
        let storage_repository = AuthStorageRepository::new(app_handle.clone());
        let token_policy = AuthTokenPolicy::new(AuthStorageRepository::new(app_handle));

        Self {
            storage_repository,
            token_policy,
        }
    }

    pub fn initial_auth_status(&self) -> (bool, u32) {
        let is_authenticated = self
            .token_policy
            .get_valid_session_snapshot()
            .ok()
            .flatten()
            .is_some();
        let app_level = self.token_policy.get_cached_app_level().unwrap_or(0);
        (is_authenticated, app_level)
    }

    pub fn save_private_jwk(&self, private_jwk: Value) -> AppResult<()> {
        self.storage_repository
            .set_jwt_private_jwk(private_jwk)
            .map_err(AppError::Data)
    }

    pub fn load_auth_flow_seed(&self) -> AppResult<AuthFlowSeed> {
        let private_jwk = self
            .storage_repository
            .get_jwt_private_jwk()
            .map_err(AppError::Data)?
            .ok_or_else(|| AppError::Auth("登录认证密钥暂不可用，请选择重新登录".to_string()))?;
        Ok(AuthFlowSeed { private_jwk })
    }

    pub fn load_flow_sisu_token(&self) -> AppResult<FlowSisuTokenData> {
        let sisu_token = self
            .storage_repository
            .get_sisu_token()
            .map_err(AppError::Data)?
            .ok_or_else(|| AppError::Data("Missing Sisu token".to_string()))?;
        serde_json::from_value(serde_json::to_value(sisu_token)?).map_err(AppError::Json)
    }

    pub fn persist_auth_bundle(&self, auth_bundle: &AuthBundle) -> AppResult<()> {
        // 协议结果统一在这里落仓，避免编排层反复做 DTO 映射。
        let user_token: UserTokenData =
            serde_json::from_value(serde_json::to_value(&auth_bundle.user_token)?)?;
        let sisu_token: SisuTokenData =
            serde_json::from_value(serde_json::to_value(&auth_bundle.sisu_token)?)?;

        self.storage_repository
            .set_user_token(user_token)
            .map_err(AppError::Data)?;
        self.storage_repository
            .set_sisu_token(sisu_token)
            .map_err(AppError::Data)?;
        self.token_policy
            .set_web_token(auth_bundle.web_token.clone())
            .map_err(AppError::Data)?;
        self.token_policy
            .set_stream_tokens(auth_bundle.stream_tokens.clone())
            .map_err(AppError::Data)?;

        Ok(())
    }

    pub fn get_valid_session_snapshot(&self) -> AppResult<Option<ValidSessionSnapshot>> {
        self.token_policy
            .get_valid_session_snapshot()
            .map_err(AppError::Data)
    }

    pub fn get_valid_session_for(
        &self,
        scope: AuthTokenScope,
    ) -> AppResult<Option<ValidSessionSnapshot>> {
        self.token_policy
            .get_valid_session_for(scope)
            .map_err(AppError::Data)
    }

    pub fn persist_refreshed_bundle(
        &self,
        bundle: &AuthBundle,
        scope: AuthTokenScope,
    ) -> AppResult<()> {
        let same_account = self
            .get_web_token()?
            .as_ref()
            .is_some_and(|old| same_web_account(old, &bundle.web_token));
        let mut bundle = bundle.clone();
        bundle.stream_tokens = merge_stream_tokens(
            self.get_stream_tokens()?,
            &bundle.stream_tokens,
            scope,
            same_account,
        );
        self.persist_auth_bundle(&bundle)
    }

    pub fn persist_user_token(&self, token: &xbox_webapi::OAuthTokenResponse) -> AppResult<()> {
        let token = serde_json::from_value(serde_json::to_value(token)?)?;
        self.storage_repository
            .set_user_token(token)
            .map_err(AppError::Data)
    }

    pub fn persist_rotated_refresh_token(&self, refresh_token: &str) -> AppResult<()> {
        let mut token = self
            .get_user_token()?
            .ok_or_else(|| AppError::Auth("登录续期凭据暂不可用，请选择重新登录".into()))?;
        token.refresh_token = refresh_token.to_string();
        self.storage_repository
            .set_user_token(token)
            .map_err(AppError::Data)
    }

    pub fn get_cached_app_level(&self) -> u32 {
        self.token_policy.get_cached_app_level().unwrap_or(0)
    }

    pub fn get_user_token(&self) -> AppResult<Option<UserTokenData>> {
        self.storage_repository
            .get_user_token()
            .map_err(AppError::Data)
    }

    pub fn get_refresh_token(&self) -> AppResult<Option<String>> {
        Ok(self
            .get_user_token()?
            .map(|token| token.refresh_token.trim().to_string())
            .filter(|token| !token.is_empty()))
    }

    pub fn can_reuse_sisu_user_token(&self) -> AppResult<bool> {
        Ok(self
            .storage_repository
            .get_sisu_token()
            .map_err(AppError::Data)?
            .as_ref()
            .is_some_and(|token| {
                can_reuse_sisu_user_token(token, chrono::Utc::now().timestamp_millis())
            }))
    }

    pub fn get_stream_tokens(&self) -> AppResult<Option<Value>> {
        self.token_policy
            .get_stream_tokens()
            .map_err(AppError::Data)
    }

    pub fn get_web_token(&self) -> AppResult<Option<Value>> {
        self.token_policy.get_web_token().map_err(AppError::Data)
    }

    pub fn clear_all_tokens(&self) -> AppResult<()> {
        self.storage_repository
            .clear_all_tokens()
            .map_err(AppError::Data)
    }

    pub fn clear_ephemeral_tokens(&self) -> AppResult<()> {
        self.token_policy
            .clear_ephemeral_tokens()
            .map_err(AppError::Data)
    }

    pub fn get_web_api_tokens(&self) -> AppResult<Option<(String, String)>> {
        let web_token = self.get_web_token()?;
        let Some(web_token) = web_token else {
            return Ok(None);
        };

        Ok(resolve_valid_web_api_tokens(
            &web_token,
            chrono::Utc::now().timestamp_millis(),
        ))
    }
}
