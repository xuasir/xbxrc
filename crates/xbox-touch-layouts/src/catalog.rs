use crate::{invalid, TouchBundleError};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TouchBundleDescriptor {
    pub id: String,
    pub tab_version: String,
    pub manifest_version: String,
    pub layout_version: String,
}

/// 云标题详情同时匹配商品与 Xbox 身份；外层 titleId 属于串流服务。
pub fn title_bundles(
    response: &Value,
    product_id: &str,
    title_id: &str,
) -> Result<Vec<TouchBundleDescriptor>, TouchBundleError> {
    let entries = response
        .as_array()
        .or_else(|| response.get("results").and_then(Value::as_array))
        .ok_or_else(|| invalid("官方云标题详情需要为列表"))?;
    let title_id = title_id
        .parse::<u64>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| invalid("Xbox 游戏身份无效"))?;
    let entry = entries.iter().find(|entry| {
        let details = &entry["details"];
        let identity = match details.get("xboxTitleId") {
            Some(Value::String(value)) => value.parse::<u64>().ok(),
            Some(Value::Number(value)) => value.as_u64(),
            _ => None,
        };
        identity == Some(title_id)
            && details
                .get("productId")
                .and_then(Value::as_str)
                .is_some_and(|value| value.eq_ignore_ascii_case(product_id))
    });
    let Some(entry) = entry else {
        return Ok(Vec::new());
    };
    // 与 better-xcloud 的地平线 6 选源一致：使用社区 forza 布局。
    if title_id == 2079757188
        || entry["titleId"]
            .as_str()
            .is_some_and(|id| id.eq_ignore_ascii_case("FORZAHORIZON6"))
    {
        return Ok(Vec::new());
    }
    let Some(tabs) = entry["details"]
        .get("supportedTabs")
        .filter(|tabs| !tabs.is_null())
    else {
        return Ok(Vec::new());
    };
    let tabs = tabs
        .as_array()
        .ok_or_else(|| invalid("官方布局包列表无效"))?;
    if tabs.len() > 64 {
        return Err(invalid("官方布局包数量超出限制"));
    }
    let mut bundles = Vec::new();
    for value in tabs {
        let bundle: TouchBundleDescriptor =
            serde_json::from_value(value.clone()).map_err(|_| invalid("官方布局包版本信息缺失"))?;
        if bundle.id.is_empty()
            || bundle.id.len() > 128
            || !bundle
                .id
                .bytes()
                .all(|value| value.is_ascii_alphanumeric() || matches!(value, b'-' | b'_' | b'.'))
        {
            return Err(invalid("官方布局包身份无效"));
        }
        version(&bundle.manifest_version)?;
        version(&bundle.layout_version)?;
        version(&bundle.tab_version)?;
        // 格式版本进入编译诊断；目录按合法身份提供资源，实际字段决定运行范围。
        bundles.push(bundle);
    }
    bundles.sort_by(|left, right| {
        version(&right.tab_version)
            .unwrap()
            .cmp(&version(&left.tab_version).unwrap())
            .then_with(|| {
                version(&right.manifest_version)
                    .unwrap()
                    .cmp(&version(&left.manifest_version).unwrap())
            })
            .then_with(|| {
                version(&right.layout_version)
                    .unwrap()
                    .cmp(&version(&left.layout_version).unwrap())
            })
            .then_with(|| left.id.cmp(&right.id))
    });
    bundles.dedup_by(|left, right| left.id == right.id);
    Ok(bundles)
}

pub(crate) fn version(value: &str) -> Result<[u32; 4], TouchBundleError> {
    let parts: Vec<_> = value.split('.').collect();
    if parts.is_empty() || parts.len() > 4 {
        return Err(invalid("官方布局版本需要一至四段数字"));
    }
    let mut result = [0; 4];
    for (index, part) in parts.into_iter().enumerate() {
        if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(invalid("官方布局版本无效"));
        }
        result[index] = part.parse().map_err(|_| invalid("官方布局版本无效"))?;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tab(id: &str, version: &str) -> Value {
        json!({"id":id,"tabVersion":version,"manifestVersion":"2.0.0.0","layoutVersion":"4.1.0.0"})
    }

    fn title(product: &str, id: Value, tabs: Value) -> Value {
        json!({"titleId":"CLOUD_STREAM_TITLE", "details":{
            "productId":product,"xboxTitleId":id,"supportedTabs":tabs
        }})
    }

    #[test]
    fn chooses_only_requested_title_and_latest_numeric_version() {
        let data = json!({"results":[
            title("OTHER",json!(20),json!([tab("other", "9.0.0.0")])),
            title("game",json!(10),json!([tab("old", "2.0.0.0"),tab("latest", "10.0.0.0")]))
        ]});
        let result = title_bundles(&data, "GAME", "10").unwrap();
        assert_eq!(result[0].id, "latest");
        assert_eq!(result.len(), 2);
        assert!(title_bundles(&data, "GAME", "30").unwrap().is_empty());
        assert!(title_bundles(&data, "OTHER", "10").unwrap().is_empty());
    }

    #[test]
    fn requires_nested_cloud_identity_and_accepts_numeric_strings() {
        let data = json!([title(
            "GAME",
            json!("0010"),
            json!([tab("bundle", "1.0.0.0")])
        )]);
        assert_eq!(title_bundles(&data, "GAME", "10").unwrap().len(), 1);
        for data in [
            json!([{"titleId":"10","supportedTabs":[tab("other", "1.0.0.0")]}]),
            json!([{"details":{"supportedTabs":[tab("other", "1.0.0.0")]}}]),
            json!([{"titleId":"10","details":{"productId":"GAME","xboxTitleId":20,
                "supportedTabs":[tab("other", "1.0.0.0")]}}]),
        ] {
            assert!(title_bundles(&data, "GAME", "10").unwrap().is_empty());
        }
    }

    #[test]
    fn native_touch_titles_have_an_empty_catalog() {
        for tabs in [Value::Null, json!([])] {
            let data = json!([title("GAME", json!(10), tabs)]);
            assert!(title_bundles(&data, "GAME", "10").unwrap().is_empty());
        }
        let data = json!([{"details":{"productId":"GAME","xboxTitleId":10}}]);
        assert!(title_bundles(&data, "GAME", "10").unwrap().is_empty());
        assert!(title_bundles(&json!({}), "GAME", "10").is_err());
        assert!(
            title_bundles(&json!([title("GAME", json!(10), json!({}))]), "GAME", "10").is_err()
        );
    }

    #[test]
    fn forza_horizon_6_uses_community_layouts_with_official_tabs_present() {
        let mut horizon = title(
            "HORIZON6EDITION",
            json!(2079757188u64),
            json!([tab("official", "1.0")]),
        );
        horizon["titleId"] = json!("FORZAHORIZON6");
        assert!(
            title_bundles(&json!([horizon]), "HORIZON6EDITION", "2079757188")
                .unwrap()
                .is_empty()
        );

        let other = title("GAME", json!(10), json!([tab("official", "1.0")]));
        assert_eq!(
            title_bundles(&json!([other]), "GAME", "10").unwrap().len(),
            1
        );
    }

    #[test]
    fn accepts_short_service_versions_and_preserves_display_values() {
        let data = json!([title(
            "GAME",
            json!(10),
            json!([
                {"id":"short","tabVersion":"10.2","manifestVersion":"2.0","layoutVersion":"4.1"},
                {"id":"older","tabVersion":"9","manifestVersion":"2","layoutVersion":"4.0.0"}
            ])
        )]);
        let bundles = title_bundles(&data, "GAME", "10").unwrap();
        assert_eq!(bundles[0].id, "short");
        assert_eq!(bundles[0].manifest_version, "2.0");
        assert_eq!(version("4.1").unwrap(), [4, 1, 0, 0]);
        for invalid in ["", "1..0", "+1", "1.0.0.0.0", "1.0-beta"] {
            assert!(version(invalid).is_err());
        }
    }

    #[test]
    fn unknown_format_version_reaches_compiler_and_bad_identity_is_rejected() {
        let mut newer = tab("future", "1.0.0.0");
        newer["layoutVersion"] = json!("5.0.0.0");
        assert_eq!(
            title_bundles(
                &json!([title("GAME", json!(10), json!([newer]))]),
                "GAME",
                "10"
            )
            .unwrap()
            .len(),
            1
        );
        assert!(title_bundles(
            &json!([title(
                "GAME",
                json!(10),
                json!([tab("../escape", "1.0.0.0")])
            )]),
            "GAME",
            "10"
        )
        .is_err());
    }
}
