use serde_json::{json, Map, Value};

pub const APP_CONFIG_KEYS: [&str; 34] = [
    "locale",
    "theme",
    "use_msal",
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
    "use_vulkan",
    "ui_haptics",
    "ui_audio",
    "debug",
];

pub fn default_config_map() -> Map<String, Value> {
    let value = json!({
        "locale": "en",
        "theme": "dark",
        "use_msal": false,
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
            "sharpness": 2,
            "saturation": 100,
            "contrast": 100,
            "brightness": 100
        },
        "use_vulkan": false,
        "ui_haptics": true,
        "ui_audio": true,
        "debug": false
    });

    value.as_object().cloned().unwrap_or_default()
}

pub fn allowed_key(key: &str) -> bool {
    APP_CONFIG_KEYS.contains(&key)
}
