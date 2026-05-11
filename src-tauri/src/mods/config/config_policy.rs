use serde_json::{Map, Value};

use super::defaults::{allowed_key, default_config_map};
use super::grouping::split_config_groups;

fn normalize_number(key: &str, value: &Value, fallback: i64, min: i64, max: i64) -> Value {
    let parsed = if let Some(val) = value.as_i64() {
        val.clamp(min, max)
    } else if let Some(val) = value.as_f64() {
        (val.round() as i64).clamp(min, max)
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
        default_map["sharpness"].as_i64().unwrap_or(2)
    };
    let saturation = if let Some(val) = map.get("saturation").and_then(Value::as_i64) {
        val.clamp(0, 200)
    } else {
        default_map["saturation"].as_i64().unwrap_or(100)
    };
    let contrast = if let Some(val) = map.get("contrast").and_then(Value::as_i64) {
        val.clamp(0, 200)
    } else {
        default_map["contrast"].as_i64().unwrap_or(100)
    };
    let brightness = if let Some(val) = map.get("brightness").and_then(Value::as_i64) {
        val.clamp(0, 200)
    } else {
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
        | "theme"
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
                Value::from(fallback.as_str().unwrap_or(""))
            }
        }
        "fullscreen"
        | "xhome_turn_fallback"
        | "enable_audio_control"
        | "vibration"
        | "power_on"
        | "ipv6"
        | "performance_style"
        | "background_keepalive"
        | "use_vulkan"
        | "ui_haptics"
        | "ui_audio"
        | "debug" => {
            if let Some(val) = value.as_bool() {
                Value::from(val)
            } else {
                Value::from(fallback.as_bool().unwrap_or(false))
            }
        }
        "resolution" => normalize_number(key, value, fallback.as_i64().unwrap_or(720), 720, 1440),
        "xhome_resolution" => {
            normalize_number(key, value, fallback.as_i64().unwrap_or(720), 720, 1081)
        }
        "xhome_bitrate" | "xcloud_bitrate" | "audio_bitrate" => {
            normalize_number(key, value, fallback.as_i64().unwrap_or(20), 0, 200)
        }
        "polling_rate" => normalize_number(key, value, fallback.as_i64().unwrap_or(250), 1, 1000),
        "vibration_strength" => normalize_string_enum(
            key,
            value,
            fallback.as_str().unwrap_or("realistic"),
            &["realistic", "enhanced", "full"],
        ),
        "xhome_bitrate_mode" | "xcloud_bitrate_mode" | "audio_bitrate_mode" => {
            normalize_string_enum(
                key,
                value,
                fallback.as_str().unwrap_or("Auto"),
                &["Auto", "Custom"],
            )
        }
        "stream_runtime_mode" => normalize_string_enum(
            key,
            value,
            fallback.as_str().unwrap_or("webrtc-direct"),
            &["webrtc-direct", "rust-owned"],
        ),
        "runtime_trace_mode" => normalize_string_enum(
            key,
            value,
            fallback.as_str().unwrap_or("minimal"),
            &["off", "minimal", "standard", "verbose", "trace"],
        ),
        "display_options" => normalize_display_options(key, value, fallback),
        "gamepad_device_profiles" => {
            if value.is_array() {
                value.clone()
            } else {
                fallback.clone()
            }
        }
        "gamepad_keyboard_mapping" => {
            if value.is_object() {
                value.clone()
            } else {
                fallback.clone()
            }
        }
        _ => fallback.clone(),
    }
}

pub fn normalize_config(source: Map<String, Value>) -> Map<String, Value> {
    let defaults = default_config_map();
    let mut normalized = Map::new();

    for (key, fallback) in &defaults {
        let candidate = source.get(key).unwrap_or(fallback);
        normalized.insert(key.to_string(), normalize_value(key, candidate, fallback));
    }

    normalized
}

pub fn filter_patch(patch: &Map<String, Value>) -> Map<String, Value> {
    let mut filtered_patch = Map::new();
    for (key, value) in patch {
        if allowed_key(key) {
            filtered_patch.insert(key.to_string(), value.clone());
        }
    }
    filtered_patch
}

pub fn split_groups(config: &Map<String, Value>) -> Value {
    split_config_groups(config)
}
