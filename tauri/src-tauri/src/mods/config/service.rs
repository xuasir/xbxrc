use serde_json::{Map, Value};

use super::defaults::{allowed_key, default_config_map};
use super::grouping::split_config_groups;
use super::repository::ConfigRepository;

pub struct ConfigService {
    repository: ConfigRepository,
}

impl ConfigService {
    pub fn new(repository: ConfigRepository) -> Self {
        Self { repository }
    }

    fn normalize_number(value: &Value, fallback: i64, min: i64, max: i64) -> Value {
        let parsed = value.as_i64().unwrap_or(fallback).clamp(min, max);
        Value::from(parsed)
    }

    fn normalize_string_enum(value: &Value, fallback: &str, allowed: &[&str]) -> Value {
        if let Some(raw) = value.as_str() {
            if allowed.contains(&raw) {
                return Value::from(raw.to_string());
            }
        }
        Value::from(fallback.to_string())
    }

    fn normalize_display_options(value: &Value, fallback: &Value) -> Value {
        let Some(map) = value.as_object() else {
            return fallback.clone();
        };

        let Some(default_map) = fallback.as_object() else {
            return fallback.clone();
        };

        let sharpness = map
            .get("sharpness")
            .and_then(Value::as_i64)
            .unwrap_or(default_map["sharpness"].as_i64().unwrap_or(2))
            .clamp(0, 10);
        let saturation = map
            .get("saturation")
            .and_then(Value::as_i64)
            .unwrap_or(default_map["saturation"].as_i64().unwrap_or(100))
            .clamp(0, 200);
        let contrast = map
            .get("contrast")
            .and_then(Value::as_i64)
            .unwrap_or(default_map["contrast"].as_i64().unwrap_or(100))
            .clamp(0, 200);
        let brightness = map
            .get("brightness")
            .and_then(Value::as_i64)
            .unwrap_or(default_map["brightness"].as_i64().unwrap_or(100))
            .clamp(0, 200);

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
            | "server_credential" => Value::from(
                value
                    .as_str()
                    .unwrap_or_else(|| fallback.as_str().unwrap_or("")),
            ),
            "use_msal"
            | "fullscreen"
            | "xhome_turn_fallback"
            | "enable_audio_control"
            | "vibration"
            | "power_on"
            | "ipv6"
            | "performance_style"
            | "background_keepalive"
            | "use_vulkan" => Value::from(
                value
                    .as_bool()
                    .unwrap_or(fallback.as_bool().unwrap_or(false)),
            ),
            "resolution" => {
                Self::normalize_number(value, fallback.as_i64().unwrap_or(720), 720, 1081)
            }
            "xhome_bitrate" | "xcloud_bitrate" | "audio_bitrate" => {
                Self::normalize_number(value, fallback.as_i64().unwrap_or(20), 0, 200)
            }
            "polling_rate" => {
                Self::normalize_number(value, fallback.as_i64().unwrap_or(250), 1, 1000)
            }
            "xhome_bitrate_mode" | "xcloud_bitrate_mode" | "audio_bitrate_mode" => {
                Self::normalize_string_enum(
                    value,
                    fallback.as_str().unwrap_or("Auto"),
                    &["Auto", "Custom"],
                )
            }
            "stream_runtime_mode" => Self::normalize_string_enum(
                value,
                fallback.as_str().unwrap_or("webrtc-direct"),
                &["webrtc-direct", "rust-owned"],
            ),
            "display_options" => Self::normalize_display_options(value, fallback),
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
