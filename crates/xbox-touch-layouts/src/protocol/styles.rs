use super::registry::{registry, warning};
use super::types::TouchProtocolIssue;
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;

pub(super) fn palette(value: &Value, issues: &mut Vec<TouchProtocolIssue>) -> Value {
    let catalog = registry();
    let mut normal: Map<String, Value> = catalog
        .system_colors
        .iter()
        .map(|(k, v)| (k.clone(), json!(v)))
        .collect();
    let mut contrast = Map::new();
    for (variant, target) in [("default", &mut normal), ("highContrast", &mut contrast)] {
        for (name, color) in value[variant].as_object().into_iter().flatten() {
            if color.as_str().is_some_and(valid_hex) {
                target.insert(name.clone(), color.clone());
            } else {
                warning(
                    issues,
                    &format!("/styles/colors/{variant}/{name}"),
                    "render.color",
                    "此色板值使用系统默认颜色",
                );
            }
        }
    }
    json!({"default":normal,"highContrast":contrast})
}

pub(super) fn normalize(
    value: &Value,
    kind: &str,
    path: &str,
    palette: &Value,
    assets: &BTreeSet<String>,
    issues: &mut Vec<TouchProtocolIssue>,
) -> Value {
    if value.is_null() {
        return json!({});
    }
    let Some(object) = value.as_object() else {
        warning(issues, path, "render.style", "控件样式使用系统默认值");
        return json!({});
    };
    let catalog = registry();
    let definition = catalog
        .control_styles
        .get(kind)
        .map(String::as_str)
        .unwrap_or("ButtonStyles");
    let states = catalog.properties(catalog.definition(definition), None);
    let mut result = Map::new();
    for (state, style) in object {
        let location = format!("{path}/{state}");
        if let Some(rule) = states.get(state) {
            result.insert(
                state.clone(),
                component(style, rule, &location, palette, assets, issues, 0),
            );
        } else {
            warning(
                issues,
                &location,
                "render.state",
                "此样式状态保留在原文，当前使用已知状态样式",
            );
        }
    }
    Value::Object(result)
}

fn component(
    value: &Value,
    rule: &Value,
    path: &str,
    palette: &Value,
    assets: &BTreeSet<String>,
    issues: &mut Vec<TouchProtocolIssue>,
    depth: usize,
) -> Value {
    if value.is_null() {
        return Value::Null;
    }
    if depth > 32 {
        warning(issues, path, "render.style", "样式层级使用默认回退");
        return Value::Null;
    }
    if let Some(items) = value.as_array() {
        let item_rule = registry().definition("PullIndicator");
        let properties = registry().properties(registry().definition("ButtonDefaultStyle"), None);
        let item_rule = properties.get("pullIndicator").unwrap_or(item_rule);
        let resolved = item_rule["items"].as_object().map(|_| &item_rule["items"]);
        let fallback = json!({"properties":{
            "background":{"$ref":"#/$defs/Background"},"faceImage":{"$ref":"#/$defs/FaceImage"},"opacity":{"$ref":"#/$defs/Opacity"}
        }});
        return Value::Array(
            items
                .iter()
                .enumerate()
                .map(|(index, item)| {
                    component(
                        item,
                        resolved.unwrap_or(&fallback),
                        &format!("{path}/{index}"),
                        palette,
                        assets,
                        issues,
                        depth + 1,
                    )
                })
                .collect(),
        );
    }
    let catalog = registry();
    let Some(object) = value.as_object() else {
        if catalog.accepts(rule, value) {
            return value.clone();
        }
        warning(issues, path, "render.value", "此样式值使用系统默认值");
        return Value::Null;
    };
    let mut object = object.clone();
    if path.ends_with("/stroke") && !object.contains_key("type") {
        object.insert("type".into(), json!("solid"));
    }
    let kind = object.get("type").and_then(Value::as_str);
    if let Some(kind) = kind {
        if kind == "icon" {
            if !object
                .get("value")
                .and_then(Value::as_str)
                .is_some_and(|name| catalog.icon_names.contains(name))
            {
                warning(issues, path, "render.icon", "此图标使用默认动作标签");
                return Value::Null;
            }
        } else if kind == "asset" {
            if !object
                .get("value")
                .and_then(Value::as_str)
                .is_some_and(|name| assets.contains(name))
            {
                warning(issues, path, "render.asset", "此图片资源使用默认图形");
                return Value::Null;
            }
        } else if kind == "color" {
            if !object
                .get("value")
                .and_then(Value::as_str)
                .is_some_and(|color| valid_color(color, palette))
            {
                warning(issues, path, "render.color", "此颜色使用系统默认值");
                return Value::Null;
            }
        }
    }
    let properties = catalog.properties(rule, kind);
    let mut result = Map::new();
    for (key, child) in &object {
        let location = format!("{path}/{key}");
        let Some(child_rule) = properties.get(key) else {
            warning(
                issues,
                &location,
                "render.field",
                "此可选样式字段保留在原文，当前绘制使用默认值",
            );
            continue;
        };
        if key == "opacity" {
            if let Some(opacity) = child
                .as_f64()
                .filter(|v| v.is_finite() && (0.0..=1.0).contains(v))
            {
                result.insert(key.clone(), json!(opacity));
            } else {
                warning(issues, &location, "render.opacity", "此透明度使用默认值");
            }
        } else if ["color", "fill"].contains(&key.as_str()) {
            if child.as_str().is_some_and(|v| valid_color(v, palette)) {
                result.insert(key.clone(), child.clone());
            } else {
                warning(issues, &location, "render.color", "此颜色使用系统默认值");
            }
        } else if key == "value" && ["icon", "asset", "color"].contains(&kind.unwrap_or("")) {
            result.insert(key.clone(), child.clone());
        } else if child.is_object() || child.is_array() {
            result.insert(
                key.clone(),
                component(
                    child,
                    child_rule,
                    &location,
                    palette,
                    assets,
                    issues,
                    depth + 1,
                ),
            );
        } else if catalog.accepts(child_rule, child) {
            result.insert(key.clone(), child.clone());
        } else {
            warning(issues, &location, "render.value", "此样式值使用系统默认值");
        }
    }
    Value::Object(result)
}

fn valid_color(value: &str, palette: &Value) -> bool {
    valid_hex(value)
        || value
            .strip_prefix("colors/")
            .is_some_and(|key| palette["default"][key].as_str().is_some_and(valid_hex))
}

fn valid_hex(value: &str) -> bool {
    value.strip_prefix('#').is_some_and(|digits| {
        [3, 4, 6, 8].contains(&digits.len()) && digits.bytes().all(|b| b.is_ascii_hexdigit())
    })
}
