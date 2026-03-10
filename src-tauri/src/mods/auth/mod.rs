pub mod events;
pub mod persistence_service;
pub mod rpc;
pub mod runtime_state;
pub mod service;
pub mod storage_repository;
pub mod token_policy;
pub mod types;

pub use service::AuthService;
pub use storage_repository::AuthStorageRepository;
pub use types::*;

use async_trait::async_trait;
use std::sync::Arc;

use crate::error::AppResult;

#[async_trait]
pub trait AuthProvider: Send + Sync {
    fn get_state(&self) -> AuthState;
    fn get_active_session(&self) -> AppResult<Option<AuthSessionReadyEvent>>;
    fn get_streaming_token(&self, target_type: &str) -> AppResult<Option<serde_json::Value>>;
    async fn login(&self) -> AppResult<LoginResponse>;
    async fn get_transfer_token(&self) -> AppResult<String>;
    fn get_web_api_tokens(&self) -> AppResult<Option<(String, String)>>;
    async fn check_authentication(&self) -> AppResult<CheckAuthResponse>;
    async fn clear_auth_cache(&self, scope: &str) -> AppResult<()>;
    async fn logout(&self) -> AppResult<()>;
    async fn handle_oauth_callback(&self, callback_url: &str) -> AppResult<()>;
    async fn cancel_pending_login(&self);
    async fn reset_runtime_after_store_purge(&self);
    fn mark_callback_processing(&self) -> AppResult<()>;
    fn unmark_callback_processing(&self) -> AppResult<()>;
}

pub type AuthProviderRef = Arc<dyn AuthProvider>;
