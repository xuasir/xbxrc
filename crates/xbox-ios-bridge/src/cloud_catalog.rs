use crate::cloud_access::{load_scoped_stream_access, StreamingAccessContext};
use crate::XboxBridgeError;
use std::time::{SystemTime, UNIX_EPOCH};
use xbox_cloud_catalog_flow::{CloudCatalogScope, XboxCloudCatalogFlow};
use xbox_streaming::Target;

const INITIAL_HYDRATION_SIZE: usize = 75;

#[derive(Debug, Clone, uniffi::Record)]
pub struct XboxCloudGame {
    pub product_id: String,
    pub stream_title_id: Option<String>,
    pub xbox_title_id: Option<String>,
    pub name: String,
    pub publisher_name: String,
    pub description: String,
    pub tile_image_url: Option<String>,
    pub poster_image_url: Option<String>,
    pub hero_image_url: Option<String>,
    pub categories: Vec<String>,
    pub supported_input_types: Vec<String>,
    pub has_entitlement: Option<bool>,
    pub is_recently_played: Option<bool>,
    pub is_new: Option<bool>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct XboxCloudCatalogSnapshot {
    pub games: Vec<XboxCloudGame>,
    pub account_id: String,
    pub region_host: String,
    pub market: String,
    pub language: String,
    pub fetched_at_ms: u64,
    pub failed_hydration_chunks: u32,
    pub pending_hydration_product_ids: Vec<String>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct XboxCloudCatalogMetadata {
    pub product_id: String,
    pub name: String,
    pub publisher_name: String,
    pub description: String,
    pub tile_image_url: Option<String>,
    pub poster_image_url: Option<String>,
    pub hero_image_url: Option<String>,
    pub categories: Vec<String>,
}

#[uniffi::export(async_runtime = "tokio")]
pub async fn fetch_cloud_catalog(
    access_handle: String,
    market: String,
    language: String,
) -> Result<XboxCloudCatalogSnapshot, XboxBridgeError> {
    let context = load_cloud_catalog_access(&access_handle)?;
    let market = normalize_locale_part(&market, "US");
    let language = normalize_locale_part(&language, "en-US");
    let scope = CloudCatalogScope {
        market: market.clone(),
        language: language.clone(),
    };
    let flow = XboxCloudCatalogFlow::new(context.host.clone(), context.bearer_token);
    let overlay = flow
        .load_overlay(&scope)
        .await
        .map_err(|error| XboxBridgeError::Data(error.to_string()))?;
    let initial_product_ids = overlay
        .product_ids
        .iter()
        .take(INITIAL_HYDRATION_SIZE)
        .cloned()
        .collect::<Vec<_>>();
    let mut pending_hydration_product_ids = overlay
        .product_ids
        .iter()
        .skip(INITIAL_HYDRATION_SIZE)
        .cloned()
        .collect::<Vec<_>>();
    let hydration = flow.hydrate_products(&scope, &initial_product_ids).await;
    if hydration.failed_chunk_count > 0 {
        pending_hydration_product_ids.splice(0..0, initial_product_ids);
    }
    let mut games = overlay
        .entries
        .into_iter()
        .map(|entry| {
            let base = hydration.entries.get(&entry.product_id);
            XboxCloudGame {
                product_id: entry.product_id.clone(),
                stream_title_id: non_empty(entry.stream_title_id),
                xbox_title_id: entry.xbox_title_id.map(|value| value.to_string()),
                name: base
                    .map(|value| value.name.clone())
                    .filter(|value| !value.is_empty())
                    .unwrap_or(entry.fallback_name),
                publisher_name: base
                    .map(|value| value.publisher_name.clone())
                    .unwrap_or_default(),
                description: base
                    .map(|value| value.description.clone())
                    .unwrap_or_default(),
                tile_image_url: base.and_then(|value| non_empty(value.tile_image_url.clone())),
                poster_image_url: base.and_then(|value| non_empty(value.poster_image_url.clone())),
                hero_image_url: base.and_then(|value| non_empty(value.hero_image_url.clone())),
                categories: base
                    .map(|value| value.categories.clone())
                    .unwrap_or_default(),
                supported_input_types: entry.supported_input_types,
                has_entitlement: Some(entry.has_entitlement),
                is_recently_played: Some(entry.is_recently_played),
                is_new: Some(entry.is_new),
            }
        })
        .collect::<Vec<_>>();
    games.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.product_id.cmp(&right.product_id))
    });

    Ok(XboxCloudCatalogSnapshot {
        games,
        account_id: context.account_id,
        region_host: context.host,
        market,
        language,
        fetched_at_ms: now_ms(),
        failed_hydration_chunks: hydration.failed_chunk_count as u32,
        pending_hydration_product_ids,
    })
}

#[uniffi::export(async_runtime = "tokio")]
pub async fn hydrate_cloud_catalog_page(
    access_handle: String,
    market: String,
    language: String,
    product_ids: Vec<String>,
) -> Result<Vec<XboxCloudCatalogMetadata>, XboxBridgeError> {
    if product_ids.len() > INITIAL_HYDRATION_SIZE {
        return Err(XboxBridgeError::InvalidData(format!(
            "catalog metadata page supports at most {INITIAL_HYDRATION_SIZE} products"
        )));
    }
    let context = load_cloud_catalog_access(&access_handle)?;
    let scope = CloudCatalogScope {
        market: normalize_locale_part(&market, "US"),
        language: normalize_locale_part(&language, "en-US"),
    };
    let flow = XboxCloudCatalogFlow::new(context.host, context.bearer_token);
    let hydration = flow.hydrate_products(&scope, &product_ids).await;
    if hydration.failed_chunk_count > 0 {
        return Err(XboxBridgeError::Data(
            "catalog metadata page hydration failed".to_string(),
        ));
    }
    let mut metadata = hydration
        .entries
        .into_values()
        .map(|entry| XboxCloudCatalogMetadata {
            product_id: entry.product_id,
            name: entry.name,
            publisher_name: entry.publisher_name,
            description: entry.description,
            tile_image_url: non_empty(entry.tile_image_url),
            poster_image_url: non_empty(entry.poster_image_url),
            hero_image_url: non_empty(entry.hero_image_url),
            categories: entry.categories,
        })
        .collect::<Vec<_>>();
    metadata.sort_by(|left, right| left.product_id.cmp(&right.product_id));
    Ok(metadata)
}

fn normalize_locale_part(value: &str, fallback: &str) -> String {
    let value = value.trim();
    if value.is_empty() || value.len() > 32 {
        fallback.to_string()
    } else {
        value.to_string()
    }
}

fn load_cloud_catalog_access(
    access_handle: &str,
) -> Result<StreamingAccessContext, XboxBridgeError> {
    load_scoped_stream_access(access_handle, Target::Cloud, None, None)
}

#[cfg(test)]
fn require_cloud_catalog_target(target: Target) -> Result<(), XboxBridgeError> {
    (target == Target::Cloud).then_some(()).ok_or_else(|| {
        XboxBridgeError::InvalidData(
            "cloud catalog requires a cloud stream access handle".to_string(),
        )
    })
}

fn non_empty(value: String) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locale_parts_use_bounded_defaults() {
        assert_eq!(normalize_locale_part("", "US"), "US");
        assert_eq!(normalize_locale_part(" zh-CN ", "en-US"), "zh-CN");
        assert_eq!(normalize_locale_part(&"a".repeat(33), "US"), "US");
    }

    #[test]
    fn non_empty_trims_values() {
        assert_eq!(
            non_empty(" https://example.com/image ".to_string()).as_deref(),
            Some("https://example.com/image")
        );
        assert_eq!(non_empty("   ".to_string()), None);
    }

    #[test]
    fn initial_hydration_keeps_first_page_bounded() {
        let ids = (0..1_000)
            .map(|index| format!("P{index:04}"))
            .collect::<Vec<_>>();
        assert_eq!(ids.iter().take(INITIAL_HYDRATION_SIZE).count(), 75);
        assert_eq!(ids.iter().skip(INITIAL_HYDRATION_SIZE).count(), 925);
    }

    #[test]
    fn cloud_catalog_rejects_home_access_target() {
        assert!(require_cloud_catalog_target(Target::Cloud).is_ok());
        assert!(require_cloud_catalog_target(Target::Home).is_err());
    }
}
