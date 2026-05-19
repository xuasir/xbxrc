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
    // 分组键保持与 renderer 端一致；streaming 组明确承载 policy + view 配置。
    json!({
        "app": pick_group_values(config, &["locale", "theme", "fullscreen", "background_keepalive", "use_vulkan", "ui_haptics", "ui_audio", "debug", "runtime_trace_mode"]),
        "streaming": pick_group_values(config, &[
            "resolution",
            "xhome_resolution",
            "force_region_ip",
            "xhome_bitrate_mode",
            "xhome_bitrate",
            "xcloud_bitrate_mode",
            "xcloud_bitrate",
            "audio_bitrate_mode",
            "audio_bitrate",
            "enable_audio_control",
            "preferred_game_language",
            "codec",
            "video_format",
            "ipv6",
            "power_on",
            "stream_runtime_mode",
            "server_url",
            "server_username",
            "server_credential",
            "xhome_turn_fallback",
            "performance_style",
            "display_options",
            "super_resolution_experimental"
        ]),
        "host": pick_group_values(config, &[]),
        // xcloud 组当前不承载策略字段，避免与 streaming policy 分组重复返回。
        "xcloud": pick_group_values(config, &[]),
        "input": pick_group_values(config, &[
            "polling_rate",
            "vibration",
            "vibration_strength",
            "gamepad_device_profiles",
            "gamepad_keyboard_mapping",
            "gamepad_cold_start_sdl_binding_nudge",
            "gamepad_fse_gate_fallback_nudge"
        ])
    })
}
