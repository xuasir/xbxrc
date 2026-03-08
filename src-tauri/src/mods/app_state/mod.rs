pub mod rpc;
pub mod service;

pub use service::{
    AppStateService, ClearDataResult, ClearUserDataResult, PingPayload, StartupFlagsPayload,
};

use async_trait::async_trait;
use std::sync::Arc;

#[async_trait]
pub trait AppStateProvider: Send + Sync {
    async fn clear_user_data(&self) -> Result<ClearUserDataResult, String>;
    async fn clear_data(&self) -> Result<ClearDataResult, String>;
    fn get_version(&self) -> String;
    fn ping(&self, message: &str) -> PingPayload;
    fn is_fullscreen(&self) -> bool;
    fn toggle_fullscreen(&self) -> Result<bool, String>;
    fn enter_fullscreen(&self) -> Result<bool, String>;
    fn exit_fullscreen(&self) -> Result<bool, String>;
    async fn get_startup_flags(&self) -> StartupFlagsPayload;
    async fn reset_auto_connect(&self) -> bool;
    async fn quit(&self);
    async fn restart(&self);
    async fn restart_delayed(&self, delay_ms: u64);
    fn open_external(&self, url: &str) -> Result<(), String>;
}

pub type AppStateProviderRef = Arc<dyn AppStateProvider>;
