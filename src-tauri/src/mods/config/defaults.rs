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
    "polling_rate",
    "vibration",
    "vibration_strength",
    "power_on",
    "video_format",
    "ipv6",
    "performance_style",
    "stream_runtime_mode",
    "server_url",
    "server_username",
    "server_credential",
    "background_keepalive",
    "display_options",
    "super_resolution_experimental",
    "use_vulkan",
    "ui_haptics",
    "ui_audio",
    "debug",
    "runtime_trace_mode",
    "gamepad_device_profiles",
    "gamepad_keyboard_mapping",
];

pub fn default_config_map() -> Map<String, Value> {
    let runtime_trace_mode_default = if cfg!(debug_assertions) {
        json!("minimal")
    } else {
        json!("off")
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
        "polling_rate": 250,
        "vibration": true,
        "vibration_strength": "realistic",
        "power_on": false,
        "video_format": "",
        "ipv6": false,
        "performance_style": false,
        "stream_runtime_mode": "webrtc-direct",
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
        "use_vulkan": false,
        "ui_haptics": true,
        "ui_audio": true,
        "debug": false,
        "runtime_trace_mode": null
        ,
        "gamepad_device_profiles": [],
        "gamepad_keyboard_mapping": {
            "bindings": []
        }
    });

    let mut map = value.as_object().cloned().unwrap_or_default();
    map.insert("runtime_trace_mode".to_string(), runtime_trace_mode_default);
    map
}

pub fn allowed_key(key: &str) -> bool {
    APP_CONFIG_KEYS.contains(&key)
}
