use crate::error::{AppError, AppResult};
use crate::mods::auth::client::{XalRedirectFlow, XboxWebApiClient};
use crate::mods::auth::repository::CoreTokenRepository;
use crate::mods::auth::token_repository::AuthTokenRepository;
use crate::mods::auth::transfer_token_service::AuthTransferTokenService;
use crate::mods::auth::types::{
    AuthClaims, AuthSessionReadyEvent, AuthState, CheckAuthResponse, LoginResponse, SisuTokenData,
    TitleClaims, TokenDetails, UserClaims, UserTokenData,
};
use crate::mods::auth::AuthProvider;
use crate::mods::config::ConfigProviderRef;
use async_trait::async_trait;
use serde_json::Value;
use tauri::{AppHandle, Manager};

const REFRESH_SKIP_WINDOW_MS: u64 = 23 * 60 * 60 * 1000;
pub const AUTH_WINDOW_LABEL: &str = "auth-oauth-window";

pub struct AuthService {
    app_handle: AppHandle,
    config_provider: ConfigProviderRef,
    core_repository: CoreTokenRepository,
    token_repository: AuthTokenRepository,
    transfer_token_service: AuthTransferTokenService,
    inner: std::sync::Mutex<AuthServiceInner>,
}

struct AuthServiceInner {
    state: AuthState,
    pending_redirect_flow: Option<XalRedirectFlow>,
}

impl AuthService {
    pub fn new(app_handle: AppHandle, config_provider: ConfigProviderRef) -> Self {
        let core_repository = CoreTokenRepository::new(app_handle.clone());
        let token_repository =
            AuthTokenRepository::new(CoreTokenRepository::new(app_handle.clone()));
        let transfer_token_service =
            AuthTransferTokenService::new(CoreTokenRepository::new(app_handle.clone()));

        let provider = "xal".to_string();
        let is_authenticated = token_repository
            .get_valid_session_snapshot()
            .ok()
            .flatten()
            .is_some();
        let app_level = token_repository.get_cached_app_level().unwrap_or(0);

        Self {
            app_handle,
            config_provider,
            core_repository,
            token_repository,
            transfer_token_service,
            inner: std::sync::Mutex::new(AuthServiceInner {
                state: AuthState {
                    provider,
                    is_authenticating: false,
                    is_authenticated,
                    app_level,
                },
                pending_redirect_flow: None,
            }),
        }
    }

    fn close_auth_window(&self) {
        if let Some(window) = self.app_handle.get_webview_window(AUTH_WINDOW_LABEL) {
            let _ = window.close();
        }
    }

    async fn login_impl(&self) -> AppResult<LoginResponse> {
        eprintln!("[auth][login] start");
        {
            let mut inner = self.inner.lock().map_err(|e| {
                AppError::Internal(format!("Failed to acquire auth service lock: {}", e))
            })?;
            inner.state.is_authenticating = true;
        }

        let result: AppResult<LoginResponse> = async {
            let client = XboxWebApiClient::new();
            let jwt_keys =
                XboxWebApiClient::generate_ecdsa_keypair().map_err(|e| AppError::Data(e))?;
            let private_jwk = jwt_keys
                .private_jwk
                .as_ref()
                .ok_or_else(|| AppError::Data("Missing private JWK".to_string()))?;
            self.core_repository
                .set_jwt_private_jwk(private_jwk.clone())
                .map_err(|e| AppError::Data(e))?;

            let device_uuid = uuid::Uuid::new_v4().to_string();
            let serial_number = uuid::Uuid::new_v4().to_string();
            let device_token_resp = client
                .get_device_token(
                    "0000000048093EE3",
                    &device_uuid,
                    &serial_number,
                    "15.0",
                    private_jwk,
                )
                .await
                .map_err(|e| AppError::Data(e))?;
            let device_token = device_token_resp
                .get("Token")
                .and_then(|value| value.as_str())
                .ok_or_else(|| AppError::Data("Invalid Device Token Response".to_string()))?;

            let code_challenge = XboxWebApiClient::create_code_challenge();
            let state_str = XboxWebApiClient::get_random_state();

            let sisu_auth = client
                .do_sisu_authentication(device_token, &code_challenge, &state_str, private_jwk)
                .await
                .map_err(|e| AppError::Data(e))?;

            {
                let mut inner = self.inner.lock().map_err(|e| {
                    AppError::Internal(format!("Failed to acquire auth service lock: {}", e))
                })?;
                inner.pending_redirect_flow = Some(XalRedirectFlow {
                    sisu_auth: sisu_auth.clone(),
                    state: state_str.clone(),
                    code_challenge,
                });
            }

            Ok(LoginResponse {
                mode: "oauth-window".to_string(),
                url: sisu_auth["MsaOauthRedirect"]
                    .as_str()
                    .unwrap_or("")
                    .to_string(),
                state: state_str,
            })
        }
        .await;

        if result.is_err() {
            let mut inner = self.inner.lock().map_err(|e| {
                AppError::Internal(format!("Failed to acquire auth service lock: {}", e))
            })?;
            inner.pending_redirect_flow = None;
            inner.state.is_authenticating = false;
            eprintln!("[auth][login] failed");
        }

        result
    }

    async fn logout_impl(&self) -> AppResult<()> {
        self.close_auth_window();
        self.core_repository
            .clear_all_tokens()
            .map_err(|e| AppError::Data(e))?;

        let mut inner = self.inner.lock().map_err(|e| {
            AppError::Internal(format!("Failed to acquire auth service lock: {}", e))
        })?;
        inner.pending_redirect_flow = None;
        inner.state.is_authenticated = false;
        inner.state.is_authenticating = false;
        inner.state.app_level = 0;
        Ok(())
    }

    async fn clear_auth_cache_impl(&self, scope: &str) -> AppResult<()> {
        self.close_auth_window();
        if scope == "all" {
            self.core_repository
                .clear_all_tokens()
                .map_err(|e| AppError::Data(e))?;
        } else {
            self.token_repository
                .clear_ephemeral_tokens()
                .map_err(|e| AppError::Data(e))?;
        }

        let mut inner = self.inner.lock().map_err(|e| {
            AppError::Internal(format!("Failed to acquire auth service lock: {}", e))
        })?;
        inner.pending_redirect_flow = None;
        inner.state.is_authenticated = false;
        inner.state.is_authenticating = false;
        inner.state.app_level = 0;
        Ok(())
    }

    async fn reset_runtime_after_store_purge_impl(&self) {
        self.close_auth_window();
        if let Ok(mut inner) = self.inner.lock() {
            inner.pending_redirect_flow = None;
            inner.state.is_authenticated = false;
            inner.state.is_authenticating = false;
            inner.state.app_level = 0;
        } else {
            log::warn!("Failed to acquire auth service lock during reset_runtime_after_store_purge");
        }
    }

    async fn check_authentication_impl(&self) -> AppResult<CheckAuthResponse> {
        eprintln!("[auth][check] start");
        {
            let mut inner = self.inner.lock().map_err(|e| {
                AppError::Internal(format!("Failed to acquire auth service lock: {}", e))
            })?;
            inner.state.is_authenticating = true;
        }

        if let Some(snapshot) = self
            .token_repository
            .get_valid_session_snapshot()
            .map_err(|e| AppError::Data(e))?
        {
            let mut inner = self.inner.lock().map_err(|e| {
                AppError::Internal(format!("Failed to acquire auth service lock: {}", e))
            })?;
            inner.state.is_authenticating = false;
            inner.state.is_authenticated = true;
            inner.state.app_level = snapshot.app_level;

            return Ok(CheckAuthResponse {
                provider: inner.state.provider.clone(),
                started_silent_flow: false,
            });
        }

        let should_start_silent_flow = self.core_repository.has_valid_auth_tokens()
            || self
                .core_repository
                .get_user_token()
                .map_err(|e| AppError::Data(e))?
                .is_some();

        if should_start_silent_flow {
            eprintln!("[auth][check] start silent flow");
            if self.prepare_silent_flow().await.is_ok() && self.complete_sisu_login().await.is_ok()
            {
                let mut inner = self.inner.lock().map_err(|e| {
                    AppError::Internal(format!("Failed to acquire auth service lock: {}", e))
                })?;
                inner.state.is_authenticating = false;
                inner.state.is_authenticated = true;
                return Ok(CheckAuthResponse {
                    provider: inner.state.provider.clone(),
                    started_silent_flow: true,
                });
            }

            // 与 Electron XAL 对齐：silent 失败后清理所有 token。
            self.core_repository
                .clear_all_tokens()
                .map_err(|e| AppError::Data(e))?;
        }

        let mut inner = self.inner.lock().map_err(|e| {
            AppError::Internal(format!("Failed to acquire auth service lock: {}", e))
        })?;
        inner.state.is_authenticating = false;
        inner.state.is_authenticated = false;
        inner.state.app_level = 0;
        Ok(CheckAuthResponse {
            provider: inner.state.provider.clone(),
            started_silent_flow: false,
        })
    }

    async fn handle_oauth_callback_impl(&self, callback_url: &str) -> AppResult<()> {
        eprintln!("[auth][callback] receive url={}", callback_url);

        let flow = {
            let mut inner = self.inner.lock().map_err(|e| {
                AppError::Internal(format!("Failed to acquire auth service lock: {}", e))
            })?;
            match inner.pending_redirect_flow.take() {
                Some(flow) => flow,
                None => {
                    inner.state.is_authenticating = false;
                    return Err(AppError::Data("No pending redirect flow".to_string()));
                }
            }
        };

        let url =
            url::Url::parse(callback_url).map_err(|error| AppError::Data(error.to_string()))?;

        if url.query_pairs().any(|(key, _)| key == "error") {
            let mut inner = self.inner.lock().map_err(|e| {
                AppError::Internal(format!("Failed to acquire auth service lock: {}", e))
            })?;
            inner.state.is_authenticating = false;
            return Err(AppError::Data("OAuth callback contains error".to_string()));
        }

        let code = url
            .query_pairs()
            .find(|(key, _)| key == "code")
            .map(|(_, value)| value.to_string())
            .ok_or_else(|| AppError::Data("Missing code in callback".to_string()))?;

        let state = url
            .query_pairs()
            .find(|(key, _)| key == "state")
            .map(|(_, value)| value.to_string())
            .ok_or_else(|| AppError::Data("Missing state in callback".to_string()))?;

        if state != flow.state {
            let mut inner = self.inner.lock().map_err(|e| {
                AppError::Internal(format!("Failed to acquire auth service lock: {}", e))
            })?;
            inner.state.is_authenticating = false;
            return Err(AppError::Data(
                "State mismatch in OAuth callback".to_string(),
            ));
        }

        let client = XboxWebApiClient::new();
        let user_token_val = client
            .exchange_code_for_token(&code, &flow.code_challenge.verifier)
            .await
            .map_err(|e| AppError::Data(e))?;

        let user_token: UserTokenData = serde_json::from_value(user_token_val)
            .map_err(|error| AppError::Data(error.to_string()))?;
        self.core_repository
            .set_user_token(user_token)
            .map_err(|e| AppError::Data(e))?;

        match self.complete_sisu_login().await {
            Ok(()) => Ok(()),
            Err(error) => {
                let mut inner = self.inner.lock().map_err(|e| {
                    AppError::Internal(format!("Failed to acquire auth service lock: {}", e))
                })?;
                inner.state.is_authenticating = false;
                Err(AppError::Data(error))
            }
        }
    }

    async fn cancel_pending_login_impl(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            if inner.pending_redirect_flow.is_some() || inner.state.is_authenticating {
                inner.pending_redirect_flow = None;
                inner.state.is_authenticating = false;
                inner.state.is_authenticated = false;
                inner.state.app_level = 0;
            }
        } else {
            log::warn!("Failed to acquire auth service lock during cancel_pending_login");
        }
    }

    async fn prepare_silent_flow(&self) -> Result<(), String> {
        let stream_tokens = self.token_repository.get_stream_tokens()?;
        let web_token = self.token_repository.get_web_token()?;

        let has_any_cached_token = stream_tokens
            .as_ref()
            .map(|tokens| tokens.get("xHomeToken").is_some() || tokens.get("xCloudToken").is_some())
            .unwrap_or(false)
            && web_token.is_some();

        let token_update_time = self.core_repository.get_token_update_time();
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or(0);

        let should_skip_refresh = has_any_cached_token
            && token_update_time > 0
            && now_ms.saturating_sub(token_update_time) < REFRESH_SKIP_WINDOW_MS;

        if should_skip_refresh {
            return Ok(());
        }

        let user_token = self
            .core_repository
            .get_user_token()?
            .ok_or("Missing user token")?;

        let refreshed_user_token = XboxWebApiClient::new()
            .refresh_user_token(&user_token.refresh_token)
            .await?;

        let parsed_user_token: UserTokenData =
            serde_json::from_value(refreshed_user_token).map_err(|error| error.to_string())?;

        self.core_repository.set_user_token(parsed_user_token)
    }

    async fn complete_sisu_login(&self) -> Result<(), String> {
        let client = XboxWebApiClient::new();
        let private_jwk = self
            .core_repository
            .get_jwt_private_jwk()?
            .ok_or("Missing JWT private key")?;
        let user_token = self
            .core_repository
            .get_user_token()?
            .ok_or("Missing user token")?;

        let device_uuid = uuid::Uuid::new_v4().to_string();
        let serial_number = uuid::Uuid::new_v4().to_string();
        let device_token_resp = client
            .get_device_token(
                "0000000048093EE3",
                &device_uuid,
                &serial_number,
                "15.0",
                &private_jwk,
            )
            .await?;

        let device_token = device_token_resp
            .get("Token")
            .and_then(|value| value.as_str())
            .ok_or("Invalid Device Token Response")?;

        let sisu_auth_res = client
            .do_sisu_authorization(&user_token.access_token, device_token, &private_jwk)
            .await?;

        let sisu_data = SisuTokenData {
            // 与 Electron 行为对齐：授权响应未返回 DeviceToken 时回填当前请求值。
            device_token: device_token.to_string(),
            title_token: parse_token_details::<TitleClaims>(&sisu_auth_res, "TitleToken")
                .map_err(|error| format!("Failed to parse sisu title token: {}", error))?,
            user_token: parse_token_details::<UserClaims>(&sisu_auth_res, "UserToken")
                .map_err(|error| format!("Failed to parse sisu user token: {}", error))?,
            authorization_token: parse_token_details::<AuthClaims>(
                &sisu_auth_res,
                "AuthorizationToken",
            )
            .map_err(|error| format!("Failed to parse sisu authorization token: {}", error))?,
        };

        let auth_token_resp = client
            .do_xsts_authorization(
                &sisu_data.user_token.token,
                "http://xboxlive.com",
                &private_jwk,
            )
            .await?;

        let gssv_token_resp = client
            .do_xsts_authorization(
                &sisu_data.user_token.token,
                "http://gssv.xboxlive.com/",
                &private_jwk,
            )
            .await?;

        let gssv_token = gssv_token_resp
            .get("Token")
            .and_then(|value| value.as_str())
            .ok_or("Missing gssv token")?;

        let force_region_ip = self.config_provider.get_force_region_ip();
        let xhome_token = client
            .get_streaming_token(gssv_token, "xhome", &force_region_ip)
            .await?;

        let xcloud_token = match client
            .get_streaming_token(gssv_token, "xgpuweb", &force_region_ip)
            .await
        {
            Ok(token) => Some(token),
            Err(_) => client
                .get_streaming_token(gssv_token, "xgpuwebf2p", &force_region_ip)
                .await
                .ok(),
        };

        self.core_repository.set_sisu_token(sisu_data)?;
        self.token_repository
            .set_web_token(serde_json::json!({ "data": auth_token_resp }))?;
        self.token_repository.set_stream_tokens(serde_json::json!({
            "xHomeToken": xhome_token,
            "xCloudToken": xcloud_token
        }))?;

        let snapshot = self.token_repository.get_valid_session_snapshot()?;

        if let Ok(mut inner) = self.inner.lock() {
            if let Some(snap) = snapshot {
                inner.state.is_authenticated = true;
                inner.state.app_level = snap.app_level;
            } else {
                inner.state.is_authenticated = false;
                inner.state.app_level = 0;
            }
            inner.state.is_authenticating = false;
        } else {
            log::warn!("Failed to acquire auth service lock during complete_sisu_login");
        }

        Ok(())
    }
}

#[async_trait]
impl AuthProvider for AuthService {
    fn get_state(&self) -> AuthState {
        self.inner.lock().expect("Auth service lock poisoned").state.clone()
    }

    fn get_active_session(&self) -> AppResult<Option<AuthSessionReadyEvent>> {
        let snapshot = self
            .token_repository
            .get_valid_session_snapshot()
            .map_err(|e| AppError::Data(e))?;
        let Some(snapshot) = snapshot else {
            return Ok(None);
        };

        let inner = self.inner.lock().map_err(|e| {
            AppError::Internal(format!("Failed to acquire auth service lock: {}", e))
        })?;
        Ok(Some(AuthSessionReadyEvent {
            provider: inner.state.provider.clone(),
            app_level: snapshot.app_level,
            streaming_tokens: snapshot.streaming_tokens,
            web_token: snapshot.web_token,
        }))
    }

    fn get_streaming_token(&self, target_type: &str) -> AppResult<Option<Value>> {
        let tokens = self
            .token_repository
            .get_stream_tokens()
            .map_err(|e| AppError::Data(e))?;
        let Some(tokens) = tokens else {
            return Ok(None);
        };

        let token = if target_type == "home" {
            tokens.get("xHomeToken")
        } else {
            tokens.get("xCloudToken")
        };

        if self.token_repository.is_stream_token_valid(token) {
            return Ok(token.cloned());
        }

        Ok(None)
    }

    async fn login(&self) -> AppResult<LoginResponse> {
        self.login_impl().await
    }

    async fn get_transfer_token(&self) -> AppResult<String> {
        self.transfer_token_service
            .get_transfer_token()
            .await
            .map_err(|e| AppError::Data(e))
    }

    fn get_web_api_tokens(&self) -> AppResult<Option<(String, String)>> {
        let web_token = self
            .token_repository
            .get_web_token()
            .map_err(|e| AppError::Data(e))?;
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

    async fn check_authentication(&self) -> AppResult<CheckAuthResponse> {
        self.check_authentication_impl().await
    }

    async fn clear_auth_cache(&self, scope: &str) -> AppResult<()> {
        self.clear_auth_cache_impl(scope).await
    }

    async fn logout(&self) -> AppResult<()> {
        self.logout_impl().await
    }

    async fn handle_oauth_callback(&self, callback_url: &str) -> AppResult<()> {
        self.handle_oauth_callback_impl(callback_url).await
    }

    async fn cancel_pending_login(&self) {
        self.cancel_pending_login_impl().await;
    }

    async fn reset_runtime_after_store_purge(&self) {
        self.reset_runtime_after_store_purge_impl().await;
    }
}

fn parse_token_details<T>(source: &serde_json::Value, key: &str) -> Result<TokenDetails<T>, String>
where
    T: serde::de::DeserializeOwned,
{
    let token_value = source
        .get(key)
        .cloned()
        .ok_or_else(|| format!("missing field `{}`", key))?;
    serde_json::from_value(token_value).map_err(|error| error.to_string())
}
