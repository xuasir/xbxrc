use crate::{cloud_access::load_scoped_stream_access, query_failure::query_error, XboxBridgeError};
use xbox_streaming::Target;
use xbox_webapi::{GamePassApi, WebApiError};

#[uniffi::export(async_runtime = "tokio")]
pub async fn fetch_catalog_source(
    access_handle: String,
    source: String,
    market: String,
    language: String,
) -> Result<Vec<String>, XboxBridgeError> {
    let access = load_scoped_stream_access(&access_handle, Target::Cloud, None, None)?;
    if source == "recent" {
        let value = crate::cloud_access::query_xcloud_api(&access_handle, &access, None)?
            .get_recent_titles(xbox_cloud_catalog_flow::DEFAULT_RECENT_TITLE_LIMIT)
            .await
            .map_err(query_error)?;
        let items = value
            .get("results")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| query_error(WebApiError::parse("invalid recent titles response")))?;
        return Ok(items
            .iter()
            .filter_map(|item| {
                item.get("details")
                    .and_then(|v| v.get("productId"))
                    .and_then(serde_json::Value::as_str)
            })
            .filter(|id| {
                !id.is_empty() && id.len() <= 32 && id.bytes().all(|b| b.is_ascii_alphanumeric())
            })
            .map(str::to_ascii_uppercase)
            .collect());
    }
    let sigl = match source.as_str() {
        "newest" => xbox_cloud_catalog_flow::GAME_PASS_NEWEST_SIGL_ID,
        "popular" => "e7590b22-e299-44db-ae22-25c61405454c",
        "leaving" => "393f05bf-e596-4ef6-9487-6d4fa0eab987",
        _ => {
            return Err(XboxBridgeError::InvalidData(
                "invalid catalog source".into(),
            ))
        }
    };
    let value = GamePassApi::new()
        .get_sigl(sigl, &market, &language)
        .await
        .map_err(query_error)?;
    if !value.is_array() {
        return Err(query_error(WebApiError::parse(
            "invalid discovery collection",
        )));
    }
    Ok(xbox_cloud_catalog_flow::extract_newest_product_ids(&value))
}
