use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::policy::compiler::resolve_session_access;
use crate::policy::config::Config;
use crate::policy::context::Context;
use crate::policy::session::{ResolvedSessionAccess, SessionAccessContext};
use crate::policy::types::{CompileError, Region, Target};

/// 强类型凭证：屏蔽原始 JSON 解析细节。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StreamingToken {
    pub gs_token: String,
}

impl StreamingToken {
    pub fn parse(token: &Value) -> Result<Self, CompileError> {
        let data = token.get("data").unwrap_or(token);
        let gs_token = data
            .get("gsToken")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .ok_or(CompileError::MissingGsToken)?;

        Ok(Self { gs_token })
    }
}

/// 从认证 token 中提取会话接入上下文，供 compiler 做统一解析。
pub fn parse_session_access_context(token: &Value) -> Result<SessionAccessContext, CompileError> {
    let streaming_token = StreamingToken::parse(token)?;

    let data = token.get("data").unwrap_or(token);
    let raw_regions = data
        .get("offeringSettings")
        .and_then(|value| value.get("regions"))
        .and_then(Value::as_array)
        .ok_or(CompileError::MissingRegions)?;

    let mut seen = std::collections::BTreeSet::new();
    let regions = raw_regions
        .iter()
        .filter_map(parse_region)
        .filter(|region| seen.insert(region.name.to_ascii_lowercase()))
        .collect::<Vec<_>>();
    if regions.is_empty() {
        return Err(CompileError::MissingRegions);
    }

    Ok(SessionAccessContext {
        gs_token: Some(streaming_token.gs_token),
        regions,
    })
}

/// 组合 token 解析与接入选择，给 adapter 一步式调用。
pub fn resolve_session_access_from_token(
    config: &Config,
    target: Target,
    target_id: String,
    token: &Value,
) -> Result<ResolvedSessionAccess, CompileError> {
    let access = parse_session_access_context(token)?;
    let context = Context {
        target,
        target_id,
        session: access,
        ..Default::default()
    };

    resolve_session_access(config, &context)
}

fn parse_region(value: &Value) -> Option<Region> {
    let object = value.as_object()?;
    let name = object.get("name")?.as_str()?.trim();
    if name.is_empty() || name.len() > 128 || name.chars().any(char::is_control) {
        return None;
    }
    let base_uri =
        crate::policy::session::compiler::normalize_base_url(object.get("baseUri")?.as_str()?)?;
    Some(Region {
        name: name.to_owned(),
        base_uri,
        is_default: object
            .get("isDefault")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        short_name: object
            .get("shortName")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        display_name: object
            .get("displayName")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        continent: object
            .get("continent")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{parse_session_access_context, resolve_session_access_from_token};
    use crate::policy::config::Config;
    use crate::policy::types::Target;

    #[test]
    fn parse_session_access_context_reads_gs_token_and_regions() {
        let token = json!({
            "data": {
                "gsToken": "gs-token",
                "offeringSettings": {
                    "regions": [
                        { "name": "WESTUS", "baseUri": "west.example.com", "isDefault": true }
                    ]
                }
            }
        });

        let access = parse_session_access_context(&token).unwrap();
        assert_eq!(access.gs_token.as_deref(), Some("gs-token"));
        assert_eq!(access.regions.len(), 1);
    }

    #[test]
    fn resolve_session_access_from_token_selects_default_region() {
        let token = json!({
            "data": {
                "gsToken": "gs-token",
                "offeringSettings": {
                    "regions": [
                        { "name": "WESTUS", "baseUri": "west.example.com", "isDefault": true }
                    ]
                }
            }
        });

        let config = Config::default();
        let access =
            resolve_session_access_from_token(&config, Target::Cloud, String::new(), &token)
                .unwrap();

        assert_eq!(access.gs_token, "gs-token");
        assert_eq!(access.base_url, "https://west.example.com");
    }
    #[test]
    fn service_regions_drop_invalid_origins_and_deduplicate_stable_names() {
        let token = serde_json::json!({"gsToken":"token", "offeringSettings":{"regions":[
            {"name":" WestUS ","baseUri":"https://wus.example/","isDefault":true},
            {"name":"westus","baseUri":"https://duplicate.example"},
            {"name":"","baseUri":"https://empty.example"},
            {"name":"unsafe","baseUri":"https://user:pass@unsafe.example"},
            {"name":"JapanEast","baseUri":"https://jp.example"}
        ]}});
        let access = parse_session_access_context(&token).unwrap();
        assert_eq!(access.regions.len(), 2);
        assert_eq!(access.regions[0].name, "WestUS");
        assert_eq!(access.regions[0].base_uri, "https://wus.example");
    }
}
