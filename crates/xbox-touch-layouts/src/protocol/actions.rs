use super::registry::{registry, warning};
use super::types::{CompiledAction, CompiledAxis, CompiledCurve, TouchProtocolIssue};
use serde_json::{json, Value};
use std::collections::BTreeSet;

pub(super) fn action(
    value: &Value,
    path: &str,
    layers: &BTreeSet<String>,
    issues: &mut Vec<TouchProtocolIssue>,
) -> Option<CompiledAction> {
    if value.is_null() {
        return None;
    }
    if let Some(name) = value.as_str() {
        if registry().gamepad_actions.contains(name) {
            return Some(CompiledAction::Gamepad { name: name.into() });
        }
    } else if let Some(items) = value.as_array() {
        let items: Vec<_> = items
            .iter()
            .enumerate()
            .filter_map(|(index, v)| action(v, &format!("{path}/{index}"), layers, issues))
            .collect();
        return (!items.is_empty()).then_some(CompiledAction::Group { items });
    } else {
        match value["type"].as_str() {
            Some("layer") => {
                if let Some(target) = value["target"]
                    .as_str()
                    .filter(|name| layers.contains(*name))
                {
                    return Some(CompiledAction::Layer {
                        target: target.into(),
                    });
                }
            }
            Some("segmented") => {
                if let Some(items) = value["items"].as_array().filter(|items| !items.is_empty()) {
                    let items = items
                        .iter()
                        .enumerate()
                        .map(|(index, v)| {
                            action(v, &format!("{path}/items/{index}"), layers, issues)
                                .unwrap_or(CompiledAction::Group { items: vec![] })
                        })
                        .collect();
                    return Some(CompiledAction::Segmented { items });
                }
            }
            Some("turbo") => {
                if let Some(interval) = value["interval"]
                    .as_f64()
                    .filter(|v| v.is_finite() && *v > 0.0)
                {
                    if let Some(inner) =
                        action(&value["action"], &format!("{path}/action"), layers, issues)
                    {
                        if controller_only(&inner) {
                            return Some(CompiledAction::Turbo {
                                action: Box::new(inner),
                                interval,
                            });
                        }
                    }
                }
            }
            _ => {}
        }
    }
    warning(
        issues,
        path,
        "input.action",
        "此输入动作已隔离，其余已知操作继续运行",
    );
    None
}

fn controller_only(action: &CompiledAction) -> bool {
    match action {
        CompiledAction::Gamepad { .. } => true,
        CompiledAction::Group { items } => !items.is_empty() && items.iter().all(controller_only),
        _ => false,
    }
}

pub(super) fn axes(
    value: &Value,
    path: &str,
    issues: &mut Vec<TouchProtocolIssue>,
) -> Vec<CompiledAxis> {
    if value.is_null() {
        return vec![];
    }
    if let Some(items) = value.as_array() {
        return items
            .iter()
            .enumerate()
            .flat_map(|(index, v)| axes(v, &format!("{path}/{index}"), issues))
            .collect();
    }
    let catalog = registry();
    let definition = match value["input"].as_str() {
        Some("axisXY") => "_InputMappingXYToGamepad2DOutput",
        Some("axisX" | "axisY") => "_InputMapping1DToGamepad1DOutput",
        _ => "_InputMappingMagnitudinalToGamepadMagnitudinalOutput",
    };
    let rules = catalog.properties(catalog.definition(definition), None);
    unknown_fields(value, &rules, path, issues);
    if let Some(rule) = rules.get("deadzone") {
        unknown_fields(
            &value["deadzone"],
            &catalog.properties(rule, None),
            &format!("{path}/deadzone"),
            issues,
        );
    }
    if let Some(rule) = rules.get("responseCurve") {
        unknown_fields(
            &value["responseCurve"],
            &catalog.properties(rule, value["responseCurve"]["type"].as_str()),
            &format!("{path}/responseCurve"),
            issues,
        );
    }
    let input = value["input"].as_str().unwrap_or("");
    let output = value["output"].as_str().unwrap_or("");
    if !catalog.axis_inputs.contains(input)
        || !catalog.axis_outputs.contains(output)
        || (input == "axisXY") != ["leftJoystick", "rightJoystick"].contains(&output)
    {
        warning(
            issues,
            path,
            "input.axis",
            "此轴输出已隔离，其余已知操作继续运行",
        );
        return vec![];
    }
    let response_curve = curve(
        &value["responseCurve"],
        &format!("{path}/responseCurve"),
        issues,
    );
    vec![CompiledAxis {
        input: input.into(),
        output: output.into(),
        deadzone: protocol_number(
            &value["deadzone"]["threshold"],
            "DeadzoneThreshold",
            0.0,
            &format!("{path}/deadzone/threshold"),
            issues,
        ),
        radial: value["deadzone"]["radial"].as_bool().unwrap_or(false),
        sensitivity: protocol_number(
            &value["sensitivity"],
            "Sensitivity",
            1.0,
            &format!("{path}/sensitivity"),
            issues,
        ),
        response_curve,
    }]
}

pub(super) fn action_source(action: &CompiledAction) -> Value {
    match action {
        CompiledAction::Gamepad { name } => json!(name),
        CompiledAction::Layer { target } => json!({"type":"layer","target":target}),
        CompiledAction::Group { items } => Value::Array(items.iter().map(action_source).collect()),
        CompiledAction::Segmented { items } => {
            json!({"type":"segmented","items":items.iter().map(action_source).collect::<Vec<_>>()})
        }
        CompiledAction::Turbo { action, interval } => {
            json!({"type":"turbo","action":action_source(action),"interval":interval})
        }
    }
}

pub(super) fn axis_source(axes: &[CompiledAxis], original: &Value) -> Value {
    let values:Vec<_>=axes.iter().map(|axis| {
        let mut value=json!({"input":axis.input,"output":axis.output,
            "deadzone":{"threshold":axis.deadzone,"radial":axis.radial},"sensitivity":axis.sensitivity});
        if let Some(curve)=&axis.response_curve {value["responseCurve"]=json!({"type":curve.kind,"range":[curve.lower,curve.upper]});}
        value
    }).collect();
    if original.is_string() && values.len() == 1 {
        return json!(axes[0].output);
    }
    if original.is_object() && values.len() == 1 {
        values.into_iter().next().unwrap()
    } else {
        Value::Array(values)
    }
}

fn unknown_fields(
    value: &Value,
    fields: &serde_json::Map<String, Value>,
    path: &str,
    issues: &mut Vec<TouchProtocolIssue>,
) {
    for key in value
        .as_object()
        .into_iter()
        .flatten()
        .map(|(key, _)| key)
        .filter(|key| !fields.contains_key(*key))
    {
        warning(
            issues,
            &format!("{path}/{key}"),
            "input.field",
            "此轴扩展字段保留在原文，当前执行已知映射",
        );
    }
}

pub(super) fn polar_axes(
    value: &Value,
    input: &str,
    path: &str,
    issues: &mut Vec<TouchProtocolIssue>,
) -> Vec<CompiledAxis> {
    if let Some(output) = value.as_str() {
        axes(&json!({"input":input,"output":output}), path, issues)
    } else {
        axes(value, path, issues)
    }
}

pub(super) fn bounded(
    value: &Value,
    default: f64,
    min: f64,
    max: f64,
    path: &str,
    issues: &mut Vec<TouchProtocolIssue>,
) -> f64 {
    if value.is_null() {
        return default;
    }
    if let Some(number) = value
        .as_f64()
        .filter(|v| v.is_finite() && (min..=max).contains(v))
    {
        return number;
    }
    warning(issues, path, "value", "此数值使用协议默认值");
    default
}

pub(super) fn protocol_number(
    value: &Value,
    definition: &str,
    default: f64,
    path: &str,
    issues: &mut Vec<TouchProtocolIssue>,
) -> f64 {
    if value.is_null() {
        return default;
    }
    if let Some(number) = value.as_f64().filter(|number| number.is_finite()) {
        if registry().accepts(registry().definition(definition), value) {
            return number;
        }
    }
    warning(issues, path, "value", "此数值使用协议默认值");
    default
}

fn curve(value: &Value, path: &str, issues: &mut Vec<TouchProtocolIssue>) -> Option<CompiledCurve> {
    if value.is_null() {
        return None;
    }
    let kind = value["type"].as_str().unwrap_or("");
    if ["circular", "circular-inverse"].contains(&kind) {
        if let Some(range) = value["range"].as_array().filter(|r| r.len() == 2) {
            if let (Some(lower), Some(upper)) = (range[0].as_f64(), range[1].as_f64()) {
                if (-1.0..=1.0).contains(&lower) && (-1.0..=1.0).contains(&upper) && lower <= upper
                {
                    return Some(CompiledCurve {
                        kind: kind.into(),
                        lower,
                        upper,
                    });
                }
            }
        }
    }
    warning(issues, path, "input.curve", "此响应曲线使用线性回退");
    None
}
