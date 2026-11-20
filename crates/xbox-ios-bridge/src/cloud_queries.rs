use crate::cloud_access::load_scoped_stream_access;
use crate::{XboxBridgeError, XboxCloudGame};
use serde_json::json;
use xbox_cloud_catalog_flow::extract_product_detail;
use xbox_streaming::{Config, Target};
use xbox_webapi::{GamePassApi, XcloudApi};

#[derive(Debug, Clone, uniffi::Record)]
pub struct XboxCloudLanguageSupport {
    pub locale: String,
    pub interface: Option<u64>,
    pub audio: Option<u64>,
    pub subtitles: Option<u64>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct XboxCloudProductDetail {
    pub product_id: String,
    pub name: String,
    pub publisher_name: String,
    pub description: String,
    pub tile_image_url: String,
    pub poster_image_url: String,
    pub hero_image_url: String,
    pub categories: Vec<String>,
    pub developer: String,
    pub release_date: String,
    pub rating: String,
    pub attributes: Vec<String>,
    pub screenshots: Vec<String>,
    pub platforms: Vec<String>,
    pub languages: Vec<XboxCloudLanguageSupport>,
    pub accessibility: Vec<String>,
}

#[uniffi::export(async_runtime = "tokio")]
pub async fn fetch_cloud_product_detail(
    product_id: String,
    market: String,
    language: String,
) -> Result<XboxCloudProductDetail, XboxBridgeError> {
    let id = product_id.trim().to_ascii_uppercase();
    if id.is_empty() || id.len() > 32 || !id.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
        return Err(XboxBridgeError::InvalidData(
            "invalid product identity".into(),
        ));
    }
    let response = GamePassApi::new()
        .get_products_with_hydration(
            std::slice::from_ref(&id),
            &market,
            &language,
            "RemoteHighSapphire0",
        )
        .await
        .map_err(|error| crate::query_failure::query_error(error))?;
    let detail = extract_product_detail(&response, &id)
        .ok_or_else(|| XboxBridgeError::Data("游戏详情暂时无法载入".into()))?;
    Ok(XboxCloudProductDetail {
        product_id: detail.product.product_id,
        name: detail.product.name,
        publisher_name: detail.product.publisher_name,
        description: detail.product.description,
        tile_image_url: detail.product.tile_image_url,
        poster_image_url: detail.product.poster_image_url,
        hero_image_url: detail.product.hero_image_url,
        categories: detail.product.categories,
        developer: detail.developer,
        release_date: detail.release_date,
        rating: detail.rating,
        attributes: detail.attributes,
        screenshots: detail.screenshots,
        platforms: detail.platforms,
        languages: detail
            .languages
            .into_iter()
            .map(|language| XboxCloudLanguageSupport {
                locale: language.locale,
                interface: language.interface,
                audio: language.audio,
                subtitles: language.subtitles,
            })
            .collect(),
        accessibility: detail.accessibility,
    })
}

#[uniffi::export(async_runtime = "tokio")]
pub async fn fetch_cloud_wait_time(
    access_handle: String,
    stream_title_id: String,
    server_region: String,
) -> Result<Option<u64>, XboxBridgeError> {
    let access = load_scoped_stream_access(&access_handle, Target::Cloud, None, None)?;
    let mut config = Config::default();
    let preferred = server_region.trim();
    if !preferred.is_empty() && !preferred.eq_ignore_ascii_case("auto") {
        config.session.preferred_region_name = Some(preferred.into());
    }
    if !access.force_region_ip.is_empty() {
        config.session.force_region_ip = Some(access.force_region_ip);
    }
    let region = xbox_streaming::policy::session::compiler::pick_region(
        &access.session_access.regions,
        &config,
    )
    .map_err(|error| XboxBridgeError::InvalidData(error.to_string()))?;
    let response = XcloudApi::new(region.base_uri.clone(), access.bearer_token)
        .get_waiting_time(&stream_title_id)
        .await
        .map_err(|error| crate::query_failure::query_error(error))?;
    Ok(response
        .get("estimatedAllocationTimeInSeconds")
        .and_then(serde_json::Value::as_u64))
}

#[uniffi::export(async_runtime = "tokio")]
pub async fn resolve_cloud_game(
    access_handle: String,
    product_id: String,
    server_region: String,
) -> Result<Option<XboxCloudGame>, XboxBridgeError> {
    let access = load_scoped_stream_access(&access_handle, Target::Cloud, None, None)?;
    let mut config = Config::default();
    if !server_region.trim().is_empty() && !server_region.eq_ignore_ascii_case("auto") {
        config.session.preferred_region_name = Some(server_region);
    }
    if !access.force_region_ip.is_empty() {
        config.session.force_region_ip = Some(access.force_region_ip);
    }
    let region = xbox_streaming::policy::session::compiler::pick_region(
        &access.session_access.regions,
        &config,
    )
    .map_err(|error| XboxBridgeError::InvalidData(error.to_string()))?;
    let product_id = product_id.trim().to_ascii_uppercase();
    if product_id.is_empty()
        || product_id.len() > 32
        || !product_id.bytes().all(|byte| byte.is_ascii_alphanumeric())
    {
        return Err(XboxBridgeError::InvalidData(
            "invalid product identity".into(),
        ));
    }
    let response = XcloudApi::new(region.base_uri.clone(), access.bearer_token)
        .get_titles_by_ids(std::slice::from_ref(&product_id), "productId")
        .await
        .map_err(|error| crate::query_failure::query_error(error))?;
    let overlay = xbox_cloud_catalog_flow::build_overlay_load(
        &response,
        &json!({"results":[]}),
        &json!([]),
        false,
        false,
    )
    .map_err(|error| crate::query_failure::query_error(error))?;
    Ok(overlay
        .entries
        .into_iter()
        .find(|entry| entry.product_id.eq_ignore_ascii_case(&product_id))
        .map(|entry| XboxCloudGame {
            product_id: entry.product_id,
            stream_title_id: Some(entry.stream_title_id),
            xbox_title_id: entry.xbox_title_id.map(|id| id.to_string()),
            name: entry.fallback_name,
            publisher_name: String::new(),
            description: String::new(),
            tile_image_url: None,
            poster_image_url: None,
            hero_image_url: None,
            categories: Vec::new(),
            supported_input_types: entry.supported_input_types,
            has_entitlement: entry.entitlement_known.then_some(entry.has_entitlement),
            is_recently_played: None,
            is_new: None,
            recent_rank: None,
            new_rank: None,
            popular_rank: None,
            leaving_rank: None,
        }))
}
