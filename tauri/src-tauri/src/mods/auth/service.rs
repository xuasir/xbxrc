use crate::mods::auth::client::{XalRedirectFlow, XboxWebApiClient};
use crate::mods::auth::config_bridge::AuthConfigBridge;
use crate::mods::auth::repository::CoreTokenRepository;
use crate::mods::auth::token_repository::AuthTokenRepository;
use crate::mods::auth::transfer_token_service::AuthTransferTokenService;
use crate::mods::auth::types::{
    AuthClaims, AuthSessionReadyEvent, AuthState, SisuTokenData, TitleClaims, TokenDetails,
    UserClaims, UserTokenData,
};
use tauri::{AppHandle, Manager};

const REFRESH_SKIP_WINDOW_MS: u64 = 23 * 60 * 60 * 1000;
pub const AUTH_WINDOW_LABEL: &str = "auth-oauth-window";

pub struct AuthService {
    app_handle: AppHandle,
    config_bridge: AuthConfigBridge,
    core_repository: CoreTokenRepository,
    token_repository: AuthTokenRepository,
    transfer_token_service: AuthTransferTokenService,
    state: AuthState,
    pending_redirect_flow: Option<XalRedirectFlow>,
}

impl AuthService {
    pub fn new(app_handle: AppHandle) -> Self {
        let config_bridge = AuthConfigBridge::new(app_handle.clone());
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
            config_bridge,
            core_repository,
            token_repository,
            transfer_token_service,
            state: AuthState {
                provider,
                is_authenticating: false,
                is_authenticated,
                app_level,
            },
            pending_redirect_flow: None,
        }
    }

    pub fn get_state(&self) -> AuthState {
        self.state.clone()
    }

    pub fn get_active_session(&self) -> Result<Option<AuthSessionReadyEvent>, String> {
        let snapshot = self.token_repository.get_valid_session_snapshot()?;
        let Some(snapshot) = snapshot else {
            return Ok(None);
        };

        Ok(Some(AuthSessionReadyEvent {
            provider: self.state.provider.clone(),
            app_level: snapshot.app_level,
            streaming_tokens: snapshot.streaming_tokens,
            web_token: snapshot.web_token,
        }))
    }

    pub fn get_streaming_token(
        &self,
        target_type: &str,
    ) -> Result<Option<serde_json::Value>, String> {
        let tokens = self.token_repository.get_stream_tokens()?;
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

    pub async fn get_transfer_token(&self) -> Result<String, String> {
        self.transfer_token_service.get_transfer_token().await
    }

    pub fn get_web_api_tokens(&self) -> Result<Option<(String, String)>, String> {
        let web_token = self.token_repository.get_web_token()?;
        let Some(web_token) = web_token else {
            return Ok(None);
        };

        let data = web_token.get("data").unwrap_or(&web_token);
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

    pub fn logout(&mut self) -> Result<(), String> {
        self.close_auth_window();
        self.core_repository.clear_all_tokens()?;
        self.pending_redirect_flow = None;
        self.state.is_authenticated = false;
        self.state.is_authenticating = false;
        self.state.app_level = 0;
        Ok(())
    }

    pub fn clear_auth_cache(&mut self, scope: &str) -> Result<(), String> {
        self.close_auth_window();
        if scope == "all" {
            self.core_repository.clear_all_tokens()?;
        } else {
            self.token_repository.clear_ephemeral_tokens()?;
        }

        self.pending_redirect_flow = None;
        self.state.is_authenticated = false;
        self.state.is_authenticating = false;
        self.state.app_level = 0;
        Ok(())
    }

    pub fn reset_runtime_state(&mut self) {
        self.close_auth_window();
        self.pending_redirect_flow = None;
        self.state.is_authenticated = false;
        self.state.is_authenticating = false;
        self.state.app_level = 0;
    }

    pub fn reset_runtime_after_store_purge(&mut self) {
        self.close_auth_window();
        self.pending_redirect_flow = None;
        self.state.is_authenticated = false;
        self.state.is_authenticating = false;
        self.state.app_level = 0;
    }

    pub async fn check_authentication(&mut self) -> Result<serde_json::Value, String> {
        eprintln!("[auth][check] start");
        self.state.is_authenticating = true;

        // 优先使用已缓存的可用会话。
        if let Some(snapshot) = self.token_repository.get_valid_session_snapshot()? {
            eprintln!(
                "[auth][check] hit valid snapshot appLevel={}",
                snapshot.app_level
            );
            self.state.is_authenticating = false;
            self.state.is_authenticated = true;
            self.state.app_level = snapshot.app_level;
            return Ok(serde_json::json!({
                "provider": "xal",
                "startedSilentFlow": false
            }));
        }

        let should_start_silent_flow = self.core_repository.has_valid_auth_tokens()
            || self.core_repository.get_user_token()?.is_some();

        if should_start_silent_flow {
            eprintln!("[auth][check] start silent flow");
            if self.prepare_silent_flow().await.is_ok() && self.complete_sisu_login().await.is_ok()
            {
                eprintln!("[auth][check] silent flow success");
                self.state.is_authenticating = false;
                self.state.is_authenticated = true;
                return Ok(serde_json::json!({
                    "provider": "xal",
                    "startedSilentFlow": true
                }));
            }

            // 与 Electron XAL 对齐：silent 失败时清理核心与派生 token。
            eprintln!("[auth][check] silent flow failed, clear all tokens");
            self.core_repository.clear_all_tokens()?;
        }

        self.state.is_authenticating = false;
        self.state.is_authenticated = false;
        self.state.app_level = 0;
        eprintln!("[auth][check] end unauthenticated");

        Ok(serde_json::json!({
            "provider": self.state.provider,
            "startedSilentFlow": false
        }))
    }

    pub async fn login(&mut self) -> Result<serde_json::Value, String> {
        // 与 Electron XAL 语义对齐：login 永远走 oauth-window。
        eprintln!("[auth][login] start");
        self.state.is_authenticating = true;

        let result: Result<serde_json::Value, String> = async {
            let client = XboxWebApiClient::new();
            let jwt_keys = XboxWebApiClient::generate_ecdsa_keypair()?;
            let private_jwk = jwt_keys.private_jwk.as_ref().ok_or("Missing private JWK")?;
            self.core_repository
                .set_jwt_private_jwk(private_jwk.clone())?;

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
                .await?;
            let device_token = device_token_resp
                .get("Token")
                .and_then(|value| value.as_str())
                .ok_or("Invalid Device Token Response")?;

            let code_challenge = XboxWebApiClient::create_code_challenge();
            let state_str = XboxWebApiClient::get_random_state();

            let sisu_auth = client
                .do_sisu_authentication(device_token, &code_challenge, &state_str, private_jwk)
                .await?;

            self.pending_redirect_flow = Some(XalRedirectFlow {
                sisu_auth: sisu_auth.clone(),
                state: state_str.clone(),
                code_challenge,
            });
            eprintln!("[auth][login] prepared oauth redirect");

            Ok(serde_json::json!({
                "mode": "oauth-window",
                "url": sisu_auth["MsaOauthRedirect"].as_str().unwrap_or(""),
                "state": state_str
            }))
        }
        .await;

        if result.is_err() {
            self.pending_redirect_flow = None;
            self.state.is_authenticating = false;
            eprintln!("[auth][login] failed");
        }

        result
    }

    pub async fn handle_oauth_callback(&mut self, callback_url: &str) -> Result<(), String> {
        eprintln!("[auth][callback] receive url={}", callback_url);
        let flow = self.pending_redirect_flow.take().ok_or_else(|| {
            self.state.is_authenticating = false;
            "No pending redirect flow".to_string()
        })?;

        let url = url::Url::parse(callback_url).map_err(|e| {
            self.state.is_authenticating = false;
            e.to_string()
        })?;
        if url.query_pairs().any(|(key, _)| key == "error") {
            self.state.is_authenticating = false;
            eprintln!("[auth][callback] contains oauth error");
            return Err("OAuth callback contains error".to_string());
        }
        let code = url
            .query_pairs()
            .find(|(key, _)| key == "code")
            .map(|(_, value)| value.to_string())
            .ok_or_else(|| {
                self.state.is_authenticating = false;
                "Missing code in callback".to_string()
            })?;
        let state = url
            .query_pairs()
            .find(|(key, _)| key == "state")
            .map(|(_, value)| value.to_string())
            .ok_or_else(|| {
                self.state.is_authenticating = false;
                "Missing state in callback".to_string()
            })?;

        if state != flow.state {
            self.state.is_authenticating = false;
            eprintln!("[auth][callback] state mismatch");
            return Err("State mismatch in OAuth callback".to_string());
        }

        let client = XboxWebApiClient::new();
        let user_token_val = client
            .exchange_code_for_token(&code, &flow.code_challenge.verifier)
            .await?;

        let user_token: UserTokenData =
            serde_json::from_value(user_token_val).map_err(|e| e.to_string())?;

        self.core_repository.set_user_token(user_token)?;
        eprintln!("[auth][callback] exchange code success, start complete_sisu_login");
        match self.complete_sisu_login().await {
            Ok(()) => {
                eprintln!(
                    "[auth][callback] success authenticated={} appLevel={}",
                    self.state.is_authenticated, self.state.app_level
                );
                Ok(())
            }
            Err(error) => {
                self.state.is_authenticating = false;
                eprintln!("[auth][callback] failed err={}", error);
                Err(error)
            }
        }
    }

    pub fn cancel_pending_login(&mut self) {
        if self.pending_redirect_flow.is_some() || self.state.is_authenticating {
            self.pending_redirect_flow = None;
            self.state.is_authenticating = false;
            self.state.is_authenticated = false;
            self.state.app_level = 0;
        }
    }

    fn close_auth_window(&self) {
        if let Some(window) = self.app_handle.get_webview_window(AUTH_WINDOW_LABEL) {
            let _ = window.close();
        }
    }

    async fn prepare_silent_flow(&mut self) -> Result<(), String> {
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
        eprintln!(
            "[auth][silent] hasCached={} tokenUpdateTime={} shouldSkipRefresh={}",
            has_any_cached_token, token_update_time, should_skip_refresh
        );

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
        eprintln!("[auth][silent] refresh user token success");
        self.core_repository.set_user_token(parsed_user_token)
    }

    async fn complete_sisu_login(&mut self) -> Result<(), String> {
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
            // sisu authorize 响应可能不返回 DeviceToken，这里与 Electron 一致使用当前请求使用的 device token 回填。
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

        let force_region_ip = self.config_bridge.get_force_region_ip();
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

        let web_token = serde_json::json!({
            "data": auth_token_resp
        });
        self.token_repository.set_web_token(web_token.clone())?;

        let streaming_tokens = serde_json::json!({
            "xHomeToken": xhome_token,
            "xCloudToken": xcloud_token
        });
        self.token_repository.set_stream_tokens(streaming_tokens)?;

        if let Some(snapshot) = self.token_repository.get_valid_session_snapshot()? {
            self.state.is_authenticated = true;
            self.state.app_level = snapshot.app_level;
        } else {
            self.state.is_authenticated = false;
            self.state.app_level = 0;
        }

        self.state.is_authenticating = false;
        eprintln!(
            "[auth][session] persist done authenticated={} appLevel={}",
            self.state.is_authenticated, self.state.app_level
        );
        Ok(())
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
