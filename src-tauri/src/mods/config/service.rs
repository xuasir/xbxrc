use serde_json::{Map, Value};
use tauri::AppHandle;

use super::config_policy::{filter_patch, normalize_config, split_groups};
use super::storage_repository::ConfigStorageRepository;
use super::ConfigProvider;
use crate::mods::streaming::types::StreamingConfigSnapshot;

pub struct ConfigService {
    storage_repository: ConfigStorageRepository,
}

impl ConfigService {
    pub fn new(app_handle: AppHandle) -> Self {
        Self {
            storage_repository: ConfigStorageRepository::new(app_handle),
        }
    }

    pub fn get_by_keys(&self, keys: &[String]) -> Result<Value, String> {
        let normalized = self.get_normalized_config()?;
        let mut result = Map::new();

        for key in keys {
            if let Some(value) = normalized.get(key) {
                result.insert(key.to_string(), value.clone());
            }
        }

        Ok(Value::Object(result))
    }

    pub fn set_by_patch(&self, patch: &Map<String, Value>) -> Result<Value, String> {
        let filtered_patch = filter_patch(patch);

        if !filtered_patch.is_empty() {
            self.storage_repository.set_by_patch(&filtered_patch)?;
        }

        Ok(Value::Object(self.get_normalized_config()?))
    }

    pub fn get_groups(&self) -> Result<Value, String> {
        Ok(split_groups(&self.get_normalized_config()?))
    }

    fn get_normalized_config(&self) -> Result<Map<String, Value>, String> {
        let raw = self.storage_repository.get_all_settings()?;
        Ok(normalize_config(raw))
    }
}

impl ConfigProvider for ConfigService {
    fn get_force_region_ip(&self) -> String {
        self.get_normalized_config()
            .ok()
            .and_then(|normalized| {
                normalized
                    .get("force_region_ip")
                    .and_then(|value| value.as_str())
                    .map(|value| value.trim().to_string())
            })
            .unwrap_or_default()
    }

    fn get_streaming_config(&self) -> StreamingConfigSnapshot {
        let normalized = self.get_normalized_config().unwrap_or_default();

        StreamingConfigSnapshot {
            resolution: normalized
                .get("resolution")
                .and_then(Value::as_i64)
                .unwrap_or(1080),
            preferred_game_language: normalized
                .get("preferred_game_language")
                .and_then(Value::as_str)
                .unwrap_or("en-US")
                .to_string(),
            ipv6: normalized
                .get("ipv6")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            force_region_ip: normalized
                .get("force_region_ip")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        }
    }

    fn get_by_keys(&self, keys: &[String]) -> Result<serde_json::Value, String> {
        self.get_by_keys(keys)
    }

    fn set_by_patch(
        &self,
        patch: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<serde_json::Value, String> {
        self.set_by_patch(patch)
    }

    fn get_groups(&self) -> Result<serde_json::Value, String> {
        self.get_groups()
    }
}
