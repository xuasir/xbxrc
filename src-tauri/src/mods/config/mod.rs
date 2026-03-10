pub mod config_policy;
pub mod defaults;
pub mod grouping;
pub mod rpc;
pub mod service;
pub mod storage_repository;

pub use service::ConfigService;

use crate::mods::streaming::types::StreamingConfigSnapshot;
use std::sync::Arc;

pub trait ConfigProvider: Send + Sync {
    fn get_force_region_ip(&self) -> String;
    fn get_streaming_config(&self) -> StreamingConfigSnapshot;
    fn get_by_keys(&self, keys: &[String]) -> Result<serde_json::Value, String>;
    fn set_by_patch(
        &self,
        patch: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<serde_json::Value, String>;
    fn get_groups(&self) -> Result<serde_json::Value, String>;
}

pub type ConfigProviderRef = Arc<dyn ConfigProvider>;
