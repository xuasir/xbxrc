use crate::mods::data::domain::{DataAuthState, DataSessionContext};
use crate::AppState;
use tauri::{AppHandle, Manager};

pub struct AuthServiceBridge {
    app_handle: AppHandle,
}

impl AuthServiceBridge {
    pub fn new(app_handle: AppHandle) -> Self {
        Self { app_handle }
    }

    pub async fn get_state(&self) -> Result<DataAuthState, String> {
        let state = self.app_handle.state::<AppState>();
        let auth_state = state.auth.get_state();

        Ok(DataAuthState {
            provider: auth_state.provider,
            is_authenticating: auth_state.is_authenticating,
            is_authenticated: auth_state.is_authenticated,
            app_level: auth_state.app_level,
        })
    }

    pub async fn check_authentication(&self) -> Result<(), String> {
        let state = self.app_handle.state::<AppState>();
        state
            .auth
            .check_authentication()
            .await
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    pub async fn get_active_session(&self) -> Result<Option<DataSessionContext>, String> {
        let state = self.app_handle.state::<AppState>();
        let Some(session) = state
            .auth
            .get_active_session()
            .map_err(|error| error.to_string())?
        else {
            return Ok(None);
        };

        Ok(Some(DataSessionContext {
            provider: session.provider,
            app_level: session.app_level,
            streaming_tokens: session.streaming_tokens,
            web_token: session.web_token,
        }))
    }
}
