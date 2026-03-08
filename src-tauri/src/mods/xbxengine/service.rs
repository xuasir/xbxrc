use crate::error::{AppError, AppResult};
use crate::mods::xbxengine::XbxEngineProvider;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::Mutex;
use xbxengine_app::XbxEngineApp;
use xbxengine_protocol::XbxEngineControlCommandDto;

pub struct XbxEngineService {
    engine: Arc<Mutex<XbxEngineApp>>,
    last_runtime_event: Arc<StdMutex<Option<Value>>>,
}

#[async_trait]
impl XbxEngineProvider for XbxEngineService {
    async fn dispatch_control(&self, command_name: &str, params: Option<Value>) -> AppResult<()> {
        self.dispatch_control_internal(command_name, params).await
    }

    async fn snapshot_stats(&self) -> AppResult<Value> {
        let engine = self.engine.lock().await;
        let stats = engine.snapshot_stats();
        Ok(serde_json::to_value(stats).map_err(|e| AppError::XbxEngine(e.to_string()))?)
    }

    fn bind_tasks(&self, is_quitting: std::sync::Arc<std::sync::atomic::AtomicBool>) {
        let engine = self.engine.clone();
        tauri::async_runtime::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_millis(50));
            while !is_quitting.load(std::sync::atomic::Ordering::Relaxed) {
                interval.tick().await;
                let mut lock = engine.lock().await;
                lock.tick();
            }
            log::info!("Engine tick loop stopped.");
        });
    }

    fn get_last_runtime_event(&self) -> AppResult<Value> {
        let event = self
            .last_runtime_event
            .lock()
            .map_err(|_| AppError::XbxEngine("Failed to lock last runtime event".to_string()))?
            .clone();

        Ok(event.unwrap_or(Value::Null))
    }

    async fn shutdown(&self) {
        let _ = self.dispatch_control("StopRuntime", None).await;
    }
}

impl XbxEngineService {
    pub fn new(
        engine: Arc<Mutex<XbxEngineApp>>,
        last_runtime_event: Arc<StdMutex<Option<Value>>>,
    ) -> Self {
        Self {
            engine,
            last_runtime_event,
        }
    }

    pub async fn dispatch_control_internal(
        &self,
        command_name: &str,
        params: Option<Value>,
    ) -> AppResult<()> {
        let value = match params {
            Some(payload) => json!({ command_name: payload }),
            None => json!(command_name),
        };

        let command =
            serde_json::from_value::<XbxEngineControlCommandDto>(value).map_err(|error| {
                AppError::InvalidParams(format!("Invalid xbxEngine command payload: {}", error))
            })?;
        let mut engine = self.engine.lock().await;
        engine
            .handle_control(command)
            .map_err(|error| AppError::XbxEngine(error.to_string()))
    }
}
