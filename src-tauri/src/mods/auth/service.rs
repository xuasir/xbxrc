use crate::error::{AppError, AppResult};
use crate::mods::auth::repository::CoreTokenRepository;
use crate::mods::auth::token_repository::AuthTokenRepository;
use crate::mods::auth::transfer_token_service::AuthTransferTokenService;
use crate::mods::auth::types::{
    AuthSessionReadyEvent, AuthState, CheckAuthResponse, LoginResponse, SisuTokenData,
    TokenDetails, UserTokenData,
};
use crate::mods::auth::AuthProvider;
use crate::mods::config::ConfigProviderRef;
use async_trait::async_trait;
use serde_json::Value;
use tauri::{AppHandle, Manager};
use xbox_webapi::{AuthApi, SisuAuthorizeResponse, StreamingTokenResponse, XalRedirectFlow};

const REFRESH_SKIP_WINDOW_MS: u64 = 23 * 60 * 60 * 1000;
const CHECK_AUTH_COOLDOWN_MS: u64 = 15 * 1000;
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
    is_processing_callback: bool,
    last_check_at_ms: u64,
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
                is_processing_callback: false,
                last_check_at_ms: 0,
            }),
        }
    }

    fn close_auth_window(&self) {
        if let Some(window) = self.app_handle.get_webview_window(AUTH_WINDOW_LABEL) {
            let _ = window.close();
        }
    }

    async fn login_impl(&self) -> AppResult<LoginResponse> {
        log::info!("Auth: 开始登录流程");
        {
            let mut inner = self.inner.lock().map_err(|e| {
                AppError::Internal(format!("Failed to acquire auth service lock: {}", e))
            })?;
            inner.state.is_authenticating = true;
        }

        let result: AppResult<LoginResponse> = async {
            let client: AuthApi = AuthApi::new();

            log::debug!("Auth: 生成 ECDSA 密钥对");
            let jwt_keys = xbox_webapi::generate_ecdsa_keypair().map_err(|e| AppError::Data(e))?;
            let private_jwk = jwt_keys
                .private_jwk
                .as_ref()
                .ok_or_else(|| AppError::Data("Missing private JWK".to_string()))?;
            self.core_repository
                .set_jwt_private_jwk(private_jwk.clone())
                .map_err(|e| AppError::Data(e))?;

            let device_uuid = uuid::Uuid::new_v4().to_string();
            let serial_number = uuid::Uuid::new_v4().to_string();
            log::info!("Auth: 正在获取 Device token");
            let device_token_resp = client
                .get_device_token(
                    "000000004c20a908",
                    &device_uuid,
                    &serial_number,
                    "15.0",
                    &private_jwk,
                )
                .await
                .map_err(|e| {
                    log::error!("Auth: 获取 Device token 失败: {}", e);
                    AppError::Data(e.to_string())
                })?;
            log::info!("Auth: 获取 Device token 成功");
            let device_token = device_token_resp.Token;

            let code_challenge = xbox_webapi::create_code_challenge();
            let state_str = xbox_webapi::get_random_state();

            log::info!("Auth: 正在进行 SISU 认证");
            let sisu_auth = client
                .sisu_authenticate(
                    &device_token,
                    &code_challenge.value,
                    &code_challenge.method,
                    &state_str,
                    &private_jwk,
                )
                .await
                .map_err(|e| {
                    log::error!("Auth: SISU 认证失败: {}", e);
                    AppError::Data(e.to_string())
                })?;
            log::info!("Auth: SISU 认证成功, MsaOAuthRedirect URL 已生成");

            {
                let mut inner = self.inner.lock().map_err(|e| {
                    AppError::Internal(format!("Failed to acquire auth service lock: {}", e))
                })?;
                inner.pending_redirect_flow = Some(XalRedirectFlow {
                    sisu_auth: serde_json::to_value(&sisu_auth).unwrap(),
                    state: state_str.clone(),
                    code_challenge,
                });
            }

            Ok(LoginResponse {
                mode: "oauth-window".to_string(),
                url: sisu_auth.msa_oauth_redirect,
                state: state_str,
            })
        }
        .await;

        if result.is_err() {
            log::error!("Auth: 登录流程失败");
            let mut inner = self.inner.lock().map_err(|e| {
                AppError::Internal(format!("Failed to acquire auth service lock: {}", e))
            })?;
            inner.pending_redirect_flow = None;
            inner.state.is_authenticating = false;
        }

        result
    }

    async fn logout_impl(&self) -> AppResult<()> {
        log::info!("Auth: 开始注销流程");
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
        log::info!("Auth: 注销完成, 所有 Token 和状态已重置");
        Ok(())
    }

    async fn clear_auth_cache_impl(&self, scope: &str) -> AppResult<()> {
        log::info!("Auth: 开始清理认证缓存, scope={}", scope);
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
        log::info!("Auth: 清理认证缓存完成");
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
            log::warn!(
                "Failed to acquire auth service lock during reset_runtime_after_store_purge"
            );
        }
    }

    async fn check_authentication_impl(&self) -> AppResult<CheckAuthResponse> {
        log::info!("Auth: 开始检查认证状态");
        let now_ms = now_ms();
        let previous_state = {
            let mut inner = self.inner.lock().map_err(|e| {
                AppError::Internal(format!("Failed to acquire auth service lock: {}", e))
            })?;

            // 避免并发重复触发 check，防止同一时刻出现多条静默刷新链路。
            if inner.state.is_authenticating {
                return Ok(CheckAuthResponse {
                    provider: inner.state.provider.clone(),
                    started_silent_flow: false,
                });
            }

            // 已认证状态下做冷却，避免短时间重复触发网络刷新。
            if inner.state.is_authenticated
                && now_ms.saturating_sub(inner.last_check_at_ms) < CHECK_AUTH_COOLDOWN_MS
            {
                return Ok(CheckAuthResponse {
                    provider: inner.state.provider.clone(),
                    started_silent_flow: false,
                });
            }

            inner.last_check_at_ms = now_ms;
            let previous_state = inner.state.clone();
            inner.state.is_authenticating = true;
            previous_state
        };

        // 优先使用缓存的 stream/web token
        if let Some(snapshot) = self
            .token_repository
            .get_valid_session_snapshot()
            .map_err(|e| AppError::Data(e))?
        {
            log::info!("Auth: 发现有效的会话快照, 直接完成认证");
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
            log::info!("Auth: 未发现有效会话快照, 但存在核心 Token, 尝试静默登录");
            match self.start_silent_flow_impl().await {
                Ok(()) => {
                    log::info!("Auth: 静默登录成功");
                    // 成功后状态已在内部更新
                    return Ok(CheckAuthResponse {
                        provider: self.inner.lock().unwrap().state.provider.clone(),
                        started_silent_flow: true,
                    });
                }
                Err(error) => {
                    if is_transient_auth_error(&error) {
                        log::warn!(
                            "Auth: 静默登录遇到可重试网络错误，保留现有会话并延后重试: {}",
                            error
                        );

                        let has_web_tokens = self.get_web_api_tokens()?.is_some();
                        let fallback_app_level =
                            self.token_repository.get_cached_app_level().unwrap_or(0);

                        let mut inner = self.inner.lock().map_err(|e| {
                            AppError::Internal(format!(
                                "Failed to acquire auth service lock: {}",
                                e
                            ))
                        })?;
                        inner.state.is_authenticating = false;
                        inner.state.is_authenticated = previous_state.is_authenticated
                            || has_web_tokens
                            || fallback_app_level > 0;
                        inner.state.app_level = if fallback_app_level > 0 {
                            fallback_app_level
                        } else if previous_state.app_level > 0 {
                            previous_state.app_level
                        } else if has_web_tokens {
                            1
                        } else {
                            0
                        };

                        return Ok(CheckAuthResponse {
                            provider: inner.state.provider.clone(),
                            started_silent_flow: false,
                        });
                    }

                    log::error!("Auth: 静默登录失败, 清理所有 Token: {}", error);
                    // 非可重试错误按策略清理主令牌与派生令牌，强制重新登录。
                    self.core_repository
                        .clear_all_tokens()
                        .map_err(|e| AppError::Data(e))?;
                }
            }
        }

        log::info!("Auth: 无有效 Token, 用户未登录");
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
        log::info!("Auth: 收到 OAuth 回调");
        let flow = {
            let mut inner = self.inner.lock().map_err(|e| {
                AppError::Internal(format!("Failed to acquire auth service lock: {}", e))
            })?;
            match inner.pending_redirect_flow.take() {
                Some(flow) => {
                    log::debug!("Auth: 找到待处理的重定向流程");
                    flow
                }
                None => {
                    inner.state.is_authenticating = false;
                    log::error!("Auth: 收到 OAuth 回调，但没有待处理的重定向流程");
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
            log::error!("Auth: OAuth 回调中包含错误");
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
            log::error!("Auth: OAuth 回调 state 不匹配");
            return Err(AppError::Data(
                "State mismatch in OAuth callback".to_string(),
            ));
        }
        log::debug!("Auth: OAuth 回调 state 匹配成功");

        let client = AuthApi::new();
        log::info!("Auth: 正在用授权码交换 User token");
        let user_token_val = client
            .exchange_code_for_token(&code, &flow.code_challenge.verifier)
            .await
            .map_err(|e| {
                log::error!("Auth: 交换 User token 失败: {}", e);
                AppError::Data(e.to_string())
            })?;
        log::info!("Auth: 交换 User token 成功");

        let user_token: UserTokenData =
            serde_json::from_value(serde_json::to_value(user_token_val).unwrap()).unwrap();
        self.core_repository
            .set_user_token(user_token)
            .map_err(|e| AppError::Data(e))?;

        // 交互式登录成功后，直接走完整的 silent flow 来获取所有 token
        log::info!("Auth: User token 已存储, 开始静默流程获取所有下游 token");
        match self.start_silent_flow_impl().await {
            Ok(()) => {
                log::info!("Auth: 交互式登录的静默流程部分成功完成");
                Ok(())
            }
            Err(error) => {
                let mut inner = self.inner.lock().map_err(|e| {
                    AppError::Internal(format!("Failed to acquire auth service lock: {}", e))
                })?;
                inner.state.is_authenticating = false;
                log::error!("Auth: 交互式登录的静默流程部分失败: {}", error);
                Err(AppError::Data(error))
            }
        }
    }

    async fn cancel_pending_login_impl(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            // 如果正在处理回调，不要清空 pending_redirect_flow
            if inner.is_processing_callback {
                return;
            }

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

    fn mark_callback_processing_impl(&self) -> AppResult<()> {
        let mut inner = self.inner.lock().map_err(|e| {
            AppError::Internal(format!("Failed to acquire auth service lock: {}", e))
        })?;
        inner.is_processing_callback = true;
        Ok(())
    }

    fn unmark_callback_processing_impl(&self) -> AppResult<()> {
        let mut inner = self.inner.lock().map_err(|e| {
            AppError::Internal(format!("Failed to acquire auth service lock: {}", e))
        })?;
        inner.is_processing_callback = false;
        Ok(())
    }

    async fn start_silent_flow_impl(&self) -> Result<(), String> {
        log::info!("Auth: 静默流程开始");
        // 检查是否应该跳过核心 token 刷新 (23小时窗口)
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

        let should_skip_core_refresh = has_any_cached_token
            && token_update_time > 0
            && now_ms.saturating_sub(token_update_time) < REFRESH_SKIP_WINDOW_MS;

        // 如果需要，刷新核心 token (user_token, sisu_token)
        if !should_skip_core_refresh {
            log::info!("Auth: 核心 Token 需要刷新 (超过23小时或无缓存)");
            // 刷新 UserToken
            let user_token = self
                .core_repository
                .get_user_token()?
                .ok_or("Missing user token for refresh")?;
            log::info!("Auth: 正在刷新 User token");
            let refreshed_user_token = AuthApi::new()
                .refresh_user_token(&user_token.refresh_token)
                .await
                .map_err(|e| {
                    log::error!("Auth: 刷新 User token 失败: {}", e);
                    e.to_string()
                })?;
            log::info!("Auth: 刷新 User token 成功");
            let parsed_user_token: UserTokenData =
                serde_json::from_value(serde_json::to_value(refreshed_user_token).unwrap())
                    .map_err(|e| e.to_string())?;
            self.core_repository.set_user_token(parsed_user_token)?;

            // 刷新 SisuToken
            self.refresh_sisu_token().await?;
        } else {
            log::info!("Auth: 跳过核心 Token 刷新 (23小时窗口内)");
        }

        // 获取下游的 XSTS 和 Stream 令牌
        log::info!("Auth: 正在获取下游的 XSTS 和 Stream 令牌");
        self.get_downstream_tokens().await
    }

    async fn refresh_sisu_token(&self) -> Result<(), String> {
        log::info!("Auth: 正在刷新 Sisu token");
        let client = AuthApi::new();
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
                "000000004c20a908",
                &device_uuid,
                &serial_number,
                "15.0",
                &private_jwk,
            )
            .await
            .map_err(|e| {
                log::error!("Auth: [Sisu刷新流程] 获取 Device token 失败: {}", e);
                e.to_string()
            })?;
        let device_token = device_token_resp.Token;

        let sisu_auth_res: SisuAuthorizeResponse = client
            .sisu_authorize(&user_token.access_token, &device_token, &private_jwk)
            .await
            .map_err(|e| {
                log::error!("Auth: [Sisu刷新流程] Sisu Authorize 失败: {}", e);
                e.to_string()
            })?;

        let title_token = sisu_auth_res
            .title_token
            .ok_or("Sisu response missing TitleToken")?;
        let user_token = sisu_auth_res
            .user_token
            .ok_or("Sisu response missing UserToken")?;
        let authorization_token = sisu_auth_res
            .authorization_token
            .ok_or("Sisu response missing AuthorizationToken")?;
        // 某些账号下接口会返回 DeviceToken=null，这里回填本次请求使用的 device token。
        let resolved_device_token = sisu_auth_res.device_token.unwrap_or_else(|| {
            log::warn!("Auth: [Sisu刷新流程] Sisu 返回 DeviceToken 为空，回填本地 device token");
            device_token.clone()
        });

        let sisu_data = SisuTokenData {
            device_token: resolved_device_token,
            title_token: convert_sisu_token_details(title_token).map_err(|e| {
                log::error!("Auth: [Sisu刷新流程] 解析 TitleToken 失败: {}", e);
                e
            })?,
            user_token: convert_sisu_token_details(user_token).map_err(|e| {
                log::error!("Auth: [Sisu刷新流程] 解析 UserToken 失败: {}", e);
                e
            })?,
            authorization_token: convert_sisu_token_details(authorization_token).map_err(|e| {
                log::error!("Auth: [Sisu刷新流程] 解析 AuthorizationToken 失败: {}", e);
                e
            })?,
        };

        self.core_repository.set_sisu_token(sisu_data)?;
        log::info!("Auth: 刷新 Sisu token 成功");
        Ok(())
    }

    async fn get_downstream_tokens(&self) -> Result<(), String> {
        log::info!("Auth: 开始获取下游令牌 (XSTS, Stream)");
        let client = AuthApi::new();
        let private_jwk = self
            .core_repository
            .get_jwt_private_jwk()?
            .ok_or("Missing JWT private key")?;
        let sisu_token = self
            .core_repository
            .get_sisu_token()?
            .ok_or("Missing Sisu token")?;

        let user_token_str = sisu_token
            .user_token
            .token
            .as_ref()
            .ok_or("Sisu response missing user token string")?;

        log::info!("Auth: 正在获取 Web API (xboxlive.com) 的 XSTS token");
        let auth_token_resp = client
            .xsts_authorize(user_token_str, "http://xboxlive.com", &private_jwk)
            .await
            .map_err(|e| {
                log::error!("Auth: 获取 Web API XSTS token 失败: {}", e);
                e.to_string()
            })?;
        log::info!("Auth: 获取 Web API XSTS token 成功");

        log::info!("Auth: 正在获取 GSSV (gssv.xboxlive.com) 的 XSTS token");
        let gssv_token_resp = client
            .xsts_authorize(user_token_str, "http://gssv.xboxlive.com/", &private_jwk)
            .await
            .map_err(|e| {
                log::error!("Auth: 获取 GSSV XSTS token 失败: {}", e);
                e.to_string()
            })?;
        let gssv_token = gssv_token_resp.Token;
        log::info!("Auth: 获取 GSSV XSTS token 成功");

        let force_region_ip = self.config_provider.get_force_region_ip();
        log::info!("Auth: 正在获取 xHome streaming token");
        let xhome_token: StreamingTokenResponse = client
            .get_streaming_token(&gssv_token, "xhome", &force_region_ip)
            .await
            .map_err(|e| {
                log::error!("Auth: 获取 xHome streaming token 失败: {}", e);
                e.to_string()
            })?;
        log::info!("Auth: 获取 xHome streaming token 成功");

        log::info!("Auth: 正在获取 xCloud streaming token");
        let xcloud_token = match client
            .get_streaming_token(&gssv_token, "xgpuweb", &force_region_ip)
            .await
        {
            Ok(token) => {
                log::info!("Auth: 获取 xCloud streaming token (xgpuweb) 成功");
                Some(token)
            }
            Err(_) => {
                log::warn!("Auth: 获取 xgpuweb token 失败, 尝试 xgpuwebf2p");
                client
                    .get_streaming_token(&gssv_token, "xgpuwebf2p", &force_region_ip)
                    .await
                    .ok()
            }
        };
        let has_xcloud_token = xcloud_token.is_some();

        let now = chrono::Utc::now().timestamp_millis();

        self.token_repository
            .set_web_token(serde_json::json!({ "data": auth_token_resp }))?;
        let mut stream_tokens_map = serde_json::Map::new();
        stream_tokens_map.insert(
            "xHomeToken".to_string(),
            serde_json::json!({
                "_objectCreateTime": now,
                "data": xhome_token.data
            }),
        );
        if let Some(xcloud_token_val) = xcloud_token {
            stream_tokens_map.insert(
                "xCloudToken".to_string(),
                serde_json::json!({
                    "_objectCreateTime": now,
                    "data": xcloud_token_val.data
                }),
            );
        }
        self.token_repository
            .set_stream_tokens(Value::Object(stream_tokens_map))?;
        log::debug!("Auth: Web token 和 Stream tokens 已存入仓库");

        let snapshot = self.token_repository.get_valid_session_snapshot()?;
        if let Ok(mut inner) = self.inner.lock() {
            let app_level = if snapshot.as_ref().map(|s| s.app_level).unwrap_or(0) > 0 {
                snapshot.as_ref().map(|s| s.app_level).unwrap_or(1)
            } else if has_xcloud_token {
                2
            } else {
                1
            };
            inner.state.is_authenticated = true;
            inner.state.app_level = app_level;
            inner.state.is_authenticating = false;
            log::info!(
                "Auth: 认证状态更新: is_authenticated={}, app_level={}",
                inner.state.is_authenticated,
                inner.state.app_level
            );
        } else {
            log::warn!("Auth: 未能获取锁来更新最终认证状态");
        }

        Ok(())
    }
}

#[async_trait]
impl AuthProvider for AuthService {
    fn get_state(&self) -> AuthState {
        // 每次读取状态都用 token 快照做一次兜底同步，避免迁移期间状态丢失。
        let snapshot = self
            .token_repository
            .get_valid_session_snapshot()
            .ok()
            .flatten();
        let mut inner = self.inner.lock().expect("Auth service lock poisoned");

        if !inner.state.is_authenticating {
            if let Some(snapshot) = snapshot {
                inner.state.is_authenticated = true;
                inner.state.app_level = snapshot.app_level;
            }
        }

        let state = inner.state.clone();
        state
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

    fn mark_callback_processing(&self) -> AppResult<()> {
        self.mark_callback_processing_impl()
    }

    fn unmark_callback_processing(&self) -> AppResult<()> {
        self.unmark_callback_processing_impl()
    }
}

fn convert_sisu_token_details<T>(
    source: xbox_webapi::TokenDetails<T>,
) -> Result<TokenDetails<serde_json::Value>, String>
where
    T: serde::Serialize,
{
    Ok(TokenDetails {
        issue_instant: Some(source.issue_instant),
        not_after: Some(source.not_after),
        token: Some(source.Token),
        display_claims: serde_json::to_value(source.display_claims)
            .map_err(|error| error.to_string())?,
    })
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn is_transient_auth_error(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("network error")
        || lower.contains("timeout")
        || lower.contains("temporarily unavailable")
        || lower.contains("http 408")
        || lower.contains("http 429")
        || lower.contains("http 500")
        || lower.contains("http 502")
        || lower.contains("http 503")
        || lower.contains("http 504")
}
