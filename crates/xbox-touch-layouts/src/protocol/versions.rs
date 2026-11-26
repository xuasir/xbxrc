use super::registry::{registry, warning};
use super::types::TouchProtocolIssue;
use serde_json::{json, Value};

pub(super) fn convert(document: &mut Value, issues: &mut Vec<TouchProtocolIssue>) {
    let Some(schema) = document["$schema"].as_str() else {
        return;
    };
    if !schema.starts_with("https://schema.play.xbox.com/")
        && !schema
            .starts_with("https://raw.githubusercontent.com/microsoft/xbox-game-streaming-tools/")
    {
        warning(
            issues,
            "/$schema",
            "version",
            "此版本标识使用已知协议字段运行",
        );
        return;
    }
    let version = schema
        .split('/')
        .collect::<Vec<_>>()
        .windows(2)
        .find(|pair| pair[0] == "layout")
        .map(|pair| pair[1].trim_start_matches('v'))
        .unwrap_or(if schema.ends_with("/schemas/layout") {
            "1.2"
        } else {
            ""
        })
        .to_owned();
    let catalog = registry();
    let profile = catalog
        .versions
        .iter()
        .find(|(_, values)| values.contains(&version))
        .map(|(key, _)| key.as_str());
    match profile {
        Some("legacyV1") => legacy_node(&mut document["content"], true),
        Some("legacyV2") => legacy_node(&mut document["content"], false),
        Some(_) => return,
        None => {
            warning(
                issues,
                "/$schema",
                "version",
                "此版本使用已知协议字段运行，扩展字段保留诊断",
            );
            return;
        }
    }
    if let Some(upper) = document["content"]["upper"].as_object() {
        let mut slots = Vec::new();
        for (key, reverse) in [
            ("right", false),
            ("rightCenter", true),
            ("leftCenter", false),
            ("left", true),
        ] {
            let mut items = upper
                .get(key)
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            if reverse {
                items.reverse();
            }
            slots.extend(items);
        }
        document["content"]["upper"] = json!({"right":slots});
    }
}

fn legacy_node(value: &mut Value, oldest: bool) {
    if let Some(object) = value.as_object_mut() {
        if let Some(kind) = object
            .get("type")
            .and_then(Value::as_str)
            .map(str::to_owned)
        {
            if registry().control_definitions.contains_key(&kind) && (oldest || kind != "button") {
                for field in ["action", "pullAction"] {
                    if let Some(action) = object.get_mut(field) {
                        let replacement = match action.as_str() {
                            Some("leftJoystickUp") => Some("leftJoystickDown"),
                            Some("leftJoystickDown") => Some("leftJoystickUp"),
                            Some("rightJoystickUp") => Some("rightJoystickDown"),
                            Some("rightJoystickDown") => Some("rightJoystickUp"),
                            _ => None,
                        };
                        if let Some(name) = replacement {
                            *action = Value::String(name.into());
                        }
                    }
                }
            }
            if kind == "joystick" {
                if let Some(threshold) = object.remove("threshold") {
                    object.insert("actionThreshold".into(), threshold);
                }
            }
            if ["button", "touchpad", "joystick", "throttle"].contains(&kind.as_str()) {
                legacy_icon(object, ["joystick", "throttle"].contains(&kind.as_str()));
            } else if kind == "arcadeButtons" {
                for value in object.values_mut() {
                    if let Some(button) = value.as_object_mut() {
                        legacy_icon(button, false);
                    }
                }
            }
        }
        for child in object.values_mut() {
            legacy_node(child, oldest);
        }
    } else if let Some(items) = value.as_array_mut() {
        for child in items {
            legacy_node(child, oldest);
        }
    }
}

fn legacy_icon(object: &mut serde_json::Map<String, Value>, knob: bool) {
    if let Some(icon) = object.remove("icon") {
        let face = json!({"faceImage":{"type":"icon","value":icon}});
        object.insert(
            "styles".into(),
            if knob {
                json!({"default":{"knob":face}})
            } else {
                json!({"default":face})
            },
        );
    }
}
