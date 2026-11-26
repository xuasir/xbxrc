use crate::{
    bundle::bind_assets, invalid, references::resolve_partial, TouchBundleError, MAX_JSON_BYTES,
};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// 按当前 Context.state 展开同一个布局；状态补丁逐项执行，保留成功项。
pub fn resolve_runtime_document(
    document_json: &str,
    state_json: Option<&str>,
    patch_json: Option<&str>,
) -> Result<String, TouchBundleError> {
    let document = parse(document_json)?;
    let Some(recipe) = document.get("_xbxrcRuntime") else {
        if !document_json.contains("\"$ref\"") {
            return Ok(document_json.to_string());
        }
        let files = BTreeMap::from([("runtime-layout.json".into(), json_bytes(&document)?)]);
        let (mut resolved, warnings) = resolve_partial(&files, "runtime-layout.json", None)?;
        if !resolved.is_object() {
            return Err(invalid("布局整体引用需要有效对象"));
        }
        resolved["_xbxrcReferenceWarnings"] = serde_json::json!(warnings);
        return String::from_utf8(json_bytes(&resolved)?).map_err(|_| invalid("布局引用编码无效"));
    };
    let layout = recipe
        .get("layout")
        .ok_or_else(|| invalid("动态布局来源缺失"))?;
    let mut context = recipe
        .get("context")
        .cloned()
        .ok_or_else(|| invalid("动态上下文来源缺失"))?;
    if !context.is_object() {
        return Err(invalid("动态上下文需要为对象"));
    }
    if let Some(state) = state_json {
        context["state"] = parse(state)?;
    }
    let mut warnings = Vec::new();
    if let Some(patch) = patch_json {
        if patch.len() > 64 * 1024 {
            return Err(invalid("状态补丁超过64KB"));
        }
        let operations = parse(patch)?;
        let operations = operations
            .as_array()
            .ok_or_else(|| invalid("状态补丁需要为操作数组"))?;
        if operations.len() > 256 {
            return Err(invalid("状态补丁操作数量超出限制"));
        }
        for (index, operation) in operations.iter().enumerate() {
            let mut operation = operation.clone();
            if let Some(path) = operation.get("path").and_then(Value::as_str) {
                if !path.is_empty() && !path.starts_with('/') {
                    operation["path"] = Value::String(format!("/{path}"));
                }
            }
            let mut next = context["state"].clone();
            let success =
                serde_json::from_value::<json_patch::Patch>(Value::Array(vec![operation]))
                    .ok()
                    .is_some_and(|patch| json_patch::patch(&mut next, &patch).is_ok());
            if success {
                context["state"] = next;
            } else {
                warnings.push(format!("状态补丁第{}项执行失败", index + 1));
            }
        }
    }
    let files = BTreeMap::from([
        ("runtime-layout.json".into(), json_bytes(layout)?),
        ("runtime-context.json".into(), json_bytes(&context)?),
    ]);
    let (mut resolved, reference_warnings) = resolve_partial(&files, "runtime-layout.json", None)?;
    if !resolved.is_object() {
        return Err(invalid("布局整体来源需要有效对象"));
    }
    if (state_json.is_some() || patch_json.is_some())
        && reference_warnings.iter().any(|(path, _)| {
            (path.ends_with("/visible") || path.ends_with("/enabled"))
                && resolved.pointer(path).is_none_or(Value::is_null)
        })
    {
        return Err(invalid("动态输入状态的引用需要有效值"));
    }
    warnings.extend(
        reference_warnings
            .iter()
            .map(|(path, message)| format!("{path}：{message}")),
    );
    let (resolved_context, context_warnings) =
        resolve_partial(&files, "runtime-context.json", None)?;
    warnings.extend(
        context_warnings
            .iter()
            .map(|(path, message)| format!("{path}：{message}")),
    );
    let styles = merge(resolved_context.get("styles"), resolved.get("styles"));
    if !styles.is_null() {
        resolved["styles"] = styles;
    }
    let assets: BTreeMap<String, String> = recipe
        .get("assets")
        .map(|value| serde_json::from_value(value.clone()))
        .transpose()
        .map_err(|_| invalid("动态资源映射无效"))?
        .unwrap_or_default();
    let mut used = BTreeSet::new();
    let mut resource_issues = Vec::new();
    bind_assets(&mut resolved, &assets, &mut used, &mut resource_issues);
    warnings.extend(resource_issues);
    resolved["_xbxrcRuntime"] = serde_json::json!({
        "layout": layout, "context": context, "assets": assets, "warnings": warnings,
    });
    String::from_utf8(json_bytes(&resolved)?).map_err(|_| invalid("动态布局编码无效"))
}

fn parse(text: &str) -> Result<Value, TouchBundleError> {
    if text.len() > MAX_JSON_BYTES {
        return Err(invalid("布局JSON超过2MB"));
    }
    serde_json::from_str(text).map_err(|_| invalid("动态布局JSON格式无效"))
}

fn json_bytes(value: &Value) -> Result<Vec<u8>, TouchBundleError> {
    let bytes = serde_json::to_vec(value).map_err(|_| invalid("动态布局编码失败"))?;
    if bytes.len() > MAX_JSON_BYTES {
        return Err(invalid("展开后的布局JSON超过2MB"));
    }
    Ok(bytes)
}

fn merge(base: Option<&Value>, patch: Option<&Value>) -> Value {
    let Some(patch) = patch else {
        return base.cloned().unwrap_or(Value::Null);
    };
    let Some(object) = patch.as_object() else {
        return patch.clone();
    };
    let mut result = base.and_then(Value::as_object).cloned().unwrap_or_default();
    for (key, value) in object {
        result.insert(key.clone(), merge(result.get(key), Some(value)));
    }
    Value::Object(result)
}

#[cfg(test)]
#[path = "runtime.test.rs"]
mod tests;
