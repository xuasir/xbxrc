use crate::{archive::relative_path, invalid, TouchBundleError, MAX_JSON_BYTES};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn read_json(
    files: &BTreeMap<String, Vec<u8>>,
    path: &str,
) -> Result<Value, TouchBundleError> {
    let data = files
        .get(path)
        .ok_or_else(|| invalid("布局包缺少引用文件"))?;
    if data.len() > MAX_JSON_BYTES {
        return Err(invalid("布局 JSON 超过 2 MB"));
    }
    // 官方 TAK 对 UTF8 文档 trim 后解析；覆盖 BOM 与文本边缘的 Unicode 空白。
    let text = std::str::from_utf8(data)
        .map_err(|_| invalid("布局 JSON 的 UTF8 编码无效"))?
        .trim_matches(json_text_edge);
    serde_json::from_str(text).map_err(|error| {
        invalid(&format!(
            "布局 JSON 格式无效（行 {}，列 {}）",
            error.line(),
            error.column()
        ))
    })
}

fn json_text_edge(character: char) -> bool {
    // ECMAScript String.trim 的 WhiteSpace / LineTerminator 集合。
    matches!(
        character,
        '\u{0009}'..='\u{000D}'
            | ' '
            | '\u{00A0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200A}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202F}'
            | '\u{205F}'
            | '\u{3000}'
            | '\u{FEFF}'
    )
}

pub(crate) fn resolve(
    files: &BTreeMap<String, Vec<u8>>,
    path: &str,
) -> Result<Value, TouchBundleError> {
    let source = read_json(files, path)?;
    let mut state = Resolver {
        files,
        active: BTreeSet::new(),
        visited: 0,
        state_context: None,
    };
    let result = state.expand(&source, path, 0)?;
    if serde_json::to_vec(&result)
        .map_err(|_| invalid("布局序列化失败"))?
        .len()
        > MAX_JSON_BYTES
    {
        return Err(invalid("展开后的布局 JSON 超过 2 MB"));
    }
    Ok(result)
}

struct Resolver<'a> {
    files: &'a BTreeMap<String, Vec<u8>>,
    active: BTreeSet<String>,
    visited: usize,
    state_context: Option<&'a str>,
}

/// 引用按使用位置降级，其他可解释引用继续展开。
pub(crate) fn resolve_partial(
    files: &BTreeMap<String, Vec<u8>>,
    path: &str,
    context: Option<&str>,
) -> Result<(Value, Vec<(String, String)>), TouchBundleError> {
    let mut state = Resolver {
        files,
        active: BTreeSet::new(),
        visited: 0,
        state_context: context,
    };
    let mut issues = Vec::new();
    let result = state.partial(&read_json(files, path)?, path, 0, "", &mut issues)?;
    if serde_json::to_vec(&result)
        .map_err(|_| invalid("布局序列化失败"))?
        .len()
        > MAX_JSON_BYTES
    {
        return Err(invalid("展开后的布局 JSON 超过 2 MB"));
    }
    Ok((result, issues))
}

/// 静态引用先展开，Context.state 引用保留为当前包的动态来源。
pub(crate) fn resolve_dynamic(
    files: &BTreeMap<String, Vec<u8>>,
    path: &str,
    context: &str,
) -> Result<Value, TouchBundleError> {
    Resolver {
        files,
        active: BTreeSet::new(),
        visited: 0,
        state_context: Some(context),
    }
    .expand(&read_json(files, path)?, path, 0)
}

impl Resolver<'_> {
    fn partial(
        &mut self,
        value: &Value,
        source: &str,
        depth: usize,
        location: &str,
        issues: &mut Vec<(String, String)>,
    ) -> Result<Value, TouchBundleError> {
        self.visited += 1;
        if depth > 32 || self.visited > 100_000 {
            return Err(invalid("布局引用复杂度超出限制"));
        }
        match value {
            Value::Array(items) => items
                .iter()
                .enumerate()
                .map(|(index, item)| {
                    self.partial(
                        item,
                        source,
                        depth + 1,
                        &format!("{location}/{index}"),
                        issues,
                    )
                })
                .collect(),
            Value::Object(object) => {
                if let Some(reference) = object.get("$ref") {
                    let result = (|| {
                        let reference = reference
                            .as_str()
                            .ok_or_else(|| invalid("布局引用需要为字符串"))?;
                        let (file, pointer) = reference.split_once('#').unwrap_or((reference, ""));
                        let path = if file.is_empty() {
                            source.into()
                        } else {
                            relative_path(source, file)?
                        };
                        if !path.ends_with(".json")
                            || (!pointer.is_empty() && !pointer.starts_with('/'))
                        {
                            return Err(invalid("布局引用需要指向包内 JSON Pointer"));
                        }
                        if self.state_context == Some(path.as_str())
                            && (pointer == "/state" || pointer.starts_with("/state/"))
                        {
                            let mut retained = serde_json::Map::new();
                            retained.insert(
                                "$ref".into(),
                                Value::String(format!("runtime-context.json#{pointer}")),
                            );
                            for (key, value) in
                                object.iter().filter(|(key, _)| key.as_str() != "$ref")
                            {
                                retained.insert(
                                    key.clone(),
                                    self.partial(
                                        value,
                                        source,
                                        depth + 1,
                                        &pointer_child(location, key),
                                        issues,
                                    )?,
                                );
                            }
                            return Ok(Value::Object(retained));
                        }
                        let identity = format!("{path}#{pointer}");
                        if !self.active.insert(identity.clone()) {
                            return Err(invalid("布局引用存在循环"));
                        }
                        let expanded = (|| {
                            let document = read_json(self.files, &path)?;
                            let target = document
                                .pointer(pointer)
                                .ok_or_else(|| invalid("布局引用目标缺失"))?;
                            self.partial(target, &path, depth + 1, location, issues)
                        })();
                        self.active.remove(&identity);
                        let mut result = expanded?;
                        if object.len() > 1 {
                            if let Some(result) = result.as_object_mut() {
                                for (key, value) in
                                    object.iter().filter(|(key, _)| key.as_str() != "$ref")
                                {
                                    result.insert(
                                        key.clone(),
                                        self.partial(
                                            value,
                                            source,
                                            depth + 1,
                                            &pointer_child(location, key),
                                            issues,
                                        )?,
                                    );
                                }
                            } else {
                                issues.push((
                                    location.into(),
                                    "标量引用继续使用已知值，附加字段已保留在原文".into(),
                                ));
                            }
                        }
                        Ok(result)
                    })();
                    return match result {
                        Ok(value) => Ok(value),
                        Err(error) if reference_budget_error(&error) => Err(error),
                        Err(error) => {
                            issues.push((location.into(), error.to_string()));
                            Ok(Value::Null)
                        }
                    };
                }
                let mut result = serde_json::Map::new();
                for (key, value) in object {
                    result.insert(
                        key.clone(),
                        self.partial(
                            value,
                            source,
                            depth + 1,
                            &pointer_child(location, key),
                            issues,
                        )?,
                    );
                }
                Ok(Value::Object(result))
            }
            _ => Ok(value.clone()),
        }
    }
    fn expand(
        &mut self,
        value: &Value,
        source: &str,
        depth: usize,
    ) -> Result<Value, TouchBundleError> {
        self.visited += 1;
        if depth > 32 || self.visited > 100_000 {
            return Err(invalid("布局引用复杂度超出限制"));
        }
        match value {
            Value::Array(items) => items
                .iter()
                .map(|item| self.expand(item, source, depth + 1))
                .collect(),
            Value::Object(object) => {
                if let Some(reference) = object.get("$ref") {
                    let reference = reference
                        .as_str()
                        .ok_or_else(|| invalid("布局引用需要为字符串"))?;
                    let (file, pointer) = reference.split_once('#').unwrap_or((reference, ""));
                    let path = if file.is_empty() {
                        source.to_string()
                    } else {
                        relative_path(source, file)?
                    };
                    if !path.ends_with(".json")
                        || (!pointer.is_empty() && !pointer.starts_with('/'))
                    {
                        return Err(invalid("布局引用需要指向包内 JSON Pointer"));
                    }
                    if self.state_context == Some(path.as_str())
                        && (pointer == "/state" || pointer.starts_with("/state/"))
                    {
                        let mut retained = serde_json::Map::new();
                        retained.insert(
                            "$ref".into(),
                            Value::String(format!("runtime-context.json#{pointer}")),
                        );
                        for (key, value) in object.iter().filter(|(key, _)| key.as_str() != "$ref")
                        {
                            retained.insert(key.clone(), self.expand(value, source, depth + 1)?);
                        }
                        return Ok(Value::Object(retained));
                    }
                    let identity = format!("{path}#{pointer}");
                    if !self.active.insert(identity.clone()) {
                        return Err(invalid("布局引用存在循环"));
                    }
                    let document = read_json(self.files, &path)?;
                    let target = document
                        .pointer(pointer)
                        .ok_or_else(|| invalid("布局引用目标缺失"))?;
                    let mut result = self.expand(target, &path, depth + 1)?;
                    self.active.remove(&identity);
                    if object.len() > 1 {
                        let resolved = result
                            .as_object_mut()
                            .ok_or_else(|| invalid("布局引用附加字段需要对象"))?;
                        for (key, value) in object.iter().filter(|(key, _)| key.as_str() != "$ref")
                        {
                            resolved.insert(key.clone(), self.expand(value, source, depth + 1)?);
                        }
                    }
                    return Ok(result);
                }
                let mut result = serde_json::Map::new();
                for (key, value) in object {
                    result.insert(key.clone(), self.expand(value, source, depth + 1)?);
                }
                Ok(Value::Object(result))
            }
            _ => Ok(value.clone()),
        }
    }
}

fn pointer_child(parent: &str, key: &str) -> String {
    format!("{parent}/{}", key.replace('~', "~0").replace('/', "~1"))
}

pub(crate) fn reference_budget_error(error: &TouchBundleError) -> bool {
    let message = error.to_string();
    message.contains("复杂度") || message.contains("超过 2 MB") || message.contains("超过2MB")
}
