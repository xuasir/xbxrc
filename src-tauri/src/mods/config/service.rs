use serde_json::{Map, Value};
use tauri::AppHandle;

use super::config_policy::{filter_patch, normalize_config, split_groups};
use super::storage_repository::ConfigStorageRepository;
use super::ConfigProvider;
use crate::mods::streaming::types::{StreamingConfigSnapshot, StreamingDisplayOptionsValue};

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
            xhome_bitrate_mode: normalized
                .get("xhome_bitrate_mode")
                .and_then(Value::as_str)
                .unwrap_or("Auto")
                .to_string(),
            xhome_bitrate: normalized
                .get("xhome_bitrate")
                .and_then(Value::as_i64)
                .unwrap_or(20),
            xcloud_bitrate_mode: normalized
                .get("xcloud_bitrate_mode")
                .and_then(Value::as_str)
                .unwrap_or("Auto")
                .to_string(),
            xcloud_bitrate: normalized
                .get("xcloud_bitrate")
                .and_then(Value::as_i64)
                .unwrap_or(20),
            audio_bitrate_mode: normalized
                .get("audio_bitrate_mode")
                .and_then(Value::as_str)
                .unwrap_or("Auto")
                .to_string(),
            audio_bitrate: normalized
                .get("audio_bitrate")
                .and_then(Value::as_i64)
                .unwrap_or(20),
            codec: normalized
                .get("codec")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            polling_rate: normalized
                .get("polling_rate")
                .and_then(Value::as_i64)
                .unwrap_or(250),
            vibration: normalized
                .get("vibration")
                .and_then(Value::as_bool)
                .unwrap_or(true),
            stream_runtime_mode: normalized
                .get("stream_runtime_mode")
                .and_then(Value::as_str)
                .unwrap_or("webrtc-direct")
                .to_string(),
            power_on: normalized
                .get("power_on")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            server_url: normalized
                .get("server_url")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            server_username: normalized
                .get("server_username")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            server_credential: normalized
                .get("server_credential")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            xhome_turn_fallback: normalized
                .get("xhome_turn_fallback")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            enable_audio_control: normalized
                .get("enable_audio_control")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            video_format: normalized
                .get("video_format")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            display_options: normalized
                .get("display_options")
                .and_then(Value::as_object)
                .map(|display| StreamingDisplayOptionsValue {
                    sharpness: display
                        .get("sharpness")
                        .and_then(Value::as_i64)
                        .unwrap_or(2) as i16,
                    saturation: display
                        .get("saturation")
                        .and_then(Value::as_i64)
                        .unwrap_or(100) as i16,
                    contrast: display
                        .get("contrast")
                        .and_then(Value::as_i64)
                        .unwrap_or(100) as i16,
                    brightness: display
                        .get("brightness")
                        .and_then(Value::as_i64)
                        .unwrap_or(100) as i16,
                })
                .unwrap_or(StreamingDisplayOptionsValue {
                    sharpness: 2,
                    saturation: 100,
                    contrast: 100,
                    brightness: 100,
                }),
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
