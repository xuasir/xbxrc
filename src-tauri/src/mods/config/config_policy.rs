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

    // 锐化独立于 SR 和兼容引擎，0/1/3 分别表示关闭/柔和/清晰。
    let sharpness = match map.get("sharpness").and_then(Value::as_i64) {
        Some(1..=2) => 1,
        Some(3..) => 3,
        _ => 0,
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
    let valid_key = |key: &str| {
        key.strip_prefix("key")
            .is_some_and(|suffix| suffix.len() == 1 && suffix.as_bytes()[0].is_ascii_uppercase())
            || key
                .strip_prefix("digit")
                .is_some_and(|suffix| suffix.len() == 1 && suffix.as_bytes()[0].is_ascii_digit())
            || [
                "enter",
                "tab",
                "escape",
                "space",
                "arrowUp",
                "arrowDown",
                "arrowLeft",
                "arrowRight",
            ]
            .contains(&key)
    };
    let valid_control = |control: &str| {
        [
            "south",
            "east",
            "west",
            "north",
            "l1",
            "r1",
            "l2",
            "r2",
            "l3",
            "r3",
            "view",
            "menu",
            "home",
            "dpadUp",
            "dpadDown",
            "dpadLeft",
            "dpadRight",
            "leftStickUp",
            "leftStickDown",
            "leftStickLeft",
            "leftStickRight",
            "rightStickUp",
            "rightStickDown",
            "rightStickLeft",
            "rightStickRight",
        ]
        .contains(&control)
    };
    let mut keys = std::collections::HashSet::new();
    let bindings: Vec<Value> = value
        .get("bindings")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(128)
        .filter_map(|binding| {
            let key = binding.get("key")?.as_str()?;
            let control = binding.get("control")?.as_str()?;
            if !valid_key(key) || !valid_control(control) || !keys.insert(key.to_owned()) {
                return None;
            }
            Some(serde_json::json!({"key":key,"control":control}))
        })
        .collect();
    let mut targets = std::collections::HashSet::new();
    let chords: Vec<Value> = value
        .get("chords")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(25)
        .filter_map(|binding| {
            let control = binding.get("control")?.as_str()?;
            let sources = binding.get("keys")?.as_array()?;
            if !valid_control(control)
                || targets.contains(control)
                || !(2..=4).contains(&sources.len())
            {
                return None;
            }
            let mut combination = Vec::new();
            for source in sources {
                let key = source.as_str().filter(|key| valid_key(key))?;
                if combination.contains(&key) {
                    return None;
                }
                combination.push(key);
            }
            combination.sort();
            if !keys.insert(combination.join(",")) {
                return None;
            }
            targets.insert(control.to_owned());
            Some(serde_json::json!({"keys":sources,"control":control}))
        })
        .collect();
    if bindings.is_empty() && chords.is_empty() {
        return fallback.clone();
    }
    let mut result = serde_json::json!({"bindings":bindings});
    if !chords.is_empty() {
        result["chords"] = Value::Array(chords);
    }
    result
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
    // 与 TS 解析共用来源/目标和预算约束；按布局隔离独立背键。
    let mut seen = std::collections::HashSet::new();
    let extra_buttons: Vec<Value> = value
        .get("extraButtons")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(128)
        .filter_map(|binding| {
            let layout = binding.get("layout")?.as_str()?;
            let button = binding.get("button")?.as_u64()?;
            let target = binding.get("target")?.as_str()?;
            if layout.is_empty()
                || layout.encode_utf16().count() > 2048
                || !(17..128).contains(&button)
                || !BUTTONS.contains(&target)
                || ["l2", "r2"].contains(&target)
                || !seen.insert((layout.to_owned(), button))
            {
                return None;
            }
            Some(serde_json::json!({"layout": layout, "button": button, "target": target}))
        })
        .collect();
    let device_mappings = normalize_device_mappings(value.get("deviceMappings"), &BUTTONS);
    serde_json::json!({
        "buttons": buttons,
        "leftDeadzone": deadzone("leftDeadzone"),
        "rightDeadzone": deadzone("rightDeadzone"),
        "invertLeftY": value.get("invertLeftY").and_then(Value::as_bool).unwrap_or(false),
        "invertRightY": value.get("invertRightY").and_then(Value::as_bool).unwrap_or(false),
        "extraButtons": extra_buttons,
        "deviceMappings": device_mappings,
    })
}

fn normalize_device_mappings(value: Option<&Value>, buttons: &[&str]) -> Vec<Value> {
    const AXES: [&str; 4] = ["leftStickX", "leftStickY", "rightStickX", "rightStickY"];
    let mut layouts = std::collections::HashSet::new();
    value.and_then(Value::as_array).into_iter().flatten().take(64).filter_map(|profile| {
        let layout = profile.get("layout")?.as_str()?;
        let bindings = profile.get("bindings")?.as_array()?;
        if layout.is_empty() || layout.encode_utf16().count() > 2048 || layouts.contains(layout) { return None; }
        let mut targets = std::collections::HashSet::new();
        let mut combinations = std::collections::HashSet::new();
        let bindings: Vec<Value> = bindings.iter().take(21).filter_map(|binding| {
            let target = binding.get("target")?.as_str()?;
            if (!buttons.contains(&target) && !AXES.contains(&target)) || targets.contains(target) { return None; }
            let normalized = match binding.get("kind")?.as_str()? {
                "buttons" => {
                    let sources = binding.get("buttons")?.as_array()?;
                    let mode = if binding.get("mode").and_then(Value::as_str) == Some("any") { "any" } else { "all" };
                    if !buttons.contains(&target) || sources.is_empty() || sources.len() > if mode == "any" { 128 } else { 4 } { return None; }
                    let mut indices = Vec::new();
                    for source in sources {
                        let index = source.as_u64().filter(|index| *index < 128)?;
                        if indices.contains(&index) { return None; }
                        indices.push(index);
                    }
                    indices.sort();
                    if mode == "all" && indices.len() > 1 && !combinations.insert(indices) { return None; }
                    serde_json::json!({"target":target,"kind":"buttons","buttons":sources,"mode":mode})
                }
                "axis" => {
                    let axis = binding.get("axis")?.as_u64().filter(|axis| *axis < 64)?;
                    let range = if AXES.contains(&target) { "signed" } else if binding.get("range").and_then(Value::as_str) == Some("full") { "full" } else { "positive" };
                    serde_json::json!({"target":target,"kind":"axis","axis":axis,"invert":binding.get("invert").and_then(Value::as_bool).unwrap_or(false),"range":range})
                }
                _ => return None,
            };
            targets.insert(target.to_owned());
            Some(normalized)
        }).collect();
        layouts.insert(layout.to_owned());
        Some(serde_json::json!({"layout":layout,"bindings":bindings}))
    }).collect()
}

fn normalize_value(key: &str, value: &Value, fallback: &Value) -> Value {
    match key {
        "locale"
        | "theme"
        | "preferred_game_language"
        | "force_region_ip"
        | "xcloud_region_name"
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
        | "xcloud_region_migrated"
        | "prevent_resolution_drops"
        | "enable_audio_control"
        | "vibration"
        | "power_on"
        | "ipv6"
        | "performance_style"
        | "background_keepalive"
        | "super_resolution_experimental"
        | "render_compatibility_mode"
        | "ui_haptics"
        | "ui_audio"
        | "debug"
        | "diagnostics_lightweight" => {
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
        "gamepad_background_policy" => {
            normalize_string_enum(key, value, "keep-active", &["keep-active", "follow-app"])
        }
        "gamepad_control_source" => {
            normalize_string_enum(key, value, "gamepad", &["gamepad", "keyboard"])
        }
        "gamepad_game_controls" => normalize_game_controls(value),
        "vibration_channels" => {
            if !value.is_object() {
                return Value::Null;
            }
            ["leftGrip", "rightGrip", "leftTrigger", "rightTrigger"]
                .into_iter()
                .filter_map(|channel| {
                    let strength = value.get(channel)?.as_str()?;
                    ["realistic", "enhanced", "full"]
                        .contains(&strength)
                        .then(|| (channel.to_owned(), serde_json::json!(strength)))
                })
                .collect::<Map<String, Value>>()
                .into()
        }
        "stream_menu_shortcuts" => {
            let buttons = [
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
            let mut result = Map::new();
            for target in ["menu", "diagnostics"] {
                let binding = value.get(target).and_then(Value::as_array);
                let valid = binding.is_some_and(|keys| {
                    keys.len() <= 4
                        && keys
                            .iter()
                            .all(|key| key.as_str().is_some_and(|key| buttons.contains(&key)))
                        && keys.iter().collect::<std::collections::HashSet<_>>().len() == keys.len()
                });
                result.insert(
                    target.into(),
                    if valid {
                        serde_json::json!(binding.unwrap())
                    } else {
                        fallback[target].clone()
                    },
                );
            }
            result.into()
        }
        "gamepad_keyboard_mapping" => normalize_keyboard_mapping(value, fallback),
        _ => fallback.clone(),
    }
}

pub fn normalize_config(source: Map<String, Value>) -> Map<String, Value> {
    // 原始兼容键区分新合同与旧模式，迁移后独立保存每项效果。
    let legacy = !source.contains_key("render_compatibility_mode");
    let legacy_mode = source.get("render_mode").and_then(Value::as_str);
    let sr = if legacy {
        match legacy_mode {
            Some("video" | "sharpen") => false,
            Some("sr") => true,
            _ => source
                .get("super_resolution_experimental")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        }
    } else {
        source
            .get("super_resolution_experimental")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    };
    let defaults = default_config_map();
    let mut normalized = Map::new();

    for (key, fallback) in &defaults {
        let candidate = source.get(key).unwrap_or(fallback);
        normalized.insert(key.to_string(), normalize_value(key, candidate, fallback));
    }

    normalized.insert("super_resolution_experimental".to_string(), Value::from(sr));
    if legacy && (matches!(legacy_mode, Some("video" | "sr")) || (legacy_mode.is_none() && sr)) {
        normalized["display_options"]["sharpness"] = Value::from(0);
    }
    normalized
}

pub fn cloud_region_migration(
    raw: &Map<String, Value>,
    regions: &[xbox_streaming::policy::types::Region],
) -> Option<String> {
    if raw.get("xcloud_region_migrated").and_then(Value::as_bool) == Some(true)
        || raw.contains_key("xcloud_region_name")
    {
        return None;
    }
    let legacy = raw
        .get("force_region_ip")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    Some(
        regions
            .iter()
            .find(|region| !legacy.is_empty() && region.base_uri.contains(legacy))
            .map(|region| region.name.clone())
            .unwrap_or_default(),
    )
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
    fn four_motor_strengths_and_stream_shortcuts_preserve_bounded_configuration() {
        let mut source = default_config_map();
        source.insert("vibration_channels".into(), json!({"leftGrip":"full", "rightGrip":"realistic", "leftTrigger":"enhanced", "rightTrigger":"invalid"}));
        source.insert(
            "stream_menu_shortcuts".into(),
            json!({"menu":["l1","r1"], "diagnostics":["home","home"]}),
        );
        let normalized = normalize_config(source);
        assert_eq!(
            normalized["vibration_channels"],
            json!({"leftGrip":"full", "rightGrip":"realistic", "leftTrigger":"enhanced"})
        );
        assert_eq!(
            normalized["stream_menu_shortcuts"],
            json!({"menu":["l1","r1"],"diagnostics":[]})
        );
        assert_eq!(normalize_config(normalized.clone()), normalized);
        assert_eq!(
            normalize_game_controls(&json!({"invertLeftY":true}))["invertLeftY"],
            true
        );
    }

    #[test]
    fn explicit_automatic_selection_and_migrated_preferences_survive_reload() {
        let regions = vec![xbox_streaming::policy::types::Region {
            name: "WestUS".into(),
            base_uri: "https://wus.example".into(),
            ..Default::default()
        }];
        let raw = serde_json::json!({"force_region_ip":"wus"});
        assert_eq!(
            cloud_region_migration(raw.as_object().unwrap(), &regions),
            Some("WestUS".into())
        );
        for raw in [
            serde_json::json!({"force_region_ip":"wus","xcloud_region_name":""}),
            serde_json::json!({"force_region_ip":"wus","xcloud_region_migrated":true}),
        ] {
            assert_eq!(
                cloud_region_migration(raw.as_object().unwrap(), &regions),
                None
            );
        }
        assert_eq!(
            cloud_region_migration(&Map::new(), &regions),
            Some(String::new())
        );
    }

    #[test]
    fn resolution_feedback_defaults_off_and_preserves_explicit_choices() {
        assert_eq!(
            normalize_config(Map::new())["prevent_resolution_drops"],
            false
        );
        for value in [true, false] {
            let raw = serde_json::json!({"prevent_resolution_drops": value});
            assert_eq!(
                normalize_config(raw.as_object().unwrap().clone())["prevent_resolution_drops"],
                value
            );
        }
    }

    #[test]
    fn render_controls_migrate_legacy_and_preserve_independent_effects() {
        for (source, sr, sharpness) in [
            (json!({}), false, 0),
            (json!({"display_options": {"sharpness": 0}}), false, 0),
            (json!({"display_options": {"sharpness": 2}}), false, 1),
            (json!({"display_options": {"sharpness": 10}}), false, 3),
            (
                json!({"render_mode": "video", "super_resolution_experimental": true, "display_options": {"sharpness": 3}}),
                false,
                0,
            ),
            (
                json!({"render_mode": "sr", "display_options": {"sharpness": 3}}),
                true,
                0,
            ),
            (
                json!({"render_compatibility_mode": false, "super_resolution_experimental": true, "display_options": {"sharpness": 3}}),
                true,
                3,
            ),
        ] {
            let normalized = normalize_config(source.as_object().unwrap().clone());
            assert_eq!(normalized["display_options"]["sharpness"], sharpness);
            assert_eq!(normalized["super_resolution_experimental"], sr);
            assert_eq!(normalized["render_compatibility_mode"], false);
            assert!(!normalized.contains_key("render_mode"));
        }
    }

    #[test]
    fn render_patch_preserves_combined_effects_and_compatibility_intent() {
        let patch = filter_patch(json!({"render_compatibility_mode": true, "super_resolution_experimental": true, "display_options": {"sharpness": 3, "brightness": 102}}).as_object().unwrap());
        let normalized = normalize_config(patch);
        assert_eq!(normalized["render_compatibility_mode"], true);
        assert_eq!(normalized["super_resolution_experimental"], true);
        assert_eq!(normalized["display_options"]["sharpness"], 3);
        assert_eq!(normalized["display_options"]["brightness"], 102);
        assert!(!filter_patch(
            json!({"super_resolution_experimental": false})
                .as_object()
                .unwrap()
        )
        .contains_key("display_options"));
    }

    #[test]
    fn device_mappings_normalize_raw_axes_chords_and_layout_isolation() {
        let controls = json!({"deviceMappings":[{"layout":"raw","bindings":[
            {"target":"home","kind":"buttons","buttons":[0,127],"mode":"all"},
            {"target":"south","kind":"buttons","buttons":[127,0],"mode":"all"},
            {"target":"east","kind":"buttons","buttons":[128]},
            {"target":"leftStickX","kind":"axis","axis":63,"invert":true,"range":"full"},
            {"target":"l2","kind":"axis","axis":4,"range":"full"},
            {"target":"r2","kind":"buttons","buttons":[4,5]},
            {"target":"west","kind":"axis","axis":64}
        ]},{"layout":"raw","bindings":[]},{"layout":"second","bindings":[]} ]});
        let result = normalize_game_controls(&controls);
        assert_eq!(result["deviceMappings"].as_array().unwrap().len(), 2);
        let bindings = result["deviceMappings"][0]["bindings"].as_array().unwrap();
        assert_eq!(bindings.len(), 4);
        assert_eq!(bindings[1]["range"], "signed");
        assert_eq!(bindings[2]["range"], "full");
        assert_eq!(bindings[3]["mode"], "all");
        assert_eq!(normalize_game_controls(&result), result);
        let profiles: Vec<Value> = (0..100)
            .map(|i| json!({"layout":format!("a{i}"),"bindings":[]}))
            .collect();
        assert_eq!(
            normalize_game_controls(&json!({"deviceMappings":profiles}))["deviceMappings"]
                .as_array()
                .unwrap()
                .len(),
            64
        );
        let alternatives: Vec<Value> = (0..128).map(Value::from).collect();
        assert_eq!(normalize_game_controls(&json!({"deviceMappings":[{"layout":"legacy","bindings":[{"target":"south","kind":"buttons","buttons":alternatives,"mode":"any"}]}]}))["deviceMappings"][0]["bindings"][0]["buttons"].as_array().unwrap().len(), 128);
    }

    #[test]
    fn keyboard_chords_preserve_valid_combinations_and_normalize_invalid_input() {
        let fallback = json!({"bindings":[{"key":"keyA","control":"south"}]});
        let result = normalize_keyboard_mapping(
            &json!({"bindings":[{"key":"unknown","control":"south"}],"chords":[
                {"keys":["keyJ","keyK"],"control":"home"},
                {"keys":["keyK","keyJ"],"control":"south"},
                {"keys":["keyA","keyA"],"control":"east"}
            ]}),
            &fallback,
        );
        assert_eq!(result["bindings"], json!([]));
        assert_eq!(result["chords"].as_array().unwrap().len(), 1);
        assert_eq!(normalize_keyboard_mapping(&result, &fallback), result);
        assert_eq!(normalize_keyboard_mapping(&json!({}), &fallback), fallback);
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
        let home = normalize_game_controls(&json!({"buttons": {"south": "home", "home": "south"}}));
        assert_eq!(home["buttons"]["south"], "home");
        assert_eq!(home["buttons"]["home"], "south");
        assert_eq!(normalize_game_controls(&home), home);
        for buttons in [
            json!({"south": "east"}),
            json!({"home": "l2", "l2": "home"}),
            json!({"south": "l2", "l2": "south"}),
            json!({"south": 7}),
        ] {
            let normalized = normalize_game_controls(&json!({"buttons": buttons}));
            assert_eq!(normalized["buttons"]["south"], "south");
            assert_eq!(normalized["buttons"]["home"], "home");
        }
    }

    #[test]
    fn game_controls_preserve_bounded_digital_extra_bindings_by_layout() {
        let result = normalize_game_controls(&json!({"extraButtons": [
            {"layout":"a","button":17,"target":"east"},
            {"layout":"a","button":17,"target":"south"},
            {"layout":"a","button":16,"target":"south"},
            {"layout":"a","button":128,"target":"south"},
            {"layout":"a","button":18.5,"target":"south"},
            {"layout":"","button":18,"target":"south"},
            {"layout":"a","button":18,"target":"l2"},
            {"layout":"a","button":18,"target":"home"},
            {"layout":"a","button":18,"target":"menu"}
        ]}));
        assert_eq!(
            result["extraButtons"],
            json!([
                {"layout":"a","button":17,"target":"east"},
                {"layout":"a","button":18,"target":"home"}
            ])
        );
        assert_eq!(
            normalize_game_controls(&json!({}))["extraButtons"],
            json!([])
        );
        let bindings: Vec<Value> = (0..200)
            .map(|index| json!({"layout":format!("a{index}"),"button":17,"target":"south"}))
            .collect();
        assert_eq!(
            normalize_game_controls(&json!({"extraButtons":bindings}))["extraButtons"]
                .as_array()
                .unwrap()
                .len(),
            128
        );
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
