use serde_json::{json, Value};
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::Mutex;
use xbxengine_app::XbxEngineApp;
use xbxengine_protocol::XbxEngineControlCommandDto;

pub struct XbxEngineService {
    engine: Arc<Mutex<XbxEngineApp>>,
    last_runtime_event: Arc<StdMutex<Option<Value>>>,
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

    // 统一分发控制命令，保持 RPC 层只做参数编排。
    pub async fn dispatch_control(
        &self,
        command_name: &str,
        params: Option<Value>,
    ) -> Result<(), String> {
        let value = match params {
            Some(payload) => json!({ command_name: payload }),
            None => json!(command_name),
        };

        let command = serde_json::from_value::<XbxEngineControlCommandDto>(value)
            .map_err(|error| format!("Invalid xbxEngine command payload: {}", error))?;
        let mut engine = self.engine.lock().await;
        engine
            .handle_control(command)
            .map_err(|error| error.to_string())
    }

    pub async fn snapshot_stats(&self) -> Value {
        let engine = self.engine.lock().await;
        let stats = engine.snapshot_stats();
        serde_json::to_value(stats).unwrap_or_else(|_| json!({}))
    }

    pub fn get_last_runtime_event(&self) -> Result<Value, String> {
        let event = self
            .last_runtime_event
            .lock()
            .map_err(|_| "Failed to lock last runtime event".to_string())?
            .clone();

        Ok(event.unwrap_or(Value::Null))
    }

    pub async fn shutdown(&self) {
        let _ = self.dispatch_control("StopRuntime", None).await;
    }
}
