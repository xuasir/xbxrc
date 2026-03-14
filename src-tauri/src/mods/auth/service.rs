use crate::error::{AppError, AppResult};
use crate::mods::auth::persistence_service::AuthPersistenceService;
use crate::mods::auth::runtime_state::{AuthRuntimeState, BeginCheckOutcome};
use crate::mods::auth::types::{
    AuthSessionReadyEvent, AuthState, CheckAuthResponse, LoginResponse,
};
use crate::mods::auth::{events, AuthProvider};
use crate::mods::config::ConfigProviderRef;
use async_trait::async_trait;
use serde_json::Value;
use tauri::{AppHandle, Manager};
use xbox_auth_flow::{
    AuthFlow, BuildDownstreamTokensInput, CompleteOAuthLoginInput, RefreshAndFinalizeInput,
    StartOAuthLoginInput, TransferTokenInput,
};

const REFRESH_SKIP_WINDOW_MS: u64 = 23 * 60 * 60 * 1000;
const CHECK_AUTH_COOLDOWN_MS: u64 = 15 * 1000;
pub const AUTH_WINDOW_LABEL: &str = "auth-oauth-window";

pub struct AuthService {
    app_handle: AppHandle,
    config_provider: ConfigProviderRef,
    persistence: AuthPersistenceService,
    runtime: AuthRuntimeState,
}

impl AuthService {
    pub fn new(app_handle: AppHandle, config_provider: ConfigProviderRef) -> Self {
        let persistence = AuthPersistenceService::new(app_handle.clone());
        let (is_authenticated, app_level) = persistence.initial_auth_status();
        let runtime = AuthRuntimeState::new("xal".to_string(), is_authenticated, app_level);

        Self {
            app_handle,
            config_provider,
            persistence,
            runtime,
        }
    }

    fn close_auth_window(&self) {
        if let Some(window) = self.app_handle.get_webview_window(AUTH_WINDOW_LABEL) {
            let _ = window.close();
        }
    }

    async fn login_impl(&self) -> AppResult<LoginResponse> {
        log::info!("Auth: 开始登录流程");
        self.runtime.begin_login()?;

        let result: AppResult<LoginResponse> = async {
            log::debug!("Auth: 调用 xbox-auth-flow 启动 OAuth 登录流程");
            let flow_output = AuthFlow::new()
                .start_oauth_login(StartOAuthLoginInput {
                    title_id: "000000004c20a908".to_string(),
                    device_version: "15.0".to_string(),
                })
                .await
                .map_err(|error| {
                    log::error!("Auth: 启动 OAuth 登录流程失败: {}", error);
                    AppError::Data(error.to_string())
                })?;

            self.persistence
                .save_private_jwk(flow_output.seed.private_jwk.clone())?;
            self.runtime
                .store_pending_redirect_flow(flow_output.pending.clone())?;

            Ok(LoginResponse {
                mode: "oauth-window".to_string(),
                url: flow_output.oauth_url,
                state: flow_output.oauth_state,
            })
        }
        .await;

        if result.is_err() {
            log::error!("Auth: 登录流程失败");
            self.runtime.fail_login()?;
        }

        self.sync_auth_state();

        result
    }

    async fn logout_impl(&self) -> AppResult<()> {
        log::info!("Auth: 开始注销流程");
        self.close_auth_window();
        self.persistence.clear_all_tokens()?;
        self.runtime.clear_auth_state()?;
        log::info!("Auth: 注销完成, 所有 Token 和状态已重置");

        self.sync_auth_state();
        Ok(())
    }

    async fn clear_auth_cache_impl(&self, scope: &str) -> AppResult<()> {
        log::info!("Auth: 开始清理认证缓存, scope={}", scope);
        self.close_auth_window();
        if scope == "all" {
            self.persistence.clear_all_tokens()?;
        } else {
            self.persistence.clear_ephemeral_tokens()?;
        }

        self.runtime.clear_auth_state()?;
        log::info!("Auth: 清理认证缓存完成");

        self.sync_auth_state();
        Ok(())
    }

    async fn reset_runtime_after_store_purge_impl(&self) {
        self.close_auth_window();
        self.runtime.reset_after_store_purge();
        self.sync_auth_state();
    }

    async fn check_authentication_impl(&self) -> AppResult<CheckAuthResponse> {
        log::info!("Auth: 开始检查认证状态");
        let now_ms = now_ms();
        let previous_state = match self.runtime.begin_check(now_ms, CHECK_AUTH_COOLDOWN_MS)? {
            BeginCheckOutcome::ShortCircuit(response) => return Ok(response),
            BeginCheckOutcome::Proceed { previous_state } => previous_state,
        };
        self.sync_auth_state();

        let result = self.check_authentication_internal(previous_state).await;
        self.sync_auth_state();
        result
    }

    async fn check_authentication_internal(
        &self,
        previous_state: AuthState,
    ) -> AppResult<CheckAuthResponse> {
        if let Some(snapshot) = self.persistence.get_valid_session_snapshot()? {
            log::info!("Auth: 发现有效的会话快照, 直接完成认证");
            return self.runtime.finish_check_from_snapshot(&snapshot);
        }

        let should_start_silent_flow = self.persistence.has_valid_auth_tokens()
            || self.persistence.get_user_token()?.is_some();

        if should_start_silent_flow {
            log::info!("Auth: 未发现有效会话快照, 但存在核心 Token, 尝试静默登录");
            match self.start_silent_flow_impl().await {
                Ok(()) => {
                    log::info!("Auth: 静默登录成功");
                    return self.runtime.finish_check_success(true);
                }
                Err(error) => {
                    if is_transient_auth_error(&error) {
                        log::warn!(
                            "Auth: 静默登录遇到可重试网络错误，保留现有会话并延后重试: {}",
                            error
                        );

                        let has_web_tokens = self.persistence.get_web_api_tokens()?.is_some();
                        let fallback_app_level = self.persistence.get_cached_app_level();

                        return self.runtime.finish_check_transient_failure(
                            &previous_state,
                            has_web_tokens,
                            fallback_app_level,
                        );
                    }

                    log::error!("Auth: 静默登录失败, 清理所有 Token: {}", error);
                    self.persistence.clear_all_tokens()?;
                }
            }
        }

        log::info!("Auth: 无有效 Token, 用户未登录");
        self.runtime.finish_check_unauthenticated()
    }

    async fn handle_oauth_callback_impl(&self, callback_url: &str) -> AppResult<()> {
        log::info!("Auth: 收到 OAuth 回调");
        let flow = match self.runtime.take_pending_redirect_flow()? {
            Some(flow) => {
                log::debug!("Auth: 找到待处理的重定向流程");
                flow
            }
            None => {
                self.runtime.mark_authenticating_idle()?;
                log::error!("Auth: 收到 OAuth 回调，但没有待处理的重定向流程");
                return Err(AppError::Data("No pending redirect flow".to_string()));
            }
        };
        let seed = self.persistence.load_auth_flow_seed()?;
        let force_region_ip = self.config_provider.get_force_region_ip();

        let result = AuthFlow::new()
            .complete_oauth_login(CompleteOAuthLoginInput {
                callback_url: callback_url.to_string(),
                pending: flow,
                seed,
                force_region_ip,
            })
            .await
            .map_err(|error| {
                log::error!("Auth: 完成 OAuth 登录流程失败: {}", error);
                AppError::Data(error.to_string())
            });

        match result {
            Ok(output) => {
                self.persistence.persist_auth_bundle(&output.auth_bundle)?;
                self.runtime
                    .mark_authenticated(output.auth_bundle.app_level)?;
                // 认证成功后由服务层兜底关闭 OAuth 窗口，避免仅依赖导航回调里的 best-effort close。
                self.close_auth_window();
                log::info!("Auth: 交互式登录完成，下游 token 已全部就绪");
                self.sync_auth_state();
                Ok(())
            }
            Err(error) => {
                self.runtime.mark_authenticating_idle()?;
                self.sync_auth_state();
                Err(error)
            }
        }
    }

    async fn cancel_pending_login_impl(&self) {
        self.runtime.cancel_pending_login();
        self.sync_auth_state();
    }

    fn mark_callback_processing_impl(&self) -> AppResult<()> {
        self.runtime.mark_callback_processing()?;
        self.sync_auth_state();
        Ok(())
    }

    fn unmark_callback_processing_impl(&self) -> AppResult<()> {
        self.runtime.unmark_callback_processing()?;
        self.sync_auth_state();
        Ok(())
    }

    async fn start_silent_flow_impl(&self) -> Result<(), String> {
        log::info!("Auth: 静默流程开始");
        let stream_tokens = self
            .persistence
            .get_stream_tokens()
            .map_err(|e| e.to_string())?;
        let web_token = self
            .persistence
            .get_web_token()
            .map_err(|e| e.to_string())?;
        let has_any_cached_token = stream_tokens
            .as_ref()
            .map(|tokens| tokens.get("xHomeToken").is_some() || tokens.get("xCloudToken").is_some())
            .unwrap_or(false)
            && web_token.is_some();

        let token_update_time = self.persistence.get_token_update_time();
        let now_ms = now_ms();
        let should_skip_core_refresh = has_any_cached_token
            && token_update_time > 0
            && now_ms.saturating_sub(token_update_time) < REFRESH_SKIP_WINDOW_MS;

        if !should_skip_core_refresh {
            log::info!("Auth: 核心 Token 需要刷新 (超过23小时或无缓存)");
            let user_token = self
                .persistence
                .get_user_token()
                .map_err(|error| error.to_string())?
                .ok_or("Missing user token for refresh")?;
            let seed = self
                .persistence
                .load_auth_flow_seed()
                .map_err(|error| error.to_string())?;
            let force_region_ip = self.config_provider.get_force_region_ip();

            log::info!("Auth: 通过 xbox-auth-flow 执行静默刷新与下游令牌收口");
            let output = AuthFlow::new()
                .refresh_and_finalize(RefreshAndFinalizeInput {
                    refresh_token: user_token.refresh_token,
                    seed,
                    force_region_ip,
                })
                .await
                .map_err(|error| {
                    log::error!("Auth: 静默刷新流程失败: {}", error);
                    error.to_string()
                })?;

            self.persistence
                .persist_auth_bundle(&output.auth_bundle)
                .map_err(|error| error.to_string())?;
            self.runtime
                .mark_authenticated(output.auth_bundle.app_level)
                .map_err(|error| error.to_string())?;
            log::info!("Auth: 静默刷新与下游令牌收口完成");
            return Ok(());
        }

        log::info!("Auth: 跳过核心 Token 刷新 (23小时窗口内)");
        let user_token = self
            .persistence
            .get_user_token()
            .map_err(|error| error.to_string())?
            .ok_or("Missing user token for downstream refresh")?;
        let sisu_token = self
            .persistence
            .load_flow_sisu_token()
            .map_err(|error| error.to_string())?;
        let seed = self
            .persistence
            .load_auth_flow_seed()
            .map_err(|error| error.to_string())?;
        let force_region_ip = self.config_provider.get_force_region_ip();

        log::info!("Auth: 通过 xbox-auth-flow 复用已有核心 token 获取下游令牌");
        let output = AuthFlow::new()
            .build_downstream_tokens(BuildDownstreamTokensInput {
                user_token: serde_json::from_value(serde_json::to_value(user_token).unwrap())
                    .map_err(|error| error.to_string())?,
                sisu_token,
                seed,
                force_region_ip,
            })
            .await
            .map_err(|error| {
                log::error!("Auth: 下游令牌收口失败: {}", error);
                error.to_string()
            })?;

        self.persistence
            .persist_auth_bundle(&output.auth_bundle)
            .map_err(|error| error.to_string())?;
        self.runtime
            .mark_authenticated(output.auth_bundle.app_level)
            .map_err(|error| error.to_string())?;
        log::info!("Auth: 复用已有核心 token 的下游令牌收口完成");
        Ok(())
    }

    fn sync_auth_state(&self) {
        let state = self.get_state();
        let _ = events::emit_auth_state_changed(&self.app_handle, &state);

        if state.is_authenticated {
            let _ = events::emit_session_ready(&self.app_handle, &state.provider, state.app_level);
        }
    }
}

#[async_trait]
impl AuthProvider for AuthService {
    fn get_state(&self) -> AuthState {
        let snapshot = self.persistence.get_valid_session_snapshot().ok().flatten();
        self.runtime.sync_state_from_snapshot(snapshot.as_ref())
    }

    fn get_active_session(&self) -> AppResult<Option<AuthSessionReadyEvent>> {
        let snapshot = self.persistence.get_valid_session_snapshot()?;
        let Some(snapshot) = snapshot else {
            return Ok(None);
        };

        Ok(Some(AuthSessionReadyEvent {
            provider: self.runtime.provider()?,
            app_level: snapshot.app_level,
            streaming_tokens: snapshot.streaming_tokens,
            web_token: snapshot.web_token,
        }))
    }

    fn get_streaming_token(&self, target_type: &str) -> AppResult<Option<Value>> {
        self.persistence.get_streaming_token(target_type)
    }

    async fn login(&self) -> AppResult<LoginResponse> {
        self.login_impl().await
    }

    async fn get_transfer_token(&self) -> AppResult<String> {
        let refresh_token = self.persistence.get_refresh_token()?.ok_or_else(|| {
            AppError::Data("Refresh token is missing. Please authenticate first.".to_string())
        })?;

        AuthFlow::new()
            .transfer_token(TransferTokenInput { refresh_token })
            .await
            .map(|output| output.transfer_token)
            .map_err(|error| AppError::Data(error.to_string()))
    }

    fn get_web_api_tokens(&self) -> AppResult<Option<(String, String)>> {
        self.persistence.get_web_api_tokens()
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
