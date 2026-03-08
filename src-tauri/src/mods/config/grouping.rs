use serde_json::{json, Map, Value};

fn pick_group_values(config: &Map<String, Value>, keys: &[&str]) -> Value {
    let mut group = Map::new();
    for key in keys {
        if let Some(value) = config.get(*key) {
            group.insert((*key).to_string(), value.clone());
        }
    }
    Value::Object(group)
}

pub fn split_config_groups(config: &Map<String, Value>) -> Value {
    // 分组键保持与 Electron 端一致，便于 renderer 逻辑无差异复用。
    json!({
        "app": pick_group_values(config, &["locale", "fullscreen", "background_keepalive", "use_vulkan"]),
        "streaming": pick_group_values(config, &[
            "resolution",
            "use_msal",
            "force_region_ip",
            "audio_bitrate_mode",
            "audio_bitrate",
            "enable_audio_control",
            "preferred_game_language",
            "codec",
            "video_format",
            "ipv6",
            "performance_style",
            "stream_runtime_mode",
            "display_options"
        ]),
        "host": pick_group_values(config, &[
            "xhome_bitrate_mode",
            "xhome_bitrate",
            "xhome_turn_fallback",
            "power_on",
            "server_url",
            "server_username",
            "server_credential"
        ]),
        "xcloud": pick_group_values(config, &["xcloud_bitrate_mode", "xcloud_bitrate"]),
        "input": pick_group_values(config, &["polling_rate", "vibration"])
    })
}
