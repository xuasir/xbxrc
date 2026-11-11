use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::time::Duration;
use tokio::time::{timeout_at, Instant};
use xbox_webapi::{GamePassApi, WebApiError, XcloudApi};

pub const GAME_PASS_NEWEST_SIGL_ID: &str = "44a55037-770f-4bbf-bde5-a9fa27dba1da";
pub const DEFAULT_RECENT_TITLE_LIMIT: u32 = 25;
pub const PRODUCT_HYDRATION_CHUNK_SIZE: usize = 75;

#[derive(Debug, thiserror::Error)]
pub enum CloudCatalogFlowError {
    #[error(transparent)]
    WebApi(#[from] WebApiError),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudCatalogScope {
    pub market: String,
    pub language: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudCatalogBaseEntry {
    pub product_id: String,
    pub name: String,
    pub publisher_name: String,
    pub description: String,
    pub tile_image_url: String,
    pub poster_image_url: String,
    pub hero_image_url: String,
    pub categories: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudCatalogOverlayEntry {
    pub product_id: String,
    pub stream_title_id: String,
    pub xbox_title_id: Option<u64>,
    pub fallback_name: String,
    pub supported_input_types: Vec<String>,
    pub has_entitlement: bool,
    #[serde(default)]
    pub entitlement_known: bool,
    #[serde(default)]
    pub recent_rank: Option<usize>,
    #[serde(default)]
    pub new_rank: Option<usize>,
    pub is_recently_played: bool,
    pub is_new: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CloudCatalogOverlayLoad {
    pub product_ids: Vec<String>,
    pub entries: Vec<CloudCatalogOverlayEntry>,
    pub recent_source_available: bool,
    pub newest_source_available: bool,
    pub source_failures: Vec<CloudCatalogSourceFailure>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CloudCatalogHydrationLoad {
    pub entries: HashMap<String, CloudCatalogBaseEntry>,
    pub failed_chunk_count: usize,
    pub source_failures: Vec<CloudCatalogSourceFailure>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudCatalogSourceFailure {
    pub source: &'static str,
    pub error: Value,
}

fn source_failure(source: &'static str, error: &WebApiError) -> CloudCatalogSourceFailure {
    CloudCatalogSourceFailure {
        source,
        error: error.safe_summary(),
    }
}

pub struct XboxCloudCatalogFlow {
    xcloud_api: XcloudApi,
    gamepass_api: GamePassApi,
}

impl XboxCloudCatalogFlow {
    pub fn new(region_host: String, bearer_token: String) -> Self {
        Self {
            xcloud_api: XcloudApi::new(region_host, bearer_token),
            gamepass_api: GamePassApi::new(),
        }
    }

    pub async fn load_overlay(
        &self,
        scope: &CloudCatalogScope,
    ) -> Result<CloudCatalogOverlayLoad, CloudCatalogFlowError> {
        let streaming_titles_response = self.xcloud_api.get_titles().await?;
        extract_streaming_titles(&streaming_titles_response)?;
        let (recent, newest) = tokio::join!(
            self.xcloud_api
                .get_recent_titles(DEFAULT_RECENT_TITLE_LIMIT),
            self.gamepass_api
                .get_sigl(GAME_PASS_NEWEST_SIGL_ID, &scope.market, &scope.language),
        );
        let mut failures = Vec::new();
        let recent = recent.and_then(|value| {
            if value.get("results").is_some_and(Value::is_array) {
                Ok(value)
            } else {
                Err(WebApiError::parse("invalid recent titles response"))
            }
        });
        let newest = newest.and_then(|value| {
            if value.is_array() {
                Ok(value)
            } else {
                Err(WebApiError::parse("invalid Game Pass SIGL response"))
            }
        });
        if let Err(error) = &recent {
            failures.push(source_failure("recentTitles", error));
        }
        if let Err(error) = &newest {
            failures.push(source_failure("newestSigl", error));
        }
        let mut load = build_overlay_load(
            &streaming_titles_response,
            &recent
                .as_ref()
                .cloned()
                .unwrap_or_else(|_| json!({ "results": [] })),
            &newest.as_ref().cloned().unwrap_or_else(|_| json!([])),
            recent.is_ok(),
            newest.is_ok(),
        )?;
        load.source_failures = failures;
        Ok(load)
    }

    pub async fn hydrate_products(
        &self,
        scope: &CloudCatalogScope,
        product_ids: &[String],
    ) -> CloudCatalogHydrationLoad {
        hydrate_catalog_products(&self.gamepass_api, scope, product_ids).await
    }
}

// 商品资料同供云游戏与主机安装列表使用，共用分批和图片归一化。
pub async fn hydrate_catalog_products(
    api: &GamePassApi,
    scope: &CloudCatalogScope,
    product_ids: &[String],
) -> CloudCatalogHydrationLoad {
    hydrate_catalog_products_with_budget(api, scope, product_ids, Duration::from_secs(30)).await
}

/// 预算耗尽时保留已经完成的商品批次，供主机封面等可选资料渐进显示。
pub async fn hydrate_catalog_products_with_budget(
    api: &GamePassApi,
    scope: &CloudCatalogScope,
    product_ids: &[String],
    budget: Duration,
) -> CloudCatalogHydrationLoad {
    let api = api.clone();
    let scope = scope.clone();
    hydrate_product_chunks_with_budget(product_ids, budget, move |chunk| {
        let api = api.clone();
        let scope = scope.clone();
        async move {
            api.get_products(&chunk, &scope.market, &scope.language)
                .await
        }
    })
    .await
}

async fn hydrate_product_chunks_with_budget<F, Fut>(
    product_ids: &[String],
    budget: Duration,
    mut query: F,
) -> CloudCatalogHydrationLoad
where
    F: FnMut(Vec<String>) -> Fut,
    Fut: std::future::Future<Output = Result<Value, WebApiError>> + Send + 'static,
{
    let mut load = CloudCatalogHydrationLoad::default();
    let ids = unique_strings(
        product_ids
            .iter()
            .filter_map(|id| normalize_product_id(Some(id)))
            .collect(),
    );
    let chunks = chunk_product_ids(&ids, PRODUCT_HYDRATION_CHUNK_SIZE);
    let deadline = Instant::now() + budget;
    let mut completed = 0;
    let mut next = chunks.into_iter();
    let total = next.len();
    let mut tasks = tokio::task::JoinSet::new();
    loop {
        while tasks.len() < 4 {
            let Some(chunk) = next.next() else {
                break;
            };
            let response = query(chunk);
            tasks.spawn(async move {
                let response = response.await?;
                if !response
                    .get("Products")
                    .is_some_and(|value| value.is_array() || value.is_object())
                {
                    return Err(WebApiError::parse("invalid Game Pass products response"));
                }
                Ok(extract_product_entries(&response))
            });
        }
        if tasks.is_empty() {
            break;
        }
        let result = match timeout_at(deadline, tasks.join_next()).await {
            Ok(Some(result)) => result,
            _ => {
                load.failed_chunk_count += total - completed;
                load.source_failures.push(source_failure(
                    "products",
                    &WebApiError::network_kind(xbox_webapi::NetworkErrorKind::Timeout),
                ));
                break;
            }
        };
        completed += 1;
        match result.unwrap_or_else(|_| Err(WebApiError::network("product query canceled"))) {
            Ok(entries) => load.entries.extend(entries),
            Err(error) => {
                load.failed_chunk_count += 1;
                if load.source_failures.len() < 8 {
                    load.source_failures
                        .push(source_failure("products", &error));
                }
            }
        }
    }
    load
}

pub fn build_overlay_load(
    streaming_response: &Value,
    recent_response: &Value,
    newest_response: &Value,
    recent_source_available: bool,
    newest_source_available: bool,
) -> Result<CloudCatalogOverlayLoad, WebApiError> {
    let streaming_titles = extract_streaming_titles(streaming_response)?;
    let product_ids = unique_strings(
        streaming_titles
            .iter()
            .filter_map(|title| {
                normalize_product_id(
                    title
                        .get("details")
                        .and_then(|value| value.get("productId"))
                        .and_then(Value::as_str),
                )
            })
            .collect(),
    );
    let live_title_map = build_live_title_map(&streaming_titles);
    let recent_product_ids = extract_recent_product_ids(recent_response);
    let newest_product_ids = extract_newest_product_ids(newest_response);
    let entries = product_ids
        .iter()
        .map(|product_id| {
            build_overlay_entry(
                product_id,
                live_title_map.get(product_id),
                &recent_product_ids,
                &newest_product_ids,
            )
        })
        .filter(|entry| !entry.stream_title_id.is_empty())
        .collect();

    Ok(CloudCatalogOverlayLoad {
        product_ids,
        entries,
        recent_source_available,
        newest_source_available,
        source_failures: Vec::new(),
    })
}

fn extract_streaming_titles(raw_response: &Value) -> Result<Vec<Value>, WebApiError> {
    Ok(raw_response
        .get("results")
        .and_then(Value::as_array)
        .ok_or_else(|| WebApiError::parse("invalid cloud catalog titles response"))?
        .clone()
        .into_iter()
        .filter(|item| {
            normalize_product_id(
                item.get("details")
                    .and_then(|value| value.get("productId"))
                    .and_then(Value::as_str),
            )
            .is_some()
        })
        .collect())
}

fn build_live_title_map(streaming_titles: &[Value]) -> HashMap<String, Value> {
    streaming_titles
        .iter()
        .filter_map(|title| {
            normalize_product_id(
                title
                    .get("details")
                    .and_then(|value| value.get("productId"))
                    .and_then(Value::as_str),
            )
            .map(|product_id| (product_id, title.clone()))
        })
        .collect()
}

fn build_overlay_entry(
    product_id: &str,
    live_title: Option<&Value>,
    recent_product_ids: &[String],
    newest_product_ids: &[String],
) -> CloudCatalogOverlayEntry {
    CloudCatalogOverlayEntry {
        product_id: product_id.to_string(),
        stream_title_id: non_empty_string(
            live_title
                .and_then(|title| title.get("titleId"))
                .and_then(Value::as_str),
        )
        .unwrap_or_default(),
        xbox_title_id: live_title
            .and_then(|title| title.get("details"))
            .and_then(|details| details.get("xboxTitleId"))
            .and_then(resolve_xbox_title_id),
        fallback_name: non_empty_string(
            live_title
                .and_then(|title| title.get("details"))
                .and_then(|details| details.get("titleName"))
                .and_then(Value::as_str),
        )
        .or_else(|| {
            non_empty_string(
                live_title
                    .and_then(|title| title.get("titleId"))
                    .and_then(Value::as_str),
            )
        })
        .unwrap_or_else(|| product_id.to_string()),
        supported_input_types: live_title
            .and_then(|title| title.get("details"))
            .and_then(|details| details.get("supportedInputTypes"))
            .and_then(Value::as_array)
            .map(|values| {
                unique_strings(
                    values
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect(),
                )
            })
            .unwrap_or_default(),
        has_entitlement: live_title
            .and_then(|title| title.get("details"))
            .and_then(|details| details.get("hasEntitlement"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        entitlement_known: live_title
            .and_then(|title| title.get("details"))
            .and_then(|details| details.get("hasEntitlement"))
            .and_then(Value::as_bool)
            .is_some(),
        recent_rank: recent_product_ids.iter().position(|id| id == product_id),
        new_rank: newest_product_ids.iter().position(|id| id == product_id),
        is_recently_played: recent_product_ids.iter().any(|id| id == product_id),
        is_new: newest_product_ids.iter().any(|id| id == product_id),
    }
}

fn extract_recent_product_ids(raw_response: &Value) -> Vec<String> {
    raw_response
        .get("results")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            normalize_product_id(
                item.get("details")
                    .and_then(|value| value.get("productId"))
                    .and_then(Value::as_str),
            )
        })
        .collect()
}

pub fn extract_newest_product_ids(raw_response: &Value) -> Vec<String> {
    raw_response
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| normalize_product_id(item.get("id").and_then(Value::as_str)))
        .collect()
}

pub fn extract_product_entries(raw_response: &Value) -> HashMap<String, CloudCatalogBaseEntry> {
    let Some(products) = raw_response.get("Products") else {
        return HashMap::new();
    };

    if let Some(products) = products.as_object() {
        return products
            .iter()
            .filter_map(|(product_id, product)| {
                map_product_entry(Some(product_id.as_str()), product)
                    .map(|entry| (entry.product_id.clone(), entry))
            })
            .collect();
    }

    products
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|product| {
            let product_id = product
                .get("ProductId")
                .or_else(|| product.get("productId"))
                .and_then(Value::as_str);
            map_product_entry(product_id, product).map(|entry| (entry.product_id.clone(), entry))
        })
        .collect()
}

fn map_product_entry(product_id: Option<&str>, product: &Value) -> Option<CloudCatalogBaseEntry> {
    let product_id = normalize_product_id(product_id)?;
    let localized = product
        .get("LocalizedProperties")
        .and_then(Value::as_array)
        .and_then(|values| values.first());

    Some(CloudCatalogBaseEntry {
        product_id,
        name: product_string(product, localized, "ProductTitle"),
        publisher_name: product_string(product, localized, "PublisherName"),
        description: product_string(product, localized, "ProductDescription"),
        tile_image_url: product_image_url(product, "Image_Tile"),
        poster_image_url: product_image_url(product, "Image_Poster"),
        hero_image_url: [
            "Image_Hero",
            "Image_SuperHero",
            "Image_Background",
            "Image_BrandedKeyArt",
        ]
        .iter()
        .find_map(|key| {
            let url = product_image_url(product, key);
            (!url.is_empty()).then_some(url)
        })
        .unwrap_or_default(),
        categories: product
            .get("LocalizedCategories")
            .and_then(Value::as_array)
            .or_else(|| product.get("Categories").and_then(Value::as_array))
            .map(|entries| {
                unique_strings(
                    entries
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(str::to_string)
                        .collect(),
                )
            })
            .unwrap_or_default(),
    })
}

fn product_string(product: &Value, localized: Option<&Value>, key: &str) -> String {
    product
        .get(key)
        .and_then(Value::as_str)
        .or_else(|| {
            localized
                .and_then(|value| value.get(key))
                .and_then(Value::as_str)
        })
        .map(str::trim)
        .unwrap_or_default()
        .to_string()
}

fn product_image_url(product: &Value, key: &str) -> String {
    resolve_image_url(
        product
            .get(key)
            .and_then(|entry| entry.get("URL"))
            .and_then(Value::as_str),
    )
}

fn normalize_product_id(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_uppercase)
}

fn non_empty_string(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn resolve_image_url(value: Option<&str>) -> String {
    let Some(url) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return String::new();
    };
    if url.starts_with("//") {
        return format!("https:{url}");
    }
    url.to_string()
}

fn resolve_xbox_title_id(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|raw| raw.parse::<u64>().ok()))
        .filter(|value| *value > 0)
}

fn unique_strings(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .filter(|value| seen.insert(value.clone()))
        .collect()
}

fn chunk_product_ids(values: &[String], size: usize) -> Vec<Vec<String>> {
    if size == 0 {
        return Vec::new();
    }
    values.chunks(size).map(<[String]>::to_vec).collect()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudProductDetail {
    pub product: CloudCatalogBaseEntry,
    pub developer: String,
    pub release_date: String,
    pub rating: String,
    pub attributes: Vec<String>,
    pub screenshots: Vec<String>,
    pub platforms: Vec<String>,
    pub languages: Vec<CloudLanguageSupport>,
    pub accessibility: Vec<String>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudLanguageSupport {
    pub locale: String,
    pub interface: Option<u64>,
    pub audio: Option<u64>,
    pub subtitles: Option<u64>,
}
pub fn extract_product_detail(raw: &Value, id: &str) -> Option<CloudProductDetail> {
    let product = raw.get("Products")?.get(id)?;
    let base = map_product_entry(Some(id), product)?;
    let strings = |key: &str| {
        product
            .get(key)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect::<Vec<_>>()
    };
    let languages = product
        .get("LanguageSupport")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .map(|(locale, value)| CloudLanguageSupport {
            locale: locale.clone(),
            interface: value
                .get("InterfaceLanguageSupport")
                .and_then(Value::as_u64),
            audio: value
                .get("GamePlayAudioLanguageSupport")
                .and_then(Value::as_u64),
            subtitles: value
                .get("SubtitlesLanguageSupport")
                .and_then(Value::as_u64),
        })
        .collect();
    let accessibility = product
        .get("AccessibilityFeatures")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .flat_map(|(_, group)| group.as_object().into_iter().flatten())
        .filter_map(|(name, value)| {
            value
                .get("LocalizedName")
                .and_then(Value::as_str)
                .or_else(|| value.as_str())
                .map(str::to_string)
                .or_else(|| (value.as_bool() == Some(true)).then(|| name.clone()))
        })
        .filter(|name| !name.is_empty())
        .collect();
    Some(CloudProductDetail {
        product: base,
        developer: product_string(product, None, "DeveloperName"),
        release_date: product_string(product, None, "OriginalReleaseDate"),
        rating: product
            .get("ContentRating")
            .and_then(|v| v.get("LongName"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .into(),
        attributes: product
            .get("Attributes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|v| v.get("LocalizedName").and_then(Value::as_str))
            .filter(|v| !v.is_empty())
            .map(str::to_string)
            .collect(),
        screenshots: product
            .get("Screenshots")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|v| resolve_image_url(v.get("URL").and_then(Value::as_str)))
            .filter(|v| !v.is_empty())
            .collect(),
        platforms: strings("AvailablePlatforms"),
        languages,
        accessibility,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        build_overlay_load, chunk_product_ids, extract_product_entries,
        PRODUCT_HYDRATION_CHUNK_SIZE,
    };
    use serde_json::json;

    #[test]
    fn assembles_streaming_overlay_with_recent_and_new_markers() {
        let streaming = json!({
            "results": [
                {
                    "titleId": "stream-a",
                    "details": {
                        "productId": "9abc",
                        "xboxTitleId": "12345",
                        "titleName": "Alpha",
                        "supportedInputTypes": ["Controller", "Touch", "Controller"],
                        "hasEntitlement": false
                    }
                },
                {
                    "titleId": "stream-b",
                    "details": {
                        "productId": "9DEF",
                        "xboxTitleId": 67890,
                        "titleName": "Beta"
                    }
                },
                {
                    "details": { "productId": "9DROP", "titleName": "No stream id" }
                }
            ]
        });
        let recent = json!({ "results": [{ "details": { "productId": "9AbC" } }] });
        let newest = json!([{ "id": "9def" }]);

        let load = build_overlay_load(&streaming, &recent, &newest, true, true).unwrap();

        assert_eq!(load.product_ids, vec!["9ABC", "9DEF", "9DROP"]);
        assert_eq!(load.entries.len(), 2);
        assert_eq!(load.entries[0].stream_title_id, "stream-a");
        assert_eq!(load.entries[0].xbox_title_id, Some(12345));
        assert_eq!(
            load.entries[0].supported_input_types,
            vec!["Controller", "Touch"]
        );
        assert!(!load.entries[0].has_entitlement);
        assert!(load.entries[0].is_recently_played);
        assert!(load.entries[1].is_new);
    }

    #[test]
    fn parses_gamepass_product_metadata_and_normalizes_images() {
        let response = json!({
            "Products": {
                "9abc": {
                    "ProductTitle": " Alpha ",
                    "PublisherName": "Publisher",
                    "ProductDescription": "Description",
                    "Image_Tile": { "URL": "//images.example/tile.jpg" },
                    "Image_Poster": { "URL": "https://images.example/poster.jpg" },
                    "Image_Hero": { "URL": "//images.example/hero.jpg" },
                    "LocalizedCategories": ["Action", "Action", "RPG"]
                }
            }
        });

        let entries = extract_product_entries(&response);
        let entry = entries.get("9ABC").expect("product entry");
        assert_eq!(entry.name, "Alpha");
        assert_eq!(entry.tile_image_url, "https://images.example/tile.jpg");
        assert_eq!(entry.hero_image_url, "https://images.example/hero.jpg");
        assert_eq!(entry.categories, vec!["Action", "RPG"]);
    }

    #[test]
    fn parses_array_product_response_variant() {
        let response = json!({
            "Products": [{
                "ProductId": "9xyz",
                "LocalizedProperties": [{
                    "ProductTitle": "Array Product",
                    "PublisherName": "Publisher",
                    "ProductDescription": "Description"
                }]
            }]
        });

        let entries = extract_product_entries(&response);
        assert_eq!(entries["9XYZ"].name, "Array Product");
    }

    #[test]
    fn chunks_product_hydration_at_seventy_five_ids() {
        let values = (0..76).map(|index| format!("P{index}")).collect::<Vec<_>>();
        let chunks = chunk_product_ids(&values, PRODUCT_HYDRATION_CHUNK_SIZE);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].len(), 75);
        assert_eq!(chunks[1].len(), 1);
    }
}

#[cfg(test)]
#[path = "hydration.test.rs"]
mod hydration_tests;

#[cfg(test)]
mod ux_tests {
    use super::*;
    #[test]
    fn preserves_order_and_unknown_entitlement() {
        let titles = json!({"results":[{"titleId":"a","details":{"productId":"A"}},
            {"titleId":"b","details":{"productId":"B","hasEntitlement":false}},
            {"titleId":"c","details":{"productId":"C","hasEntitlement":true}}]});
        let recent =
            json!({"results":[{"details":{"productId":"C"}},{"details":{"productId":"A"}}]});
        let load = build_overlay_load(
            &titles,
            &recent,
            &json!([{"id":"B"},{"id":"A"}]),
            true,
            true,
        )
        .unwrap();
        assert_eq!(load.entries[0].recent_rank, Some(1));
        assert_eq!(load.entries[2].recent_rank, Some(0));
        assert_eq!(load.entries[0].new_rank, Some(1));
        assert!(!load.entries[0].entitlement_known);
        assert!(!load.entries[0].has_entitlement);
        assert!(load.entries[1].entitlement_known);
        assert!(!load.entries[1].has_entitlement);
        assert!(load.entries[2].has_entitlement);
    }
    #[test]
    fn reads_observed_rich_product_schema() {
        let raw: Value =
            serde_json::from_str(include_str!("../tests/fixtures/product-rich.json")).unwrap();
        let id = raw["Products"].as_object().unwrap().keys().next().unwrap();
        let detail = extract_product_detail(&raw, id).unwrap();
        assert!(!detail.product.description.is_empty());
        assert!(!detail.product.hero_image_url.is_empty());
        assert!(!detail.developer.is_empty());
        assert!(!detail.languages.is_empty());
        assert!(!detail.screenshots.is_empty());
        assert!(!detail.attributes.is_empty());
        assert!(detail.platforms.iter().any(|p| p == "XCloud"));
    }
}
