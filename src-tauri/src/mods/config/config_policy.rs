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

fn normalize_runtime_trace_mode(value: &Value, fallback: &Value) -> Value {
    let fallback_profile = crate::mods::runtime_trace::RuntimeTraceProfile::from_stored_mode(
        fallback.as_str().unwrap_or("production"),
    );
    let profile = value
        .as_str()
        .map(crate::mods::runtime_trace::RuntimeTraceProfile::from_stored_mode)
        .unwrap_or(fallback_profile);
    Value::from(profile.as_str().to_string())
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

fn normalize_keyboard_mapping(value: &Value, fallback: &Value) -> Value {
    let Some(map) = value.as_object() else {
        return fallback.clone();
    };
    let has_bindings = map
        .get("bindings")
        .and_then(Value::as_array)
        .map(|bindings| !bindings.is_empty())
        .unwrap_or(false);
    if has_bindings {
        value.clone()
    } else {
        fallback.clone()
    }
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
        | "server_credential"
        | "runtime_trace_dimensions" => {
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
        | "gamepad_cold_start_sdl_binding_nudge"
        | "super_resolution_experimental"
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
        "runtime_trace_mode" => normalize_runtime_trace_mode(value, fallback),
        "display_options" => normalize_display_options(key, value, fallback),
        "gamepad_device_profiles" => {
            if value.is_array() {
                value.clone()
            } else {
                fallback.clone()
            }
        }
        "gamepad_keyboard_mapping" => normalize_keyboard_mapping(value, fallback),
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn default_keyboard_mapping_is_non_empty() {
        let defaults = default_config_map();
        let bindings = defaults["gamepad_keyboard_mapping"]["bindings"]
            .as_array()
            .expect("default keyboard bindings");
        assert!(!bindings.is_empty());
    }

    #[test]
    fn normalize_config_replaces_empty_keyboard_mapping_with_default() {
        let mut source = Map::new();
        source.insert(
            "gamepad_keyboard_mapping".to_owned(),
            json!({
                "bindings": []
            }),
        );

        let normalized = normalize_config(source);
        let bindings = normalized["gamepad_keyboard_mapping"]["bindings"]
            .as_array()
            .expect("normalized keyboard bindings");
        assert!(!bindings.is_empty());
    }

    #[test]
    fn normalize_runtime_trace_mode_keeps_legacy_values_compatible() {
        let mut source = Map::new();
        source.insert("runtime_trace_mode".to_owned(), json!("verbose"));
        source.insert(
            "runtime_trace_dimensions".to_owned(),
            json!("network,recovery,-input"),
        );

        let normalized = normalize_config(source);

        assert_eq!(normalized["runtime_trace_mode"], "dev");
        assert_eq!(
            normalized["runtime_trace_dimensions"],
            "network,recovery,-input"
        );
    }
}
