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
    serde_json::from_slice(data).map_err(|_| invalid("布局 JSON 格式无效"))
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
}

impl Resolver<'_> {
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
