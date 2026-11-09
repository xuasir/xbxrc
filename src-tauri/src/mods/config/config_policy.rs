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

    // 独立锐化只开放柔和(1)与清晰(3)，关闭状态由 render_mode 表达。
    let sharpness = match map.get("sharpness").and_then(Value::as_i64) {
        Some(1..=2) => 1,
        _ => 3,
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

fn normalize_game_controls(value: &Value) -> Value {
    const BUTTONS: [&str; 17] = [
        "south",
        "east",
        "west",
        "north",
        "l1",
        "r1",
        "l2",
        "r2",
        "view",
        "menu",
        "l3",
        "r3",
        "dpadUp",
        "dpadDown",
        "dpadLeft",
        "dpadRight",
        "home",
    ];
    let mapping = value.get("buttons").and_then(Value::as_object);
    let sources: Vec<&str> = BUTTONS
        .iter()
        .map(|key| {
            mapping
                .and_then(|map| map.get(*key))
                .map_or(Some(*key), Value::as_str)
                .unwrap_or("")
        })
        .collect();
    let valid = sources.iter().all(|key| BUTTONS.contains(key))
        && sources
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len()
            == BUTTONS.len()
        && sources[16] == "home"
        && ["l2", "r2"].contains(&sources[6])
        && ["l2", "r2"].contains(&sources[7]);
    let buttons: Map<String, Value> = BUTTONS
        .iter()
        .enumerate()
        .map(|(index, key)| {
            (
                (*key).to_owned(),
                Value::from(if valid { sources[index] } else { *key }),
            )
        })
        .collect();
    let deadzone = |key: &str| {
        value
            .get(key)
            .and_then(Value::as_f64)
            .filter(|value| value.is_finite())
            .unwrap_or(0.1)
            .clamp(0.0, 0.3)
    };
    serde_json::json!({
        "buttons": buttons,
        "leftDeadzone": deadzone("leftDeadzone"),
        "rightDeadzone": deadzone("rightDeadzone"),
        "invertRightY": value.get("invertRightY").and_then(Value::as_bool).unwrap_or(false),
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
        | "super_resolution_experimental"
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
            normalize_number(key, value, fallback.as_i64().unwrap_or(720), 720, 1440)
        }
        "xhome_bitrate" | "xcloud_bitrate" | "audio_bitrate" => {
            normalize_number(key, value, fallback.as_i64().unwrap_or(20), 0, 200)
        }
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
        "runtime_trace_mode" => normalize_runtime_trace_mode(value, fallback),
        "display_options" => normalize_display_options(key, value, fallback),
        "render_mode" => normalize_string_enum(key, value, "video", &["video", "sharpen", "sr"]),
        "gamepad_background_policy" => {
            normalize_string_enum(key, value, "keep-active", &["keep-active", "follow-app"])
        }
        "gamepad_control_source" => {
            normalize_string_enum(key, value, "gamepad", &["gamepad", "keyboard"])
        }
        "gamepad_game_controls" => normalize_game_controls(value),
        "gamepad_keyboard_mapping" => normalize_keyboard_mapping(value, fallback),
        _ => fallback.clone(),
    }
}

pub fn normalize_config(source: Map<String, Value>) -> Map<String, Value> {
    // 在注入默认值之前迁移旧意图，明确关闭拥有完整覆盖效力。
    let render_mode = source
        .get("render_mode")
        .and_then(Value::as_str)
        .filter(|mode| ["video", "sharpen", "sr"].contains(mode))
        .unwrap_or_else(|| {
            if source
                .get("super_resolution_experimental")
                .and_then(Value::as_bool)
                == Some(true)
            {
                "sr"
            } else if source
                .get("display_options")
                .and_then(|value| value.get("sharpness"))
                .and_then(Value::as_i64)
                .is_some_and(|sharpness| sharpness > 0)
            {
                "sharpen"
            } else {
                "video"
            }
        });
    let defaults = default_config_map();
    let mut normalized = Map::new();

    for (key, fallback) in &defaults {
        let candidate = source.get(key).unwrap_or(fallback);
        normalized.insert(key.to_string(), normalize_value(key, candidate, fallback));
    }

    normalized.insert("render_mode".to_string(), Value::from(render_mode));
    normalized.insert(
        "super_resolution_experimental".to_string(),
        Value::from(render_mode == "sr"),
    );
    normalized
}

pub fn filter_patch(patch: &Map<String, Value>) -> Map<String, Value> {
    let mut filtered_patch = Map::new();
    for (key, value) in patch {
        if allowed_key(key) {
            filtered_patch.insert(key.to_string(), value.clone());
        }
    }
    // 兼容旧入口的写入；新入口以一个 patch 同时提交模式与完整画面配置。
    if !patch.contains_key("render_mode") {
        let legacy_mode = if let Some(enabled) = patch
            .get("super_resolution_experimental")
            .and_then(Value::as_bool)
        {
            Some(if enabled { "sr" } else { "video" })
        } else {
            patch
                .get("display_options")
                .and_then(|value| value.get("sharpness"))
                .and_then(Value::as_i64)
                .map(|sharpness| if sharpness > 0 { "sharpen" } else { "video" })
        };
        if let Some(mode) = legacy_mode {
            filtered_patch.insert("render_mode".to_string(), Value::from(mode));
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
    fn render_controls_migrate_raw_legacy_intent_and_preserve_explicit_mode() {
        for (source, mode, sharpness) in [
            (json!({}), "video", 3),
            (json!({"display_options": {"sharpness": 0}}), "video", 3),
            (json!({"display_options": {"sharpness": 2}}), "sharpen", 1),
            (json!({"display_options": {"sharpness": 3}}), "sharpen", 3),
            (json!({"display_options": {"sharpness": 10}}), "sharpen", 3),
            (
                json!({"super_resolution_experimental": true, "display_options": {"sharpness": 3}}),
                "sr",
                3,
            ),
            (
                json!({"render_mode": "video", "super_resolution_experimental": true, "display_options": {"sharpness": 3}}),
                "video",
                3,
            ),
            (
                json!({"render_mode": "sharpen", "super_resolution_experimental": true, "display_options": {"sharpness": 1}}),
                "sharpen",
                1,
            ),
        ] {
            let normalized = normalize_config(source.as_object().unwrap().clone());
            assert_eq!(normalized["render_mode"], mode);
            assert_eq!(normalized["display_options"]["sharpness"], sharpness);
            assert_eq!(normalized["super_resolution_experimental"], mode == "sr");
        }
    }

    #[test]
    fn render_patch_keeps_new_mode_authoritative_and_adapts_legacy_writes() {
        let patch = |value: Value| filter_patch(value.as_object().unwrap());
        assert_eq!(
            patch(json!({"super_resolution_experimental": true}))["render_mode"],
            "sr"
        );
        assert_eq!(
            patch(json!({"super_resolution_experimental": false}))["render_mode"],
            "video"
        );
        assert_eq!(
            patch(json!({"display_options": {"sharpness": 3}}))["render_mode"],
            "sharpen"
        );
        let video = patch(
            json!({"render_mode": "video", "display_options": {"sharpness": 3}, "super_resolution_experimental": true}),
        );
        assert_eq!(video["render_mode"], "video");
        let normalized = normalize_config(video);
        assert_eq!(normalized["super_resolution_experimental"], false);
        assert_eq!(normalized["display_options"]["sharpness"], 3);
    }

    #[test]
    fn game_controls_enforce_bijection_analog_roles_and_bounds() {
        let valid = normalize_game_controls(&json!({
            "buttons": {"south": "east", "east": "south", "l2": "r2", "r2": "l2"},
            "leftDeadzone": -1, "rightDeadzone": 2, "invertRightY": true
        }));
        assert_eq!(valid["buttons"]["south"], "east");
        assert_eq!(valid["buttons"]["l2"], "r2");
        assert_eq!(valid["leftDeadzone"], 0.0);
        assert_eq!(valid["rightDeadzone"], 0.3);
        assert_eq!(valid["invertRightY"], true);
        for buttons in [
            json!({"south": "east"}),
            json!({"south": "home", "home": "south"}),
            json!({"south": "l2", "l2": "south"}),
            json!({"south": 7}),
        ] {
            let normalized = normalize_game_controls(&json!({"buttons": buttons}));
            assert_eq!(normalized["buttons"]["south"], "south");
            assert_eq!(normalized["buttons"]["home"], "home");
        }
    }

    #[test]
    fn gamepad_config_projects_active_keys_and_valid_preferences() {
        let profiles =
            json!([{"id": "one", "buttons": {"south": 1}}, {"id": "two", "unknown": 42}]);
        let mut source = Map::new();
        source.insert("gamepad_device_profiles".into(), profiles.clone());
        source.insert("gamepad_background_policy".into(), json!("follow-app"));
        source.insert("ui_haptics".into(), json!(true));
        let normalized = normalize_config(source);
        assert!(!normalized.contains_key("gamepad_device_profiles"));
        assert!(!filter_patch(
            &json!({"gamepad_device_profiles": profiles})
                .as_object()
                .unwrap()
        )
        .contains_key("gamepad_device_profiles"));
        assert_eq!(normalized["gamepad_background_policy"], "follow-app");
        assert_eq!(normalized["ui_haptics"], true);
        assert!(!normalized.contains_key("native_gamepad_preview"));
    }

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

    #[test]
    fn xhome_resolution_accepts_1440_and_clamps_upper_bound() {
        let mut source = Map::new();
        source.insert("xhome_resolution".to_owned(), json!(1440));
        assert_eq!(normalize_config(source)["xhome_resolution"], 1440);

        let mut source = Map::new();
        source.insert("xhome_resolution".to_owned(), json!(2160));
        assert_eq!(normalize_config(source)["xhome_resolution"], 1440);
    }
}
