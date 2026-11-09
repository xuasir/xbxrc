use crate::error::{AppError, AppResult};
use crate::mods::auth::token_policy::ValidSessionSnapshot;
use crate::mods::auth::types::{AuthState, AuthTokenScope, CheckAuthResponse};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use xbox_auth_flow::PendingOAuthLogin;

pub struct AuthRuntimeState {
    inner: std::sync::Mutex<AuthRuntimeStateInner>,
    check_gate: tokio::sync::Mutex<()>,
}

struct AuthRuntimeStateInner {
    state: AuthState,
    pending_redirect_flow: Option<PendingOAuthLogin>,
    is_processing_callback: bool,
    login_in_progress: bool,
    last_check_at: Option<Instant>,
    generation: u64,
    rejected_web_token: Option<String>,
    rejected_stream_tokens: HashMap<&'static str, String>,
    refresh_token_rejected: bool,
    refresh_failures: HashMap<AuthTokenScope, (Instant, xbox_webapi::WebApiError)>,
}

pub struct AuthCheckLease<'a> {
    runtime: &'a AuthRuntimeState,
    pub generation: u64,
}

impl Drop for AuthCheckLease<'_> {
    fn drop(&mut self) {
        if let Ok(mut inner) = self.runtime.inner.lock() {
            if inner.generation == self.generation {
                inner.state.is_authenticating = false;
            }
        }
    }
}

pub enum BeginCheckOutcome {
    ShortCircuit(CheckAuthResponse),
    Proceed { previous_state: AuthState },
}

impl AuthRuntimeState {
    pub fn new(provider: String, is_authenticated: bool, app_level: u32) -> Self {
        Self {
            inner: std::sync::Mutex::new(AuthRuntimeStateInner {
                state: AuthState {
                    provider,
                    is_authenticating: false,
                    is_authenticated,
                    app_level,
                },
                pending_redirect_flow: None,
                is_processing_callback: false,
                login_in_progress: false,
                last_check_at: None,
                generation: 0,
                rejected_web_token: None,
                rejected_stream_tokens: HashMap::new(),
                refresh_token_rejected: false,
                refresh_failures: HashMap::new(),
            }),
            check_gate: tokio::sync::Mutex::new(()),
        }
    }

    pub fn begin_login(&self) -> AppResult<()> {
        let mut inner = self.lock_mut()?;
        inner.generation = inner.generation.wrapping_add(1);
        inner.login_in_progress = true;
        inner.state.is_authenticating = true;
        Ok(())
    }

    pub fn fail_login(&self) -> AppResult<()> {
        let mut inner = self.lock_mut()?;
        inner.pending_redirect_flow = None;
        inner.login_in_progress = false;
        inner.state.is_authenticating = false;
        Ok(())
    }

    pub fn store_pending_redirect_flow(&self, pending: PendingOAuthLogin) -> AppResult<()> {
        let mut inner = self.lock_mut()?;
        inner.pending_redirect_flow = Some(pending);
        Ok(())
    }

    pub fn take_pending_redirect_flow(&self) -> AppResult<Option<PendingOAuthLogin>> {
        let mut inner = self.lock_mut()?;
        Ok(inner.pending_redirect_flow.take())
    }

    pub fn mark_authenticated(&self, app_level: u32) -> AppResult<()> {
        let mut inner = self.lock_mut()?;
        inner.login_in_progress = false;
        inner.state.is_authenticated = true;
        inner.state.is_authenticating = false;
        inner.state.app_level = app_level;
        Ok(())
    }

    pub fn mark_authenticating_idle(&self) -> AppResult<()> {
        let mut inner = self.lock_mut()?;
        inner.login_in_progress = false;
        inner.state.is_authenticating = false;
        Ok(())
    }

    pub fn clear_auth_state(&self) -> AppResult<()> {
        let mut inner = self.lock_mut()?;
        inner.generation = inner.generation.wrapping_add(1);
        inner.rejected_web_token = None;
        inner.rejected_stream_tokens.clear();
        inner.refresh_token_rejected = false;
        inner.refresh_failures.clear();
        inner.last_check_at = None;
        inner.pending_redirect_flow = None;
        inner.login_in_progress = false;
        inner.is_processing_callback = false;
        inner.state.is_authenticated = false;
        inner.state.is_authenticating = false;
        inner.state.app_level = 0;
        Ok(())
    }

    pub fn reset_after_store_purge(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.generation = inner.generation.wrapping_add(1);
            inner.rejected_web_token = None;
            inner.rejected_stream_tokens.clear();
            inner.refresh_token_rejected = false;
            inner.refresh_failures.clear();
            inner.last_check_at = None;
            inner.pending_redirect_flow = None;
            inner.login_in_progress = false;
            inner.is_processing_callback = false;
            inner.state.is_authenticated = false;
            inner.state.is_authenticating = false;
            inner.state.app_level = 0;
        } else {
            log::warn!("Failed to acquire auth runtime state lock during reset");
        }
    }

    pub fn cancel_pending_login(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            if inner.is_processing_callback {
                return;
            }

            if inner.pending_redirect_flow.is_some() || inner.login_in_progress {
                inner.generation = inner.generation.wrapping_add(1);
                inner.pending_redirect_flow = None;
                inner.login_in_progress = false;
                inner.state.is_authenticating = false;
                inner.state.is_authenticated = false;
                inner.state.app_level = 0;
            }
        } else {
            log::warn!("Failed to acquire auth runtime state lock during cancel_pending_login");
        }
    }

    pub fn mark_callback_processing(&self) -> AppResult<()> {
        let mut inner = self.lock_mut()?;
        inner.is_processing_callback = true;
        Ok(())
    }

    pub fn unmark_callback_processing(&self) -> AppResult<()> {
        let mut inner = self.lock_mut()?;
        inner.is_processing_callback = false;
        Ok(())
    }

    pub fn begin_check(
        &self,
        now: Instant,
        cooldown: Duration,
        has_valid_credentials: bool,
    ) -> AppResult<BeginCheckOutcome> {
        let mut inner = self.lock_mut()?;

        if inner.state.is_authenticating {
            return Ok(BeginCheckOutcome::ShortCircuit(CheckAuthResponse {
                provider: inner.state.provider.clone(),
                started_silent_flow: false,
            }));
        }

        if inner.state.is_authenticated
            && has_valid_credentials
            && inner
                .last_check_at
                .is_some_and(|previous| now.saturating_duration_since(previous) < cooldown)
        {
            return Ok(BeginCheckOutcome::ShortCircuit(CheckAuthResponse {
                provider: inner.state.provider.clone(),
                started_silent_flow: false,
            }));
        }

        inner.last_check_at = Some(now);
        let previous_state = inner.state.clone();
        inner.state.is_authenticating = true;
        Ok(BeginCheckOutcome::Proceed { previous_state })
    }

    pub fn finish_check_from_snapshot(
        &self,
        snapshot: &ValidSessionSnapshot,
    ) -> AppResult<CheckAuthResponse> {
        let mut inner = self.lock_mut()?;
        inner.state.is_authenticating = false;
        inner.state.is_authenticated = true;
        inner.login_in_progress = false;
        inner.state.app_level = snapshot.app_level;
        Ok(CheckAuthResponse {
            provider: inner.state.provider.clone(),
            started_silent_flow: false,
        })
    }

    pub fn finish_check_success(&self, started_silent_flow: bool) -> AppResult<CheckAuthResponse> {
        let inner = self.lock()?;
        Ok(CheckAuthResponse {
            provider: inner.state.provider.clone(),
            started_silent_flow,
        })
    }

    pub fn finish_check_transient_failure(
        &self,
        previous_state: &AuthState,
        has_web_tokens: bool,
        fallback_app_level: u32,
    ) -> AppResult<CheckAuthResponse> {
        let mut inner = self.lock_mut()?;
        inner.state.is_authenticating = false;
        inner.state.is_authenticated = has_web_tokens || fallback_app_level > 0;
        inner.state.app_level = if fallback_app_level > 0 {
            fallback_app_level
        } else if has_web_tokens && previous_state.app_level > 0 {
            previous_state.app_level
        } else if has_web_tokens {
            1
        } else {
            0
        };

        Ok(CheckAuthResponse {
            provider: inner.state.provider.clone(),
            started_silent_flow: false,
        })
    }

    pub fn finish_check_unauthenticated(&self) -> AppResult<CheckAuthResponse> {
        let mut inner = self.lock_mut()?;
        inner.state.is_authenticating = false;
        inner.state.is_authenticated = false;
        inner.state.app_level = 0;
        Ok(CheckAuthResponse {
            provider: inner.state.provider.clone(),
            started_silent_flow: false,
        })
    }

    pub fn sync_state_from_snapshot(
        &self,
        snapshot: Option<&ValidSessionSnapshot>,
        has_web_tokens: bool,
    ) -> AuthState {
        let mut inner = self.inner.lock().expect("Auth runtime state lock poisoned");

        if !inner.state.is_authenticating {
            if let Some(snapshot) = snapshot {
                inner.state.is_authenticated = true;
                inner.state.app_level = snapshot.app_level;
            } else {
                inner.state.is_authenticated = has_web_tokens;
                inner.state.app_level = 0;
            }
        }

        inner.state.clone()
    }

    pub fn current_state(&self) -> AuthState {
        self.inner
            .lock()
            .expect("Auth runtime state lock poisoned")
            .state
            .clone()
    }

    pub fn provider(&self) -> AppResult<String> {
        Ok(self.lock()?.state.provider.clone())
    }

    pub async fn lock_check(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.check_gate.lock().await
    }

    pub fn check_lease(&self) -> AppResult<AuthCheckLease<'_>> {
        Ok(AuthCheckLease {
            runtime: self,
            generation: self.lock()?.generation,
        })
    }

    pub fn generation(&self) -> AppResult<u64> {
        Ok(self.lock()?.generation)
    }

    pub fn ensure_generation(&self, generation: u64) -> AppResult<()> {
        if self.lock()?.generation == generation {
            Ok(())
        } else {
            Err(AppError::WebApi(xbox_webapi::WebApiError::parse(
                "登录状态已更新，请重新查询",
            )))
        }
    }

    pub fn commit_authentication(
        &self,
        generation: u64,
        app_level: u32,
        persist: impl FnOnce() -> AppResult<()>,
    ) -> AppResult<()> {
        self.commit_scoped(generation, app_level, None, persist)
    }

    pub fn commit_scoped_authentication(
        &self,
        generation: u64,
        app_level: u32,
        scope: AuthTokenScope,
        persist: impl FnOnce() -> AppResult<()>,
    ) -> AppResult<()> {
        self.commit_scoped(generation, app_level, Some(scope), persist)
    }

    fn commit_scoped(
        &self,
        generation: u64,
        app_level: u32,
        scope: Option<AuthTokenScope>,
        persist: impl FnOnce() -> AppResult<()>,
    ) -> AppResult<()> {
        let mut inner = self.lock_mut()?;
        if inner.generation != generation {
            return Err(AppError::WebApi(xbox_webapi::WebApiError::parse(
                "登录状态已更新，请重新查询",
            )));
        }
        // 同步持久化与注销共用 generation 锁，迟到的刷新结果无法重新写入已注销凭据。
        persist()?;
        inner.rejected_web_token = None;
        if let Some(scope) = scope {
            inner.rejected_stream_tokens.remove(scope.as_str());
            inner.refresh_failures.remove(&scope);
        } else {
            inner.rejected_stream_tokens.clear();
            inner.refresh_token_rejected = false;
            inner.refresh_failures.clear();
        }
        inner.state.is_authenticated = true;
        inner.state.is_authenticating = false;
        inner.state.app_level = app_level;
        inner.login_in_progress = false;
        Ok(())
    }

    pub fn clear_credentials(&self, clear: impl FnOnce() -> AppResult<()>) -> AppResult<()> {
        let mut inner = self.lock_mut()?;
        inner.generation = inner.generation.wrapping_add(1);
        inner.rejected_web_token = None;
        inner.rejected_stream_tokens.clear();
        inner.refresh_token_rejected = false;
        inner.refresh_failures.clear();
        inner.last_check_at = None;
        inner.pending_redirect_flow = None;
        inner.login_in_progress = false;
        inner.is_processing_callback = false;
        inner.state.is_authenticated = false;
        inner.state.is_authenticating = false;
        inner.state.app_level = 0;
        clear()
    }

    pub fn reject_web_token(&self, token: &str) -> AppResult<()> {
        self.lock_mut()?.rejected_web_token = Some(token.to_string());
        Ok(())
    }

    pub fn is_web_token_rejected(&self, token: &str) -> bool {
        self.inner
            .lock()
            .map(|inner| inner.rejected_web_token.as_deref() == Some(token))
            .unwrap_or(true)
    }

    pub fn reject_stream_token(&self, scope: AuthTokenScope, token: &str) -> AppResult<()> {
        self.lock_mut()?
            .rejected_stream_tokens
            .insert(scope.as_str(), token.to_string());
        Ok(())
    }

    pub fn is_stream_token_rejected(&self, scope: AuthTokenScope, token: &str) -> bool {
        self.inner
            .lock()
            .map(|inner| {
                inner
                    .rejected_stream_tokens
                    .get(scope.as_str())
                    .is_some_and(|rejected| rejected == token)
            })
            .unwrap_or(true)
    }

    pub fn is_refresh_token_rejected(&self) -> bool {
        self.inner
            .lock()
            .map(|inner| inner.refresh_token_rejected)
            .unwrap_or(true)
    }

    pub fn cached_refresh_failure(
        &self,
        scope: AuthTokenScope,
        now: Instant,
    ) -> Option<xbox_webapi::WebApiError> {
        self.inner
            .lock()
            .ok()?
            .refresh_failures
            .get(&scope)
            .filter(|(recorded, _)| {
                now.saturating_duration_since(*recorded) < Duration::from_secs(1)
            })
            .map(|(_, error)| error.clone())
    }

    pub fn remember_refresh_failure(
        &self,
        generation: u64,
        scope: AuthTokenScope,
        now: Instant,
        error: xbox_webapi::WebApiError,
    ) -> AppResult<()> {
        let mut inner = self.lock_mut()?;
        if inner.generation != generation {
            return Err(AppError::WebApi(xbox_webapi::WebApiError::parse(
                "登录状态已更新，请重新查询",
            )));
        }
        inner.refresh_failures.insert(scope, (now, error));
        Ok(())
    }

    pub fn reject_refresh_token(&self, generation: u64) -> AppResult<()> {
        let mut inner = self.lock_mut()?;
        if inner.generation != generation {
            return Err(AppError::WebApi(xbox_webapi::WebApiError::parse(
                "登录状态已更新，请重新查询",
            )));
        }
        inner.refresh_token_rejected = true;
        Ok(())
    }

    /// 核心凭据轮换先落盘；下游失败时仍保留本轮续期结果。
    pub fn checkpoint_credentials(
        &self,
        generation: u64,
        persist: impl FnOnce() -> AppResult<()>,
    ) -> AppResult<()> {
        let mut inner = self.lock_mut()?;
        if inner.generation != generation {
            return Err(AppError::WebApi(xbox_webapi::WebApiError::parse(
                "登录状态已更新，请重新查询",
            )));
        }
        persist()?;
        inner.refresh_token_rejected = false;
        Ok(())
    }

    pub fn store_login_seed(
        &self,
        generation: u64,
        pending: PendingOAuthLogin,
        persist: impl FnOnce() -> AppResult<()>,
    ) -> AppResult<()> {
        let mut inner = self.lock_mut()?;
        if inner.generation != generation {
            return Err(AppError::WebApi(xbox_webapi::WebApiError::parse(
                "登录状态已更新，请重新查询",
            )));
        }
        persist()?;
        inner.pending_redirect_flow = Some(pending);
        Ok(())
    }

    fn lock(&self) -> AppResult<std::sync::MutexGuard<'_, AuthRuntimeStateInner>> {
        self.inner.lock().map_err(|error| {
            AppError::Internal(format!(
                "Failed to acquire auth runtime state lock: {}",
                error
            ))
        })
    }

    fn lock_mut(&self) -> AppResult<std::sync::MutexGuard<'_, AuthRuntimeStateInner>> {
        self.lock()
    }
}

#[cfg(test)]
#[path = "runtime_state.test.rs"]
mod tests;
