use crate::error::{AppError, AppResult};
use crate::mods::auth::token_policy::ValidSessionSnapshot;
use crate::mods::auth::types::{AuthState, CheckAuthResponse};
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
    last_check_at: Option<Instant>,
    generation: u64,
    rejected_web_token: Option<String>,
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
                last_check_at: None,
                generation: 0,
                rejected_web_token: None,
            }),
            check_gate: tokio::sync::Mutex::new(()),
        }
    }

    pub fn begin_login(&self) -> AppResult<()> {
        let mut inner = self.lock_mut()?;
        inner.generation = inner.generation.wrapping_add(1);
        inner.state.is_authenticating = true;
        Ok(())
    }

    pub fn fail_login(&self) -> AppResult<()> {
        let mut inner = self.lock_mut()?;
        inner.pending_redirect_flow = None;
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
        inner.state.is_authenticated = true;
        inner.state.is_authenticating = false;
        inner.state.app_level = app_level;
        Ok(())
    }

    pub fn mark_authenticating_idle(&self) -> AppResult<()> {
        let mut inner = self.lock_mut()?;
        inner.state.is_authenticating = false;
        Ok(())
    }

    pub fn clear_auth_state(&self) -> AppResult<()> {
        let mut inner = self.lock_mut()?;
        inner.generation = inner.generation.wrapping_add(1);
        inner.rejected_web_token = None;
        inner.last_check_at = None;
        inner.pending_redirect_flow = None;
        inner.state.is_authenticated = false;
        inner.state.is_authenticating = false;
        inner.state.app_level = 0;
        Ok(())
    }

    pub fn reset_after_store_purge(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.generation = inner.generation.wrapping_add(1);
            inner.rejected_web_token = None;
            inner.last_check_at = None;
            inner.pending_redirect_flow = None;
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

            if inner.pending_redirect_flow.is_some() || inner.state.is_authenticating {
                inner.generation = inner.generation.wrapping_add(1);
                inner.rejected_web_token = None;
                inner.pending_redirect_flow = None;
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
            Err(AppError::Auth("登录状态已更新，请重新查询".into()))
        }
    }

    pub fn commit_authentication(
        &self,
        generation: u64,
        app_level: u32,
        persist: impl FnOnce() -> AppResult<()>,
    ) -> AppResult<()> {
        let mut inner = self.lock_mut()?;
        if inner.generation != generation {
            return Err(AppError::Auth("登录状态已更新，请重新查询".into()));
        }
        // 同步持久化与注销共用 generation 锁，迟到的刷新结果无法重新写入已注销凭据。
        persist()?;
        inner.rejected_web_token = None;
        inner.state.is_authenticated = true;
        inner.state.is_authenticating = false;
        inner.state.app_level = app_level;
        Ok(())
    }

    pub fn clear_credentials(&self, clear: impl FnOnce() -> AppResult<()>) -> AppResult<()> {
        let mut inner = self.lock_mut()?;
        inner.generation = inner.generation.wrapping_add(1);
        inner.rejected_web_token = None;
        inner.last_check_at = None;
        inner.pending_redirect_flow = None;
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

    pub fn store_login_seed(
        &self,
        generation: u64,
        pending: PendingOAuthLogin,
        persist: impl FnOnce() -> AppResult<()>,
    ) -> AppResult<()> {
        let mut inner = self.lock_mut()?;
        if inner.generation != generation {
            return Err(AppError::Auth("登录状态已更新，请重新查询".into()));
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
