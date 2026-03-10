use crate::error::{AppError, AppResult};
use crate::mods::auth::storage_repository::AuthStorageRepository;
use crate::mods::auth::token_policy::{AuthTokenPolicy, ValidSessionSnapshot};
use crate::mods::auth::types::{SisuTokenData, UserTokenData};
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
            .ok_or_else(|| AppError::Data("Missing JWT private key".to_string()))?;
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

    pub fn get_cached_app_level(&self) -> u32 {
        self.token_policy.get_cached_app_level().unwrap_or(0)
    }

    pub fn has_valid_auth_tokens(&self) -> bool {
        self.storage_repository.has_valid_auth_tokens()
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

    pub fn get_token_update_time(&self) -> u64 {
        self.storage_repository.get_token_update_time()
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

    pub fn get_streaming_token(&self, target_type: &str) -> AppResult<Option<Value>> {
        let tokens = self.get_stream_tokens()?;
        let Some(tokens) = tokens else {
            return Ok(None);
        };

        let token = if target_type == "home" {
            tokens.get("xHomeToken")
        } else {
            tokens.get("xCloudToken")
        };

        if self.token_policy.is_stream_token_valid(token) {
            return Ok(token.cloned());
        }

        Ok(None)
    }

    pub fn get_web_api_tokens(&self) -> AppResult<Option<(String, String)>> {
        let web_token = self.get_web_token()?;
        let Some(web_token) = web_token else {
            return Ok(None);
        };

        let data = web_token.get("data").unwrap_or_else(|| {
            log::warn!("Web token missing 'data' field, using full token");
            &web_token
        });
        let uhs = data
            .get("DisplayClaims")
            .and_then(|value| value.get("xui"))
            .and_then(|value| value.as_array())
            .and_then(|xui| xui.first())
            .and_then(|first| first.get("uhs"))
            .and_then(|value| value.as_str());
        let token = data.get("Token").and_then(|value| value.as_str());

        if let (Some(uhs), Some(token)) = (uhs, token) {
            return Ok(Some((uhs.to_string(), token.to_string())));
        }

        Ok(None)
    }
}
