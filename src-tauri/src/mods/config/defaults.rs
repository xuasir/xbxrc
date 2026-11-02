use serde_json::{json, Map, Value};

pub const APP_CONFIG_KEYS: [&str; 37] = [
    "locale",
    "theme",
    "fullscreen",
    "resolution",
    "xhome_resolution",
    "xhome_bitrate_mode",
    "xhome_bitrate",
    "xhome_turn_fallback",
    "xcloud_bitrate_mode",
    "xcloud_bitrate",
    "audio_bitrate_mode",
    "audio_bitrate",
    "enable_audio_control",
    "preferred_game_language",
    "force_region_ip",
    "codec",
    "vibration",
    "vibration_strength",
    "power_on",
    "video_format",
    "ipv6",
    "performance_style",
    "server_url",
    "server_username",
    "server_credential",
    "background_keepalive",
    "display_options",
    "super_resolution_experimental",
    "ui_haptics",
    "ui_audio",
    "debug",
    "runtime_trace_mode",
    "runtime_trace_dimensions",
    "gamepad_background_policy",
    "gamepad_control_source",
    "gamepad_game_controls",
    "gamepad_keyboard_mapping",
];

pub fn default_config_map() -> Map<String, Value> {
    let runtime_trace_mode_default = if cfg!(debug_assertions) {
        json!("dev")
    } else {
        json!("production")
    };

    let value = json!({
        "locale": "en",
        "theme": "dark",
        "fullscreen": false,
        "resolution": 720,
        "xhome_resolution": 1080,
        "xhome_bitrate_mode": "Auto",
        "xhome_bitrate": 20,
        "xhome_turn_fallback": false,
        "xcloud_bitrate_mode": "Auto",
        "xcloud_bitrate": 20,
        "audio_bitrate_mode": "Auto",
        "audio_bitrate": 20,
        "enable_audio_control": false,
        "preferred_game_language": "en-US",
        "force_region_ip": "",
        "codec": "",
        "vibration": true,
        "vibration_strength": "realistic",
        "power_on": false,
        "video_format": "",
        "ipv6": false,
        "performance_style": false,
        "server_url": "",
        "server_username": "",
        "server_credential": "",
        "background_keepalive": false,
        "display_options": {
            "sharpness": 0,
            "saturation": 100,
            "contrast": 100,
            "brightness": 100
        },
        "super_resolution_experimental": false,
        "ui_haptics": false,
        "ui_audio": true,
        "debug": false,
        "runtime_trace_mode": null,
        "runtime_trace_dimensions": "",
        "gamepad_background_policy": "keep-active",
        "gamepad_control_source": "gamepad",
        "gamepad_game_controls": { "buttons": {}, "leftDeadzone": 0.1, "rightDeadzone": 0.1, "invertRightY": false },
        "gamepad_keyboard_mapping": default_keyboard_mapping_value()
    });

    let mut map = value.as_object().cloned().unwrap_or_default();
    map.insert("runtime_trace_mode".to_string(), runtime_trace_mode_default);
    map
}

pub fn default_keyboard_mapping_value() -> Value {
    serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../src/shared/gamepad/keyboard-defaults.json"
    )))
    .expect("valid shared keyboard defaults")
}

pub fn allowed_key(key: &str) -> bool {
    APP_CONFIG_KEYS.contains(&key)
}
