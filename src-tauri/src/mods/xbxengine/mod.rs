pub mod events;
pub mod rpc;
pub mod runtime_state;
pub mod service;
mod trace_projection;

pub use service::XbxEngineService;

use async_trait::async_trait;
use std::sync::Arc;

use crate::error::AppResult;

#[async_trait]
pub trait XbxEngineProvider: Send + Sync {
    async fn dispatch_control(
        &self,
        command_name: &str,
        params: Option<serde_json::Value>,
    ) -> AppResult<()>;
    async fn snapshot_stats(&self) -> AppResult<serde_json::Value>;
    fn get_last_runtime_event(&self) -> AppResult<serde_json::Value>;
    /// 标记当前宿主是否具备可用的 xbxengine runtime。
    fn is_runtime_available(&self) -> bool;
    fn bind_tasks(&self, is_quitting: std::sync::Arc<std::sync::atomic::AtomicBool>);
    async fn shutdown(&self);
}

pub type XbxEngineProviderRef = Arc<dyn XbxEngineProvider>;
