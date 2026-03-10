use crate::error::{AppError, AppResult};
use crate::mods::auth::token_policy::ValidSessionSnapshot;
use crate::mods::auth::types::{AuthState, CheckAuthResponse};
use xbox_auth_flow::PendingOAuthLogin;

pub struct AuthRuntimeState {
    inner: std::sync::Mutex<AuthRuntimeStateInner>,
}

struct AuthRuntimeStateInner {
    state: AuthState,
    pending_redirect_flow: Option<PendingOAuthLogin>,
    is_processing_callback: bool,
    last_check_at_ms: u64,
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
                last_check_at_ms: 0,
            }),
        }
    }

    pub fn begin_login(&self) -> AppResult<()> {
        let mut inner = self.lock_mut()?;
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
        inner.pending_redirect_flow = None;
        inner.state.is_authenticated = false;
        inner.state.is_authenticating = false;
        inner.state.app_level = 0;
        Ok(())
    }

    pub fn reset_after_store_purge(&self) {
        if let Ok(mut inner) = self.inner.lock() {
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

    pub fn begin_check(&self, now_ms: u64, cooldown_ms: u64) -> AppResult<BeginCheckOutcome> {
        let mut inner = self.lock_mut()?;

        if inner.state.is_authenticating {
            return Ok(BeginCheckOutcome::ShortCircuit(CheckAuthResponse {
                provider: inner.state.provider.clone(),
                started_silent_flow: false,
            }));
        }

        if inner.state.is_authenticated
            && now_ms.saturating_sub(inner.last_check_at_ms) < cooldown_ms
        {
            return Ok(BeginCheckOutcome::ShortCircuit(CheckAuthResponse {
                provider: inner.state.provider.clone(),
                started_silent_flow: false,
            }));
        }

        inner.last_check_at_ms = now_ms;
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
        inner.state.is_authenticated =
            previous_state.is_authenticated || has_web_tokens || fallback_app_level > 0;
        inner.state.app_level = if fallback_app_level > 0 {
            fallback_app_level
        } else if previous_state.app_level > 0 {
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

    pub fn sync_state_from_snapshot(&self, snapshot: Option<&ValidSessionSnapshot>) -> AuthState {
        let mut inner = self.inner.lock().expect("Auth runtime state lock poisoned");

        if !inner.state.is_authenticating {
            if let Some(snapshot) = snapshot {
                inner.state.is_authenticated = true;
                inner.state.app_level = snapshot.app_level;
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
