use crate::AppState;
use tauri::{AppHandle, Manager};

pub struct AuthServiceBridge {
    app_handle: AppHandle,
}

impl AuthServiceBridge {
    pub fn new(app_handle: AppHandle) -> Self {
        Self { app_handle }
    }

    pub async fn get_streaming_token(
        &self,
        target_type: &str,
    ) -> Result<Option<serde_json::Value>, String> {
        let state = self.app_handle.state::<AppState>();
        let auth = state.auth.read().await;
        auth.get_streaming_token(target_type)
    }

    pub async fn get_transfer_token(&self) -> Result<String, String> {
        let state = self.app_handle.state::<AppState>();
        let auth = state.auth.read().await;
        auth.get_transfer_token().await
    }
}
