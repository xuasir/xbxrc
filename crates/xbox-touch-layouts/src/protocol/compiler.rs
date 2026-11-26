use super::actions::{
    action as compile_action, action_source, axes, axis_source, bounded, polar_axes,
    protocol_number,
};
use super::registry::{registry, warning};
use super::styles;
use super::types::{CompiledTouchLayout, TouchCompiledControl, TouchProtocolIssue};
use super::{versions, MAX_COMPILED_BYTES};
use crate::{invalid, resolve_runtime_document, TouchBundleError, MAX_JSON_BYTES};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

pub fn compile_layout_json(
    document_json: &str,
    id: &str,
    format: &str,
    assets: &[String],
    state_json: Option<&str>,
    patch_json: Option<&str>,
) -> Result<String, TouchBundleError> {
    let compiled = compile_layout(document_json, id, format, assets, state_json, patch_json)?;
    let result = serde_json::to_string(&compiled).map_err(|_| invalid("协议布局编译编码失败"))?;
    if result.len() > MAX_COMPILED_BYTES {
        return Err(invalid("协议布局编译结果超过8MB"));
    }
    Ok(result)
}

pub fn compile_layout(
    document_json: &str,
    id: &str,
    format: &str,
    assets: &[String],
    state_json: Option<&str>,
    patch_json: Option<&str>,
) -> Result<CompiledTouchLayout, TouchBundleError> {
    if document_json.len() > MAX_COMPILED_BYTES || assets.len() > 256 {
        return Err(invalid("布局或资源数量超出编译预算"));
    }
    let mut source: Value =
        serde_json::from_str(document_json).map_err(|_| invalid("布局JSON格式无效"))?;
    // 旧编译快照保留原文；新修订始终从同一来源重新解释。
    if source.get("irVersion").is_some() {
        source = serde_json::from_str(
            source["runtimeDocument"]
                .as_str()
                .ok_or_else(|| invalid("编译快照缺少原文"))?,
        )
        .map_err(|_| invalid("编译快照原文无效"))?;
    }
    let source_json = serde_json::to_string(&source).map_err(|_| invalid("布局原文编码失败"))?;
    if source_json.len() > MAX_JSON_BYTES {
        return Err(invalid("布局JSON超过2MB"));
    }
    guard_depth(&source, 0)?;
    let mut issues = Vec::new();
    let mut document = match resolve_runtime_document(&source_json, state_json, patch_json) {
        Ok(resolved) => serde_json::from_str(&resolved).map_err(|_| invalid("布局引用结果无效"))?,
        Err(error) if crate::references::reference_budget_error(&error) => return Err(error),
        Err(error) => {
            warning(
                &mut issues,
                "/references",
                "reference",
                &format!("{}；已知控件继续运行", error),
            );
            if state_json.is_some() || patch_json.is_some() {
                issues.last_mut().unwrap().severity = "unsupported".into();
            }
            source.clone()
        }
    };
    let runtime_document =
        serde_json::to_string(&document).map_err(|_| invalid("布局运行来源编码失败"))?;
    versions::convert(&mut document, &mut issues);
    for message in document["_xbxrcProtocolWarnings"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        warning(&mut issues, "/schema_version", "version", message);
    }
    for issue in document["_xbxrcReferenceWarnings"]
        .as_array()
        .into_iter()
        .flatten()
    {
        warning(
            &mut issues,
            issue[0].as_str().unwrap_or("/references"),
            "reference",
            issue[1].as_str().unwrap_or("引用使用默认值"),
        );
    }
    if !document["content"].is_object() && !issues.iter().any(|i| i.capability == "reference") {
        return Err(invalid("布局缺少有效content对象"));
    }
    if !document["content"].is_object() {
        document["content"] = json!({});
    }
    for message in document["_xbxrcRuntime"]["warnings"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        warning(&mut issues, "/context/state", "context", message);
    }
    let palette = styles::palette(&document["styles"]["colors"], &mut issues);
    let assets: BTreeSet<_> = assets.iter().cloned().collect();
    let layers: BTreeSet<_> = document["content"]["layers"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(k, _)| k.clone())
        .collect();
    let mut compiler = Compiler {
        controls: BTreeMap::new(),
        issues,
        palette: palette.clone(),
        assets: assets.clone(),
        layers,
        control_types: BTreeSet::new(),
        behaviors: BTreeSet::new(),
        capabilities: BTreeSet::new(),
        runnable_controls: 0,
        isolated_controls: 0,
        state_update: state_json.is_some() || patch_json.is_some(),
    };
    let mut content = document["content"].clone();
    compiler.content(&mut content, "/content");
    let uses_standard_fallback = compiler.runnable_controls == 0
        && (compiler.isolated_controls > 0
            || compiler.issues.iter().any(|i| i.capability == "reference"));
    if uses_standard_fallback {
        warning(
            &mut compiler.issues,
            "/content",
            "layout.fallback",
            "此布局使用通用手柄回退，原始布局已保留",
        );
        content = serde_json::from_str::<Value>(include_str!(
            "../../../../iosapp/XBXRC/Resources/TouchLayouts/generic.json"
        ))
        .expect("built-in touch layout")["content"]
            .clone();
        compiler.content(&mut content, "/fallback");
    }
    let is_runnable = compiler
        .issues
        .iter()
        .all(|issue| issue.severity != "unsupported");
    let is_fully_compatible = compiler.issues.is_empty();
    Ok(CompiledTouchLayout {
        ir_version: registry().ir_version,
        protocol_revision: registry().revision.clone(),
        id: id.into(),
        name: document["name"]
            .as_str()
            .or_else(|| document["displayName"].as_str())
            .unwrap_or(id)
            .into(),
        author: document["author"].as_str().map(str::to_owned),
        format: format.into(),
        schema: document["$schema"]
            .as_str()
            .unwrap_or("runtime-v4.1")
            .into(),
        content,
        controls: compiler.controls,
        color_palette: palette,
        available_assets: assets.into_iter().collect(),
        runtime_document,
        required_capabilities: compiler.capabilities.into_iter().collect(),
        control_types: compiler.control_types.into_iter().collect(),
        behaviors: compiler.behaviors.into_iter().collect(),
        issues: compiler.issues,
        is_runnable,
        is_fully_compatible,
        uses_standard_fallback,
    })
}

fn guard_depth(value: &Value, depth: usize) -> Result<(), TouchBundleError> {
    if depth > 64 {
        return Err(invalid("布局嵌套超过64层"));
    }
    match value {
        Value::Object(map) => {
            for v in map.values() {
                guard_depth(v, depth + 1)?;
            }
        }
        Value::Array(items) => {
            for v in items {
                guard_depth(v, depth + 1)?;
            }
        }
        _ => {}
    }
    Ok(())
}

struct Compiler {
    controls: BTreeMap<String, TouchCompiledControl>,
    issues: Vec<TouchProtocolIssue>,
    palette: Value,
    assets: BTreeSet<String>,
    layers: BTreeSet<String>,
    control_types: BTreeSet<String>,
    behaviors: BTreeSet<String>,
    capabilities: BTreeSet<String>,
    runnable_controls: usize,
    isolated_controls: usize,
    state_update: bool,
}

impl Compiler {
    fn content(&mut self, content: &mut Value, path: &str) {
        let Some(object) = content.as_object_mut() else {
            self.isolated_controls += 1;
            return;
        };
        for key in object
            .keys()
            .filter(|key| {
                ![
                    "left", "right", "center", "upper", "lower", "layers", "sensors",
                ]
                .contains(&key.as_str())
            })
            .cloned()
            .collect::<Vec<_>>()
        {
            warning(
                &mut self.issues,
                &format!("{path}/{key}"),
                "layout.field",
                "此区域保留在原文，当前执行已知区域",
            );
        }
        for side in ["left", "right", "center"] {
            if let Some(region) = object.get(side) {
                self.region(region, &format!("{path}/{side}"), "LayoutControlWheel");
            }
            for ring in ["inner", "outer"] {
                if let Some(slots) = object.get_mut(side).and_then(|wheel| wheel.get_mut(ring)) {
                    self.slots(
                        slots,
                        &format!("{path}/{side}/{ring}"),
                        if ring == "inner" { 4 } else { 8 },
                        ring == "outer",
                    );
                }
            }
        }
        for (region, fields) in [
            ("upper", vec!["right"]),
            ("lower", vec!["leftCenter", "rightCenter"]),
        ] {
            if let Some(value) = object.get(region) {
                self.region(
                    value,
                    &format!("{path}/{region}"),
                    if region == "upper" {
                        "LayoutUpperContent"
                    } else {
                        "LayoutLowerContent"
                    },
                );
            }
            for field in fields {
                if let Some(slots) = object.get_mut(region).and_then(|edge| edge.get_mut(field)) {
                    self.slots(slots, &format!("{path}/{region}/{field}"), 32, false);
                }
            }
        }
        if let Some(slot) = object
            .get_mut("lower")
            .and_then(|edge| edge.get_mut("center"))
        {
            self.slot(slot, &format!("{path}/lower/center"));
        }
        if let Some(layers) = object.get_mut("layers").and_then(Value::as_object_mut) {
            self.behaviors.insert("layer".into());
            for (name, layer) in layers {
                self.content(layer, &format!("{path}/layers/{name}"));
            }
        }
        if object
            .get("sensors")
            .is_some_and(|value| !value.is_null() && value != &json!([]))
        {
            self.isolated_controls += 1;
            warning(
                &mut self.issues,
                &format!("{path}/sensors"),
                "host.sensor",
                "传感器能力暂缓，手柄控件继续运行",
            );
        }
    }

    fn region(&mut self, value: &Value, path: &str, definition: &str) {
        if value.is_null() {
            return;
        }
        if let Some(object) = value.as_object() {
            let fields = registry().properties(registry().definition(definition), None);
            for key in object.keys().filter(|key| !fields.contains_key(*key)) {
                warning(
                    &mut self.issues,
                    &format!("{path}/{key}"),
                    "layout.field",
                    "此区域扩展保留在原文，当前使用已知槽位",
                );
            }
        } else {
            self.isolated_controls += 1;
            warning(
                &mut self.issues,
                path,
                "layout.region",
                "此区域使用已有控件或通用布局回退",
            );
        }
    }

    fn slots(&mut self, value: &mut Value, path: &str, limit: usize, outer: bool) {
        let Some(items) = value.as_array_mut() else {
            warning(&mut self.issues, path, "layout.slot", "此槽位区域已隔离");
            self.isolated_controls += 1;
            *value = json!([]);
            return;
        };
        let mut count = 0;
        for (index, item) in items.iter_mut().enumerate() {
            if count >= limit {
                *item = Value::Null;
                warning(
                    &mut self.issues,
                    path,
                    "layout.slot",
                    "超出物理槽位的控件已隔离",
                );
                continue;
            }
            count += if outer && item.is_array() { 2 } else { 1 };
            if !outer && item.is_array() && path.ends_with("/inner") {
                warning(
                    &mut self.issues,
                    &format!("{path}/{index}"),
                    "layout.slot",
                    "内轮盘控件组已隔离",
                );
                *item = Value::Null;
                self.isolated_controls += 1;
            } else {
                self.slot(item, &format!("{path}/{index}"));
            }
        }
    }

    fn slot(&mut self, value: &mut Value, path: &str) {
        if value.is_null() {
            return;
        }
        if let Some(items) = value.as_array_mut() {
            self.behaviors.insert("cluster".into());
            if items.len() > 4 {
                warning(
                    &mut self.issues,
                    path,
                    "layout.cluster",
                    "控件组按四项容量运行",
                );
                items.truncate(4);
            }
            for (index, child) in items.iter_mut().enumerate() {
                self.slot(child, &format!("{path}/{index}"));
            }
            return;
        }
        let kind = value["type"].as_str().unwrap_or("").to_owned();
        if kind == "blank" {
            return;
        }
        let Some(definition) = registry().control_definitions.get(&kind) else {
            self.isolated_controls += 1;
            warning(
                &mut self.issues,
                path,
                "control",
                "此控件已隔离，其余操作继续运行",
            );
            *value = json!({"type":"blank"});
            return;
        };
        if self.controls.len() >= 512 {
            warning(
                &mut self.issues,
                path,
                "layout.budget",
                "控件编译达到512项预算",
            );
            *value = json!({"type":"blank"});
            return;
        }
        self.control_types.insert(kind.clone());
        self.capabilities.insert(format!("control:{kind}"));
        let properties = registry().properties(registry().definition(definition), Some(&kind));
        for key in value
            .as_object()
            .unwrap()
            .keys()
            .filter(|key| {
                !properties.contains_key(*key)
                    && ![
                        "scale",
                        "priority",
                        "relative",
                        "expand",
                        "sticky",
                        "toggle",
                        "visible",
                        "enabled",
                        "actionThreshold",
                        "_xbxrcControlId",
                    ]
                    .contains(&key.as_str())
            })
            .cloned()
            .collect::<Vec<_>>()
        {
            warning(
                &mut self.issues,
                &format!("{path}/{key}"),
                "control.field",
                "此扩展字段保留在原文，当前使用已知协议字段",
            );
            value.as_object_mut().unwrap().remove(&key);
        }
        for (field, default) in [
            ("visible", true),
            ("enabled", true),
            ("relative", true),
            ("expand", true),
            ("toggle", false),
            ("sticky", false),
            ("renderAsButton", false),
        ] {
            let flag = match value.get(field) {
                None => default,
                Some(v) if v.is_boolean() => v.as_bool().unwrap(),
                Some(v)
                    if !self.state_update
                        && [Some("true"), Some("false")].contains(&v.as_str()) =>
                {
                    warning(
                        &mut self.issues,
                        &format!("{path}/{field}"),
                        "value.boolean",
                        "此状态值已归一为布尔值",
                    );
                    v.as_str() == Some("true")
                }
                Some(_) => {
                    warning(
                        &mut self.issues,
                        &format!("{path}/{field}"),
                        "value.boolean",
                        "此状态值使用默认值",
                    );
                    if self.state_update {
                        self.issues.last_mut().unwrap().severity = "unsupported".into();
                    }
                    default
                }
            };
            value[field] = json!(flag);
            if ["toggle", "sticky"].contains(&field) && flag {
                self.behaviors.insert(field.into());
            }
        }
        for (field, default, min, max) in [
            ("scale", 1.0, 0.25, 3.0),
            ("priority", 0.0, -1_000_000.0, 1_000_000.0),
        ] {
            value[field] = json!(bounded(
                &value[field],
                default,
                min,
                max,
                &format!("{path}/{field}"),
                &mut self.issues
            ));
        }
        value["actionThreshold"] = json!(protocol_number(
            &value["actionThreshold"],
            "ActionThreshold",
            0.0,
            &format!("{path}/actionThreshold"),
            &mut self.issues
        ));
        if kind == "directionalPad" {
            value["deadzone"] = json!(protocol_number(
                &value["deadzone"],
                "DeadzoneDirectionalPad",
                0.25,
                &format!("{path}/deadzone"),
                &mut self.issues
            ));
        }
        let declared_input = ["action", "pullAction", "axis", "axisUp", "axisDown"]
            .iter()
            .any(|key| value.get(*key).is_some_and(|v| !v.is_null()));
        let action = compile_action(
            &value["action"],
            &format!("{path}/action"),
            &self.layers,
            &mut self.issues,
        );
        let pull_action = compile_action(
            &value["pullAction"],
            &format!("{path}/pullAction"),
            &self.layers,
            &mut self.issues,
        );
        let axis = axes(&value["axis"], &format!("{path}/axis"), &mut self.issues);
        let axis_up = polar_axes(
            &value["axisUp"],
            "axisUp",
            &format!("{path}/axisUp"),
            &mut self.issues,
        );
        let axis_down = polar_axes(
            &value["axisDown"],
            "axisDown",
            &format!("{path}/axisDown"),
            &mut self.issues,
        );
        if let Some(interaction) = value.get("interaction") {
            let fields =
                registry().properties(registry().definition("DirectionalPadInteraction"), None);
            for key in interaction
                .as_object()
                .into_iter()
                .flatten()
                .map(|(key, _)| key)
                .filter(|key| !fields.contains_key(*key))
            {
                warning(
                    &mut self.issues,
                    &format!("{path}/interaction/{key}"),
                    "control.interaction",
                    "此交互扩展使用已知模式",
                );
            }
            if interaction
                .get("activationType")
                .is_some_and(|v| !matches!(v.as_str(), Some("exclusive" | "allowNeighboring")))
            {
                warning(
                    &mut self.issues,
                    &format!("{path}/interaction/activationType"),
                    "control.interaction",
                    "方向键使用默认激活模式",
                );
            }
        }
        value["action"] = action.as_ref().map(action_source).unwrap_or(Value::Null);
        value["pullAction"] = pull_action
            .as_ref()
            .map(action_source)
            .unwrap_or(Value::Null);
        value["axis"] = axis_source(&axis, &value["axis"]);
        value["axisUp"] = axis_source(&axis_up, &value["axisUp"]);
        value["axisDown"] = axis_source(&axis_down, &value["axisDown"]);
        let activation_type = value["interaction"]["activationType"]
            .as_str()
            .filter(|v| ["exclusive", "allowNeighboring"].contains(v))
            .unwrap_or("allowNeighboring")
            .into();
        let styles = styles::normalize(
            &value["styles"],
            &kind,
            &format!("{path}/styles"),
            &self.palette,
            &self.assets,
            &mut self.issues,
        );
        value["styles"] = styles.clone();
        value["_xbxrcControlId"] = json!(path);
        let mut children = BTreeMap::new();
        if kind == "arcadeButtons" {
            self.behaviors.insert("arcadeCombination".into());
            for name in [
                "lightPunch",
                "mediumPunch",
                "heavyPunch",
                "lightKick",
                "mediumKick",
                "heavyKick",
                "specialPunch",
                "specialKick",
            ] {
                if value[name].is_object() {
                    value[name]["type"] = json!("button");
                    self.slot(&mut value[name], &format!("{path}/{name}"));
                    if let Some(child) = self.controls.get(&format!("{path}/{name}")) {
                        children.insert(name.into(), child.clone());
                    }
                }
            }
        }
        let usable = kind == "directionalPad"
            || action.is_some()
            || pull_action.is_some()
            || !axis.is_empty()
            || !axis_up.is_empty()
            || !axis_down.is_empty()
            || children.values().any(|child| child.action.is_some());
        if usable {
            self.runnable_controls += 1;
        } else if declared_input
            || ["joystick", "touchpad", "throttle", "arcadeButtons"].contains(&kind.as_str())
        {
            self.isolated_controls += 1;
            value["visible"] = json!(false);
            warning(
                &mut self.issues,
                path,
                "input.control",
                "此控件缺少可用输入，已保留其余控件",
            );
        }
        for mapping in axis.iter().chain(&axis_up).chain(&axis_down) {
            self.capabilities
                .insert(format!("output:{}", mapping.output));
        }
        if pull_action.is_some() {
            self.behaviors.insert("pullAction".into());
        }
        self.controls.insert(
            path.into(),
            TouchCompiledControl {
                id: path.into(),
                kind,
                action,
                pull_action,
                axis,
                axis_up,
                axis_down,
                activation_type,
                styles,
                children,
            },
        );
    }
}
