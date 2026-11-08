use crate::error::{AppError, AppResult};
use crate::mods::auth::persistence_service::AuthPersistenceService;
use crate::mods::auth::runtime_state::{AuthRuntimeState, BeginCheckOutcome};
use crate::mods::auth::trace::{
    app_error_summary, flow_error_summary, record_auth_event, state_summary, token_summary,
    AuthOperationTrace,
};
use crate::mods::auth::types::{
    AuthSessionReadyEvent, AuthState, CheckAuthResponse, LoginResponse,
};
use crate::mods::auth::{events, AuthProvider};
use crate::mods::config::ConfigProviderRef;
use crate::mods::runtime_trace::RuntimeTraceRecorderRef;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};
use xbox_auth_flow::{
    AuthFlow, BuildDownstreamTokensInput, CompleteOAuthLoginInput, RefreshAndFinalizeInput,
    StartOAuthLoginInput, TransferTokenInput,
};

const REFRESH_SKIP_WINDOW_MS: u64 = 23 * 60 * 60 * 1000;
const CHECK_AUTH_COOLDOWN: Duration = Duration::from_secs(15);
pub const AUTH_WINDOW_LABEL: &str = "auth-oauth-window";

pub struct AuthService {
    app_handle: AppHandle,
    config_provider: ConfigProviderRef,
    persistence: AuthPersistenceService,
    runtime: AuthRuntimeState,
    runtime_trace: RuntimeTraceRecorderRef,
}

impl AuthService {
    pub fn new(
        app_handle: AppHandle,
        config_provider: ConfigProviderRef,
        runtime_trace: RuntimeTraceRecorderRef,
    ) -> Self {
        let persistence = AuthPersistenceService::new(app_handle.clone());
        let (is_authenticated, app_level) = persistence.initial_auth_status();
        let runtime = AuthRuntimeState::new("xal".to_string(), is_authenticated, app_level);
        record_auth_event(
            &runtime_trace,
            "authInitialized",
            state_summary(&runtime.current_state()),
        );

        Self {
            app_handle,
            config_provider,
            persistence,
            runtime,
            runtime_trace,
        }
    }

    fn close_auth_window(&self) {
        if let Some(window) = self.app_handle.get_webview_window(AUTH_WINDOW_LABEL) {
            let _ = window.close();
        }
    }

    async fn login_impl(&self) -> AppResult<LoginResponse> {
        let _gate = self.runtime.lock_check().await;
        log::info!("Auth: 开始登录流程");
        self.runtime.begin_login()?;
        let generation = self.runtime.generation()?;

        let result: AppResult<LoginResponse> = async {
            log::debug!("Auth: 调用 xbox-auth-flow 启动 OAuth 登录流程");
            let flow_output = AuthFlow::new()
                .start_oauth_login(StartOAuthLoginInput {
                    title_id: "000000004c20a908".to_string(),
                    device_version: "15.0".to_string(),
                })
                .await
                .map_err(|error| {
                    log::error!(
                        "Auth: 启动 OAuth 登录流程失败: {}",
                        flow_error_summary(&error)
                    );
                    auth_flow_app_error(error)
                })?;

            self.runtime
                .store_login_seed(generation, flow_output.pending.clone(), || {
                    self.persistence
                        .save_private_jwk(flow_output.seed.private_jwk.clone())
                })?;

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
        self.runtime
            .clear_credentials(|| self.persistence.clear_all_tokens())?;
        log::info!("Auth: 注销完成, 所有 Token 和状态已重置");

        self.sync_auth_state();
        Ok(())
    }

    async fn clear_auth_cache_impl(&self, scope: &str) -> AppResult<()> {
        log::info!("Auth: 开始清理认证缓存, scope={}", scope);
        self.close_auth_window();
        self.runtime.clear_credentials(|| {
            if scope == "all" {
                self.persistence.clear_all_tokens()
            } else {
                self.persistence.clear_ephemeral_tokens()
            }
        })?;
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
        self.check_authentication_with_rejection(None).await
    }

    async fn check_authentication_with_rejection(
        &self,
        rejected_token: Option<&str>,
    ) -> AppResult<CheckAuthResponse> {
        log::info!("Auth: 开始检查认证状态");
        let trace = AuthOperationTrace::new(&self.runtime_trace, "checkAuthentication");
        let _gate = self.runtime.lock_check().await;
        let result = async {
            if let Some(rejected) = rejected_token {
                // 等待同一轮恢复完成后，新 Token 可直接供其他 401 请求重试。
                if self.persistence.get_web_api_tokens()?.is_some_and(|(_, token)| {
                    token != rejected && !self.runtime.is_web_token_rejected(&token)
                }) {
                    return self.runtime.finish_check_success(false);
                }
            }
            let cached_token = self.persistence.get_web_token()?;
            let force_core_refresh = core_refresh_required(rejected_token, cached_token.as_ref(), &self.runtime);
            let has_valid_credentials = !force_core_refresh && self.get_web_api_tokens()?.is_some();
            let previous_state = match self.runtime.begin_check(
                Instant::now(), CHECK_AUTH_COOLDOWN, has_valid_credentials,
            )? {
                BeginCheckOutcome::ShortCircuit(response) => {
                    let state = self.runtime.current_state();
                    trace.stage("checkSkipped", json!({ "reason": if state.is_authenticating { "inFlight" } else { "cooldown" }, "state": state_summary(&state) }));
                    if force_core_refresh && state.is_authenticating {
                        return Err(AppError::Auth("登录正在处理中，请完成登录后刷新".into()));
                    }
                    return Ok(response);
                }
                BeginCheckOutcome::Proceed { previous_state } => previous_state,
            };
            if let Some(token) = rejected_token {
                self.runtime.reject_web_token(token)?;
            }
            let lease = self.runtime.check_lease()?;
            self.sync_auth_state_with_operation(Some(&trace));
            let result = self.check_authentication_internal(
                previous_state, &trace, lease.generation, force_core_refresh,
            ).await;
            drop(lease);
            self.sync_auth_state_with_operation(Some(&trace));
            result
        }.await;
        trace.finish(&result, &self.runtime.current_state());
        result
    }

    async fn check_authentication_internal(
        &self,
        previous_state: AuthState,
        trace: &AuthOperationTrace,
        generation: u64,
        force_core_refresh: bool,
    ) -> AppResult<CheckAuthResponse> {
        trace.stage("sessionSnapshotReadStarted", json!({}));
        if let Some(snapshot) = self
            .accepted_session_snapshot()?
            .filter(|_| !force_core_refresh)
        {
            trace.stage(
                "sessionSnapshotAccepted",
                json!({ "appLevel": snapshot.app_level }),
            );
            log::info!("Auth: 发现有效的会话快照, 直接完成认证");
            return self.runtime.finish_check_from_snapshot(&snapshot);
        }

        let should_start_silent_flow = self.persistence.has_valid_auth_tokens()
            || self.persistence.get_user_token()?.is_some();
        trace.stage(
            "silentFlowPolicy",
            json!({ "hasValidSessionSnapshot": false, "eligible": should_start_silent_flow }),
        );

        if should_start_silent_flow {
            log::info!("Auth: 未发现有效会话快照, 但存在核心 Token, 尝试静默登录");
            let flow_result = self
                .start_silent_flow_impl(trace, generation, force_core_refresh)
                .await;
            self.runtime.ensure_generation(generation)?;
            match flow_result {
                Ok(()) => {
                    log::info!("Auth: 静默登录成功");
                    return self.runtime.finish_check_success(true);
                }
                Err(error) => {
                    if !requires_interactive_login(&error) {
                        trace.stage("transientSessionFallbackStarted", json!({}));
                        trace.stage("sessionRefreshFailed", app_error_summary(&error));
                        log::warn!(
                            "Auth: 静默登录失败，保留凭据并延后重试: {}",
                            app_error_summary(&error)
                        );

                        let has_web_tokens = self.get_web_api_tokens()?.is_some();
                        let fallback_app_level = self
                            .accepted_session_snapshot()?
                            .map(|snapshot| snapshot.app_level)
                            .unwrap_or(0);

                        let response = self.runtime.finish_check_transient_failure(
                            &previous_state,
                            has_web_tokens,
                            fallback_app_level,
                        )?;
                        return if has_web_tokens {
                            Ok(response)
                        } else {
                            Err(error)
                        };
                    }

                    log::error!(
                        "Auth: 登录凭据已被拒绝, 清理 Token: {}",
                        app_error_summary(&error)
                    );
                    trace.stage("credentialsClearStarted", json!({}));
                    self.runtime
                        .clear_credentials(|| self.persistence.clear_all_tokens())?;
                    trace.stage("credentialsCleared", json!({}));
                }
            }
        }

        log::info!("Auth: 无有效 Token, 用户未登录");
        trace.stage("unauthenticated", json!({}));
        self.runtime.finish_check_unauthenticated()
    }

    async fn handle_oauth_callback_impl(&self, callback_url: &str) -> AppResult<()> {
        let _gate = self.runtime.lock_check().await;
        let lease = self.runtime.check_lease()?;
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
                include_streaming_tokens: true,
            })
            .await
            .map_err(|error| {
                log::error!(
                    "Auth: 完成 OAuth 登录流程失败: {}",
                    flow_error_summary(&error)
                );
                auth_flow_app_error(error)
            });

        match result {
            Ok(output) => {
                self.runtime.commit_authentication(
                    lease.generation,
                    output.auth_bundle.app_level,
                    || self.persistence.persist_auth_bundle(&output.auth_bundle),
                )?;
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

    async fn start_silent_flow_impl(
        &self,
        trace: &AuthOperationTrace,
        generation: u64,
        force_core_refresh: bool,
    ) -> AppResult<()> {
        log::info!("Auth: 静默流程开始");
        trace.stage("silentCacheReadStarted", json!({}));
        let stream_tokens = self.persistence.get_stream_tokens()?;
        let web_token = self.persistence.get_web_token()?;
        let has_any_cached_token = stream_tokens
            .as_ref()
            .map(|tokens| tokens.get("xHomeToken").is_some() || tokens.get("xCloudToken").is_some())
            .unwrap_or(false)
            && web_token.is_some();

        let token_update_time = self.persistence.get_token_update_time();
        let now_ms = now_ms();
        let should_skip_core_refresh = !force_core_refresh
            && has_any_cached_token
            && token_update_time > 0
            && now_ms.saturating_sub(token_update_time) < REFRESH_SKIP_WINDOW_MS
            && self.persistence.can_reuse_sisu_user_token()?;
        trace.stage("refreshPolicy", json!({
            "skipCoreRefresh": should_skip_core_refresh,
            "hasCachedTokens": has_any_cached_token,
            "tokenAgeMs": (token_update_time > 0).then(|| now_ms.saturating_sub(token_update_time)),
        }));

        if !should_skip_core_refresh {
            log::info!("Auth: 核心 Token 需要刷新 (超过23小时或无缓存)");
            let user_token = self
                .persistence
                .get_user_token()?
                .ok_or_else(|| AppError::Auth("Missing user token for refresh".into()))?;
            let seed = self.persistence.load_auth_flow_seed()?;
            let force_region_ip = self.config_provider.get_force_region_ip();

            log::info!("Auth: 通过 xbox-auth-flow 执行静默刷新与下游令牌收口");
            trace.stage(
                "coreAndDownstreamRefreshStarted",
                json!({ "forceRegionApplied": !force_region_ip.trim().is_empty() }),
            );
            let output = AuthFlow::new()
                .refresh_and_finalize(RefreshAndFinalizeInput {
                    refresh_token: user_token.refresh_token,
                    seed,
                    force_region_ip,
                    include_streaming_tokens: true,
                })
                .await
                .map_err(|error| {
                    trace.stage("refreshFailed", flow_error_summary(&error));
                    log::error!("Auth: 静默刷新流程失败: {}", flow_error_summary(&error));
                    auth_flow_app_error(error)
                })?;

            trace.stage(
                "authBundlePersistStarted",
                token_summary(&output.auth_bundle.stream_tokens),
            );
            self.runtime
                .commit_authentication(generation, output.auth_bundle.app_level, || {
                    self.persistence.persist_auth_bundle(&output.auth_bundle)
                })?;
            trace.stage(
                "authBundlePersistCompleted",
                json!({ "appLevel": output.auth_bundle.app_level }),
            );
            log::info!("Auth: 静默刷新与下游令牌收口完成");
            return Ok(());
        }

        log::info!("Auth: 跳过核心 Token 刷新 (23小时窗口内)");
        let user_token = self
            .persistence
            .get_user_token()?
            .ok_or_else(|| AppError::Auth("Missing user token for downstream refresh".into()))?;
        let sisu_token = self.persistence.load_flow_sisu_token()?;
        let seed = self.persistence.load_auth_flow_seed()?;
        let force_region_ip = self.config_provider.get_force_region_ip();

        log::info!("Auth: 通过 xbox-auth-flow 复用已有核心 token 获取下游令牌");
        trace.stage(
            "downstreamRefreshStarted",
            json!({ "forceRegionApplied": !force_region_ip.trim().is_empty() }),
        );
        let output = AuthFlow::new()
            .build_downstream_tokens(BuildDownstreamTokensInput {
                user_token: serde_json::from_value(serde_json::to_value(user_token)?)?,
                sisu_token,
                seed,
                force_region_ip,
            })
            .await
            .map_err(|error| {
                trace.stage("refreshFailed", flow_error_summary(&error));
                log::error!("Auth: 下游令牌收口失败: {}", flow_error_summary(&error));
                auth_flow_app_error(error)
            })?;

        trace.stage(
            "authBundlePersistStarted",
            token_summary(&output.auth_bundle.stream_tokens),
        );
        self.runtime
            .commit_authentication(generation, output.auth_bundle.app_level, || {
                self.persistence.persist_auth_bundle(&output.auth_bundle)
            })?;
        trace.stage(
            "authBundlePersistCompleted",
            json!({ "appLevel": output.auth_bundle.app_level }),
        );
        log::info!("Auth: 复用已有核心 token 的下游令牌收口完成");
        Ok(())
    }

    fn accepted_session_snapshot(
        &self,
    ) -> AppResult<Option<crate::mods::auth::token_policy::ValidSessionSnapshot>> {
        Ok(self
            .persistence
            .get_valid_session_snapshot()?
            .filter(|snapshot| {
                web_token_secret(&snapshot.web_token)
                    .is_some_and(|token| !self.runtime.is_web_token_rejected(token))
            }))
    }

    fn sync_auth_state(&self) {
        self.sync_auth_state_with_operation(None);
    }

    fn sync_auth_state_with_operation(&self, operation: Option<&AuthOperationTrace>) {
        let state = self.get_state();
        let operation_id = operation.map(|trace| trace.id());
        record_auth_event(
            &self.runtime_trace,
            "authStateEmissionStarted",
            json!({ "operationId": operation_id, "state": state_summary(&state) }),
        );
        let result = events::emit_auth_state_changed(&self.app_handle, &state);
        record_auth_event(
            &self.runtime_trace,
            "authStateEmitted",
            json!({ "operationId": operation_id, "state": state_summary(&state), "emitSucceeded": result.is_ok() }),
        );

        if state.is_authenticated {
            record_auth_event(
                &self.runtime_trace,
                "authSessionReadyEmissionStarted",
                json!({ "operationId": operation_id, "appLevel": state.app_level }),
            );
            let result =
                events::emit_session_ready(&self.app_handle, &state.provider, state.app_level);
            record_auth_event(
                &self.runtime_trace,
                "authSessionReadyEmitted",
                json!({ "operationId": operation_id, "appLevel": state.app_level, "emitSucceeded": result.is_ok() }),
            );
        }
    }
}

#[async_trait]
impl AuthProvider for AuthService {
    fn get_state(&self) -> AuthState {
        let snapshot = self.accepted_session_snapshot().ok().flatten();
        let has_web_tokens = self.get_web_api_tokens().ok().flatten().is_some();
        self.runtime
            .sync_state_from_snapshot(snapshot.as_ref(), has_web_tokens)
    }

    fn get_active_session(&self) -> AppResult<Option<AuthSessionReadyEvent>> {
        let snapshot = self.accepted_session_snapshot()?;
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
            .map_err(auth_flow_app_error)
    }

    fn get_web_api_tokens(&self) -> AppResult<Option<(String, String)>> {
        Ok(self
            .persistence
            .get_web_api_tokens()?
            .filter(|(_, token)| !self.runtime.is_web_token_rejected(token)))
    }

    fn get_web_api_token_snapshot(&self) -> AppResult<Option<Value>> {
        if self.get_web_api_tokens()?.is_some() {
            self.persistence.get_web_token()
        } else {
            Ok(None)
        }
    }

    async fn check_authentication(&self) -> AppResult<CheckAuthResponse> {
        self.check_authentication_impl().await
    }

    async fn recover_web_api_session(&self, rejected_token: &str) -> AppResult<()> {
        self.check_authentication_with_rejection(Some(rejected_token))
            .await
            .map(|_| ())
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

fn requires_interactive_login(error: &AppError) -> bool {
    matches!(error, AppError::WebApi(error) if error.to_status_code() == Some(401) || error.safe_service_code().as_deref() == Some("invalid_grant"))
}

fn auth_flow_app_error(error: xbox_auth_flow::AuthFlowError) -> AppError {
    match error {
        xbox_auth_flow::AuthFlowError::Upstream(error) => AppError::WebApi(match error {
            xbox_webapi::WebApiError::Network { kind, .. } => {
                xbox_webapi::WebApiError::network_kind(kind)
            }
            xbox_webapi::WebApiError::Http {
                status, ref code, ..
            } => match code.as_deref().and_then(xbox_webapi::sanitize_service_code) {
                Some(code) => xbox_webapi::WebApiError::http_with_code(
                    status,
                    code,
                    "Xbox authentication request rejected",
                ),
                None => {
                    xbox_webapi::WebApiError::http(status, "Xbox authentication request rejected")
                }
            },
            xbox_webapi::WebApiError::Parse { .. } => {
                xbox_webapi::WebApiError::parse("Xbox 登录协议响应异常，请稍后重试")
            }
            xbox_webapi::WebApiError::Auth { .. } => {
                xbox_webapi::WebApiError::auth("Xbox 登录信息暂不可用，请重新登录")
            }
        }),
        _ => AppError::Auth("Xbox 登录协议响应异常，请稍后重试".into()),
    }
}

fn web_token_secret(raw: &Value) -> Option<&str> {
    raw.get("data")
        .unwrap_or(raw)
        .get("Token")
        .and_then(Value::as_str)
}

fn core_refresh_required(
    rejected_token: Option<&str>,
    cached_token: Option<&Value>,
    runtime: &AuthRuntimeState,
) -> bool {
    // 401 恢复被网络中断后，下一轮检查继续刷新核心凭据。
    rejected_token.is_some()
        || cached_token
            .and_then(web_token_secret)
            .is_some_and(|token| runtime.is_web_token_rejected(token))
}

#[cfg(test)]
#[path = "service.test.rs"]
mod tests;
