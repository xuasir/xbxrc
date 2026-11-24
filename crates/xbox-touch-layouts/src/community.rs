use crate::{
    decode_bundle, invalid, TouchBundle, TouchBundleError, MAX_BUNDLE_BYTES, MAX_JSON_BYTES,
    MAX_LAYOUTS,
};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    io::{Cursor, Write},
};
use zip::write::SimpleFileOptions;

pub fn validate_community_game(
    definition: &Value,
    product_id: &str,
) -> Result<(), TouchBundleError> {
    // 旧社区定义按已校验的 Xbox 文件身份关联；附带商品身份时继续核验。
    if definition.get("product_id").is_some_and(|id| {
        !id.as_str()
            .is_some_and(|id| id.eq_ignore_ascii_case(product_id))
    }) {
        return Err(invalid("社区布局与当前游戏商品身份不匹配"));
    }
    Ok(())
}

/// 社区包装进入既有 TAKX 解包、引用、能力校验和本机资源合同。
pub fn community_bundle(
    definition: &Value,
    sources: &[Value],
    revision: &str,
    product_id: &str,
    locale: &str,
    scale: f64,
) -> Result<(Vec<u8>, TouchBundle), TouchBundleError> {
    validate_community_game(definition, product_id)?;
    if revision.len() != 40 || !revision.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(invalid("社区布局来源版本无效"));
    }
    let expected = definition["layouts"]
        .as_array()
        .filter(|names| !names.is_empty() && names.len() <= 16)
        .ok_or_else(|| invalid("社区布局引用数量无效"))?;
    if expected.len() != sources.len() {
        return Err(invalid("社区布局引用尚未加载完整"));
    }
    let mut documents = BTreeMap::new();
    let mut files = BTreeMap::new();
    files.insert("sources/game.json".to_owned(), json_bytes(definition)?);
    for (index, source) in sources.iter().enumerate() {
        if source["schema_version"] != 1 {
            return Err(invalid("社区布局包装版本需要为 1"));
        }
        let layouts = source["layouts"]
            .as_object()
            .ok_or_else(|| invalid("社区布局内容无效"))?;
        files.insert(format!("sources/{index}.json"), json_bytes(source)?);
        for (id, document) in layouts {
            if id.is_empty()
                || id.len() > 128
                || !id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            {
                return Err(invalid("社区布局身份无效"));
            }
            // 按游戏索引顺序合并，后续共用布局覆盖同身份条目。
            documents.insert(id.to_ascii_lowercase(), document.clone());
        }
    }
    if documents.is_empty() || documents.len() > MAX_LAYOUTS {
        return Err(invalid("社区布局数量无效"));
    }
    let default = definition["default_layout"]
        .as_str()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let default = if default.is_empty() {
        documents.keys().next().unwrap().clone()
    } else {
        default
    };
    if !documents.contains_key(&default) {
        return Err(invalid("社区默认布局引用缺失"));
    }
    for (id, document) in documents {
        files.insert(format!("layouts/neutral/{id}.json"), json_bytes(&document)?);
    }
    files.insert(
        "takxmanifest.json".to_owned(),
        json_bytes(&json!({
            "version":"1.0", "versionName":revision, "layouts":"layouts", "defaultLayout":default,
            "source":{"provider":"better-xcloud","revision":revision,"productId":product_id}
        }))?,
    );
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let mut expanded = 0usize;
    for (path, bytes) in files {
        expanded = expanded.saturating_add(bytes.len());
        if expanded > MAX_BUNDLE_BYTES {
            return Err(invalid("社区布局资源超出大小限制"));
        }
        writer
            .start_file(
                path,
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated),
            )
            .map_err(|_| invalid("社区布局资源封装失败"))?;
        writer
            .write_all(&bytes)
            .map_err(|_| invalid("社区布局资源封装失败"))?;
    }
    let original = writer
        .finish()
        .map_err(|_| invalid("社区布局资源封装失败"))?
        .into_inner();
    let mut bundle = decode_bundle(&original, locale, scale)?;
    // 提交版本与下载快照一致，布局索引保存其不可变来源修订。
    bundle.version = revision[..12].to_owned();
    Ok((original, bundle))
}

fn json_bytes(value: &Value) -> Result<Vec<u8>, TouchBundleError> {
    let bytes = serde_json::to_vec(value).map_err(|_| invalid("社区布局序列化失败"))?;
    if bytes.len() > MAX_JSON_BYTES {
        return Err(invalid("社区布局文档超出大小限制"));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_real_community_layouts_with_default_and_author() {
        let source: Value = serde_json::from_slice(include_bytes!(
            "../../../iosapp/XBXRC/Resources/TouchLayouts/forza-layouts.json"
        ))
        .unwrap();
        let definition =
            json!({"product_id":"GAME","default_layout":"layout-1","layouts":["forza"]});
        let (bytes, bundle) = community_bundle(
            &definition,
            &[source.clone()],
            "4416b1ab07588a3244a5c02ce1fa5dfa64d8df74",
            "game",
            "zh-Hans",
            2.0,
        )
        .unwrap();
        assert_eq!(bundle.version, "4416b1ab0758");
        assert!(bundle
            .layouts
            .iter()
            .any(|layout| layout.is_default && layout.document["author"] == "redphx"));
        assert!(!crate::content_hash(&bytes).is_empty());
        assert!(validate_community_game(&json!({"layouts":["forza"]}), "GAME").is_ok());
        assert!(community_bundle(
            &definition,
            &[source],
            "4416b1ab07588a3244a5c02ce1fa5dfa64d8df74",
            "OTHER",
            "zh-Hans",
            2.0
        )
        .is_err());
        assert!(community_bundle(
            &definition,
            &[],
            "4416b1ab07588a3244a5c02ce1fa5dfa64d8df74",
            "GAME",
            "zh-Hans",
            2.0
        )
        .is_err());
    }
}
