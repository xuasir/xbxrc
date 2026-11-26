use crate::{
    archive::{archive_path, read_archive},
    invalid,
    references::{read_json, resolve},
    TouchBundleError, MAX_JSON_BYTES, MAX_LAYOUTS,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TouchAssetReference {
    pub id: String,
    pub path: String,
    pub content_hash: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResolvedTouchLayout {
    pub id: String,
    pub name: String,
    pub is_default: bool,
    pub document: Value,
    pub assets: Vec<TouchAssetReference>,
    pub issues: Vec<String>,
}

#[derive(Debug)]
pub struct TouchBundle {
    pub version: String,
    pub version_name: String,
    pub layouts: Vec<ResolvedTouchLayout>,
    pub files: BTreeMap<String, Vec<u8>>,
}

pub fn decode_bundle(
    data: &[u8],
    locale: &str,
    scale: f64,
) -> Result<TouchBundle, TouchBundleError> {
    let files = read_archive(data)?;
    // 文档预算先于兼容性降级判断，超限资源保持整个包拒绝。
    if files
        .iter()
        .any(|(path, data)| path.ends_with(".json") && data.len() > MAX_JSON_BYTES)
    {
        return Err(invalid("布局 JSON 超过 2 MB"));
    }
    let manifest = read_json(&files, "takxmanifest.json")?;
    let version = manifest["version"]
        .as_str()
        .filter(|value| valid_version(value))
        .ok_or_else(|| invalid("官方布局包版本无效"))?
        .to_string();
    let root = archive_path(
        manifest["layouts"]
            .as_str()
            .ok_or_else(|| invalid("官方布局包缺少布局目录"))?,
    )?;
    let context = manifest["contextFile"]
        .as_str()
        .map(archive_path)
        .transpose()?;
    if let Some(path) = &context.filter(|path| files.contains_key(path)) {
        read_json(&files, path)?;
    }
    let directories = locale_directories(&root, locale)?;
    let mut selected = BTreeMap::new();
    for directory in directories {
        let prefix = format!("{directory}/");
        for path in files.keys() {
            let Some(name) = path
                .strip_prefix(&prefix)
                .filter(|name| !name.contains('/') && name.ends_with(".json"))
            else {
                continue;
            };
            selected.insert(name.trim_end_matches(".json").to_string(), path.clone());
        }
    }
    if selected.is_empty() || selected.len() > MAX_LAYOUTS {
        return Err(invalid("官方布局包的可用布局数量无效"));
    }
    let assets = asset_catalog(&files, manifest["assets"].as_str(), locale, scale)?;
    let default = manifest["defaultLayout"]
        .as_str()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let mut layouts = Vec::new();
    for (id, path) in selected {
        // 单份布局的引用失败保留为诊断项，其他有效布局继续可选择。
        let (mut document, mut issues) = match resolve(&files, &path) {
            Ok(value) => (value, Vec::new()),
            Err(error) => (
                read_json(&files, &path).unwrap_or(Value::Null),
                vec![error.to_string()],
            ),
        };
        if !document["content"].is_object() {
            issues.push("布局缺少有效控件内容".into());
        }
        let name = document["displayName"]
            .as_str()
            .or_else(|| document["name"].as_str())
            .unwrap_or(&id)
            .chars()
            .take(80)
            .collect();
        let mut used = BTreeSet::new();
        bind_assets(&mut document, &assets, &mut used, &mut issues);
        layouts.push(ResolvedTouchLayout {
            id: id.clone(),
            name,
            is_default: id == default,
            document,
            assets: used
                .into_iter()
                .map(|path| TouchAssetReference {
                    id: path.clone(),
                    content_hash: crate::content_hash(&files[&path]),
                    path,
                })
                .collect(),
            issues,
        });
    }
    Ok(TouchBundle {
        version,
        version_name: manifest["versionName"]
            .as_str()
            .unwrap_or_default()
            .chars()
            .take(80)
            .collect(),
        layouts,
        files,
    })
}

pub fn valid_version(version: &str) -> bool {
    crate::catalog::version(version).is_ok()
}

fn locale_directories(root: &str, locale: &str) -> Result<Vec<String>, TouchBundleError> {
    if locale.len() > 40
        || !locale
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err(invalid("布局语言无效"));
    }
    let mut result = vec![root.to_string(), format!("{root}/neutral")];
    let mut language = String::new();
    let mut nested = root.to_string();
    for part in locale
        .to_ascii_lowercase()
        .split('-')
        .filter(|part| !part.is_empty())
    {
        if !language.is_empty() {
            language.push('-');
        }
        language.push_str(part);
        result.push(format!("{root}/{language}"));
        nested.push('/');
        nested.push_str(&language);
        if !result.contains(&nested) {
            result.push(nested.clone());
        }
    }
    Ok(result)
}

fn asset_catalog(
    files: &BTreeMap<String, Vec<u8>>,
    root: Option<&str>,
    locale: &str,
    scale: f64,
) -> Result<BTreeMap<String, String>, TouchBundleError> {
    let Some(root) = root else {
        return Ok(BTreeMap::new());
    };
    let root = archive_path(root)?;
    let directories = locale_directories(&root, locale)?;
    let scale = if scale.is_finite() {
        scale.clamp(1.0, 4.0)
    } else {
        1.0
    };
    let mut choices: BTreeMap<String, (usize, f64, String)> = BTreeMap::new();
    for (rank, directory) in directories.into_iter().enumerate() {
        for path in files.keys() {
            let Some(relative) = path.strip_prefix(&format!("{directory}/")) else {
                continue;
            };
            let parts: Vec<_> = relative.split('/').collect();
            let (density, filename) = match parts.as_slice() {
                [filename] => (1.0, *filename),
                [density, filename] if density.starts_with('@') && density.ends_with('x') => {
                    let Some(value) = density
                        .trim_start_matches('@')
                        .trim_end_matches('x')
                        .parse::<f64>()
                        .ok()
                        .filter(|value| value.is_finite() && *value > 0.0)
                    else {
                        continue;
                    };
                    (value, *filename)
                }
                _ => continue,
            };
            let distance = (density - scale).abs();
            if choices
                .get(filename)
                .is_none_or(|(old_rank, old_distance, _)| {
                    rank > *old_rank || (rank == *old_rank && distance < *old_distance)
                })
            {
                choices.insert(filename.into(), (rank, distance, path.clone()));
            }
        }
    }
    let mut aliases = BTreeMap::new();
    for (filename, (_, _, path)) in choices {
        aliases.insert(filename.clone(), path.clone());
        if let Some((stem, _)) = filename.rsplit_once('.') {
            aliases.insert(stem.into(), path.clone());
        }
        aliases.insert(path.clone(), path);
    }
    Ok(aliases)
}

fn bind_assets(
    value: &mut Value,
    assets: &BTreeMap<String, String>,
    used: &mut BTreeSet<String>,
    issues: &mut Vec<String>,
) {
    match value {
        Value::Array(items) => {
            for item in items {
                bind_assets(item, assets, used, issues);
            }
        }
        Value::Object(object) => {
            if object.get("type").and_then(Value::as_str) == Some("asset") {
                if let Some(path) = object
                    .get("value")
                    .and_then(Value::as_str)
                    .and_then(|name| assets.get(&name.to_ascii_lowercase()))
                {
                    used.insert(path.clone());
                    object.insert("value".into(), Value::String(path.clone()));
                } else {
                    issues.push("布局图片资源缺失".into());
                }
            }
            for value in object.values_mut() {
                bind_assets(value, assets, used, issues);
            }
        }
        _ => {}
    }
}
