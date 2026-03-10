use crate::error::AppResult;
use crate::mods::xbxengine::XbxEngineProvider;
use async_trait::async_trait;
use serde_json::Value;
use std::sync::{Arc, Mutex as StdMutex};

pub struct PlaceholderXbxEngineService {
    last_runtime_event: Arc<StdMutex<Option<Value>>>,
}

#[async_trait]
impl XbxEngineProvider for PlaceholderXbxEngineService {
    async fn dispatch_control(&self, _command_name: &str, _params: Option<Value>) -> AppResult<()> {
        log::warn!(
            "PlaceholderXbxEngineService::dispatch_control called - xbxengine host bridge is not connected yet"
        );
        Ok(())
    }

    async fn snapshot_stats(&self) -> AppResult<Value> {
        Ok(serde_json::json!({ "placeholder": true }))
    }

    fn bind_tasks(&self, _is_quitting: std::sync::Arc<std::sync::atomic::AtomicBool>) {
        log::info!("PlaceholderXbxEngineService::bind_tasks - no-op");
    }

    fn get_last_runtime_event(&self) -> AppResult<Value> {
        let event = self
            .last_runtime_event
            .lock()
            .map_err(|_| {
                crate::error::AppError::XbxEngine("Failed to lock last runtime event".to_string())
            })?
            .clone();

        Ok(event.unwrap_or(Value::Null))
    }

    async fn shutdown(&self) {
        log::info!("PlaceholderXbxEngineService::shutdown - no-op");
    }
}

impl PlaceholderXbxEngineService {
    pub fn new(last_runtime_event: Arc<StdMutex<Option<Value>>>) -> Self {
        Self { last_runtime_event }
    }
}
