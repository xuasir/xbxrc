use serde_json::Value;

use crate::policy::compiler::resolve_session_access;
use crate::policy::config::Config;
use crate::policy::context::Context;
use crate::policy::session::{ResolvedSessionAccess, SessionAccessContext};
use crate::policy::types::{CompileError, Region, Target};

/// 从认证 token 中提取会话接入上下文，供 compiler 做统一解析。
pub fn parse_session_access_context(token: &Value) -> Result<SessionAccessContext, CompileError> {
    let data = token.get("data").unwrap_or(token);

    let gs_token = data
        .get("gsToken")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or(CompileError::MissingGsToken)?;

    let raw_regions = data
        .get("offeringSettings")
        .and_then(|value| value.get("regions"))
        .and_then(Value::as_array)
        .ok_or(CompileError::MissingRegions)?;

    let regions = raw_regions
        .iter()
        .filter_map(parse_region)
        .collect::<Vec<_>>();
    if regions.is_empty() {
        return Err(CompileError::MissingRegions);
    }

    Ok(SessionAccessContext {
        gs_token: Some(gs_token),
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
    Some(Region {
        name: object
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        base_uri: object
            .get("baseUri")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
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
}
