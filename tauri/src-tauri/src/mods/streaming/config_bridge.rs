use crate::mods::streaming::types::StreamingConfigSnapshot;
use crate::AppState;
use serde_json::Value;
use tauri::{AppHandle, Manager};

pub struct ConfigServiceBridge {
    app_handle: AppHandle,
}

impl ConfigServiceBridge {
    pub fn new(app_handle: AppHandle) -> Self {
        Self { app_handle }
    }

    pub async fn get_streaming_config(&self) -> Result<StreamingConfigSnapshot, String> {
        let keys = vec![
            "resolution".to_string(),
            "preferred_game_language".to_string(),
            "ipv6".to_string(),
            "force_region_ip".to_string(),
        ];

        let state = self.app_handle.state::<AppState>();
        let config = state.config.read().await;
        let value = config.get_by_keys(&keys)?;

        let obj = value
            .as_object()
            .ok_or("Invalid streaming config payload")?;

        Ok(StreamingConfigSnapshot {
            resolution: obj
                .get("resolution")
                .and_then(Value::as_i64)
                .unwrap_or(1080),
            preferred_game_language: obj
                .get("preferred_game_language")
                .and_then(Value::as_str)
                .unwrap_or("en-US")
                .to_string(),
            ipv6: obj.get("ipv6").and_then(Value::as_bool).unwrap_or(false),
            force_region_ip: obj
                .get("force_region_ip")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        })
    }
}
