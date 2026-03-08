use serde_json::{Map, Value};

use super::defaults::{allowed_key, default_config_map};
use super::grouping::split_config_groups;
use super::repository::ConfigRepository;
use super::ConfigProvider;
use crate::mods::streaming::types::StreamingConfigSnapshot;

pub struct ConfigService {
    repository: ConfigRepository,
}

impl ConfigService {
    pub fn new(repository: ConfigRepository) -> Self {
        Self { repository }
    }

    fn normalize_number(key: &str, value: &Value, fallback: i64, min: i64, max: i64) -> Value {
        let parsed = if let Some(val) = value.as_i64() {
            val.clamp(min, max)
        } else {
            log::warn!(
                "Config key '{}' missing or invalid, using fallback: {}",
                key,
                fallback
            );
            fallback
        };
        Value::from(parsed)
    }

    fn normalize_string_enum(key: &str, value: &Value, fallback: &str, allowed: &[&str]) -> Value {
        if let Some(raw) = value.as_str() {
            if allowed.contains(&raw) {
                return Value::from(raw.to_string());
            }
        }
        log::warn!(
            "Config key '{}' missing or invalid enum, using fallback: {}",
            key,
            fallback
        );
        Value::from(fallback.to_string())
    }

    fn normalize_display_options(key: &str, value: &Value, fallback: &Value) -> Value {
        let Some(map) = value.as_object() else {
            log::warn!(
                "Config key '{}' missing or invalid display options, using fallback",
                key
            );
            return fallback.clone();
        };

        let Some(default_map) = fallback.as_object() else {
            return fallback.clone();
        };

        let sharpness = if let Some(val) = map.get("sharpness").and_then(Value::as_i64) {
            val.clamp(0, 10)
        } else {
            log::warn!(
                "Config key '{}.sharpness' missing or invalid, using fallback: {}",
                key,
                default_map["sharpness"].as_i64().unwrap_or(2)
            );
            default_map["sharpness"].as_i64().unwrap_or(2)
        };
        let saturation = if let Some(val) = map.get("saturation").and_then(Value::as_i64) {
            val.clamp(0, 200)
        } else {
            log::warn!(
                "Config key '{}.saturation' missing or invalid, using fallback: {}",
                key,
                default_map["saturation"].as_i64().unwrap_or(100)
            );
            default_map["saturation"].as_i64().unwrap_or(100)
        };
        let contrast = if let Some(val) = map.get("contrast").and_then(Value::as_i64) {
            val.clamp(0, 200)
        } else {
            log::warn!(
                "Config key '{}.contrast' missing or invalid, using fallback: {}",
                key,
                default_map["contrast"].as_i64().unwrap_or(100)
            );
            default_map["contrast"].as_i64().unwrap_or(100)
        };
        let brightness = if let Some(val) = map.get("brightness").and_then(Value::as_i64) {
            val.clamp(0, 200)
        } else {
            log::warn!(
                "Config key '{}.brightness' missing or invalid, using fallback: {}",
                key,
                default_map["brightness"].as_i64().unwrap_or(100)
            );
            default_map["brightness"].as_i64().unwrap_or(100)
        };

        serde_json::json!({
            "sharpness": sharpness,
            "saturation": saturation,
            "contrast": contrast,
            "brightness": brightness
        })
    }

    fn normalize_value(key: &str, value: &Value, fallback: &Value) -> Value {
        match key {
            "locale"
            | "preferred_game_language"
            | "force_region_ip"
            | "codec"
            | "video_format"
            | "server_url"
            | "server_username"
            | "server_credential" => {
                if let Some(val) = value.as_str() {
                    Value::from(val.to_string())
                } else {
                    log::warn!(
                        "Config key '{}' missing or invalid, using fallback: {}",
                        key,
                        fallback.as_str().unwrap_or("")
                    );
                    Value::from(fallback.as_str().unwrap_or(""))
                }
            }
            "use_msal"
            | "fullscreen"
            | "xhome_turn_fallback"
            | "enable_audio_control"
            | "vibration"
            | "power_on"
            | "ipv6"
            | "performance_style"
            | "background_keepalive"
            | "use_vulkan" => {
                if let Some(val) = value.as_bool() {
                    Value::from(val)
                } else {
                    log::warn!(
                        "Config key '{}' missing or invalid, using fallback: {}",
                        key,
                        fallback.as_bool().unwrap_or(false)
                    );
                    Value::from(fallback.as_bool().unwrap_or(false))
                }
            }
            "resolution" => {
                Self::normalize_number(key, value, fallback.as_i64().unwrap_or(720), 720, 1081)
            }
            "xhome_bitrate" | "xcloud_bitrate" | "audio_bitrate" => {
                Self::normalize_number(key, value, fallback.as_i64().unwrap_or(20), 0, 200)
            }
            "polling_rate" => {
                Self::normalize_number(key, value, fallback.as_i64().unwrap_or(250), 1, 1000)
            }
            "xhome_bitrate_mode" | "xcloud_bitrate_mode" | "audio_bitrate_mode" => {
                Self::normalize_string_enum(
                    key,
                    value,
                    fallback.as_str().unwrap_or("Auto"),
                    &["Auto", "Custom"],
                )
            }
            "stream_runtime_mode" => Self::normalize_string_enum(
                key,
                value,
                fallback.as_str().unwrap_or("webrtc-direct"),
                &["webrtc-direct", "rust-owned"],
            ),
            "display_options" => Self::normalize_display_options(key, value, fallback),
            _ => fallback.clone(),
        }
    }

    fn normalize_config(&self, source: Map<String, Value>) -> Map<String, Value> {
        let defaults = default_config_map();
        let mut normalized = Map::new();

        for (key, fallback) in &defaults {
            let candidate = source.get(key).unwrap_or(fallback);
            normalized.insert(
                key.to_string(),
                Self::normalize_value(key, candidate, fallback),
            );
        }

        normalized
    }

    pub fn get_by_keys(&self, keys: &[String]) -> Result<Value, String> {
        let raw = self.repository.get_all_settings()?;
        let normalized = self.normalize_config(raw);
        let mut result = Map::new();

        for key in keys {
            if let Some(value) = normalized.get(key) {
                result.insert(key.to_string(), value.clone());
            }
        }

        Ok(Value::Object(result))
    }

    pub fn set_by_patch(&self, patch: &Map<String, Value>) -> Result<Value, String> {
        let mut filtered_patch = Map::new();
        for (key, value) in patch {
            if allowed_key(key) {
                filtered_patch.insert(key.to_string(), value.clone());
            }
        }

        if !filtered_patch.is_empty() {
            self.repository.set_by_patch(&filtered_patch)?;
        }

        let all = self.repository.get_all_settings()?;
        let normalized = self.normalize_config(all);
        Ok(Value::Object(normalized))
    }

    pub fn get_groups(&self) -> Result<Value, String> {
        let all = self.repository.get_all_settings()?;
        let normalized = self.normalize_config(all);
        Ok(split_config_groups(&normalized))
    }
}

impl ConfigProvider for ConfigService {
    fn get_force_region_ip(&self) -> String {
        let all = self.repository.get_all_settings().unwrap_or_default();
        let normalized = self.normalize_config(all);
        normalized
            .get("force_region_ip")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string()
    }

    fn get_streaming_config(&self) -> StreamingConfigSnapshot {
        let all = self.repository.get_all_settings().unwrap_or_default();
        let normalized = self.normalize_config(all);

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
