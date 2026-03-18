use crate::mods::data::cache_repository::{
    CachedXcloudCatalogBaseEntry, CachedXcloudCatalogOverlayEntry, DataCacheRepository,
};
use crate::mods::data::session_resolver::resolve_web_token_claims;
use crate::mods::data::types::{
    DataSessionContext, DataXcloudCatalogCacheState, DataXcloudCatalogPayload,
    XcloudCatalogCacheScope,
};
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, CONTENT_TYPE};
use reqwest::Client;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use xbox_webapi::XcloudApi;

const XCLOUD_CATALOG_DEFAULT_MARKET: &str = "US";

pub(crate) struct XcloudCatalogRefreshOutcome {
    pub payload: DataXcloudCatalogPayload,
    pub missing_product_count: usize,
}

pub struct XcloudService {
    client: Client,
}

impl XcloudService {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }

    pub fn resolve_cache_scope(
        &self,
        session: &DataSessionContext,
    ) -> Option<XcloudCatalogCacheScope> {
        let claims = resolve_web_token_claims(&session.web_token)?;
        let region = Self::resolve_xcloud_region(session)?;
        Some(XcloudCatalogCacheScope {
            account_id: claims.uhs,
            region_host: region.host,
            language: Self::resolve_catalog_language(),
            market: XCLOUD_CATALOG_DEFAULT_MARKET.to_string(),
        })
    }

    pub(crate) async fn refresh_catalog(
        &self,
        session: &DataSessionContext,
        scope: &XcloudCatalogCacheScope,
        cache_repository: &DataCacheRepository,
        _force_refresh: bool,
    ) -> Result<XcloudCatalogRefreshOutcome, String> {
        let Some(region) = Self::resolve_xcloud_region(session) else {
            return Ok(XcloudCatalogRefreshOutcome {
                payload: DataXcloudCatalogPayload {
                    titles: Vec::new(),
                    cache_state: DataXcloudCatalogCacheState::Miss,
                    updated_at: None,
                    refreshing: false,
                },
                missing_product_count: 0,
            });
        };

        let xcloud_api = XcloudApi::new(region.host.clone(), region.bearer_token.clone());

        let streaming_titles_response = xcloud_api.get_titles().await.map_err(|e| e.to_string())?;
        let recent_titles_response = xcloud_api
            .get_recent_titles(25)
            .await
            .unwrap_or_else(|_| json!({ "results": [] }));
        let newest_titles_response = self
            .fetch_json_or_fallback(
                &format!(
                    "https://catalog.gamepass.com/sigls/v2?id=f13cf6b4-57e6-4459-89df-6aec18cf0538&market={}&language={}",
                    scope.market, scope.language
                ),
                json!([]),
                None,
                None,
            )
            .await;

        let streaming_titles = Self::extract_streaming_titles(&streaming_titles_response);
        let product_ids = Self::unique_strings(
            streaming_titles
                .iter()
                .filter_map(|title| {
                    Self::normalize_product_id(
                        title
                            .get("details")
                            .and_then(|value| value.get("productId"))
                            .and_then(|value| value.as_str()),
                    )
                })
                .collect(),
        );

        let live_title_map = Self::build_live_title_map(&streaming_titles);
        let recent_product_ids = Self::extract_recent_product_ids(&recent_titles_response);
        let newest_product_ids = Self::extract_newest_product_ids(&newest_titles_response);
        let overlay_entries = product_ids
            .iter()
            .map(|product_id| {
                Self::build_overlay_entry(
                    product_id,
                    live_title_map.get(product_id),
                    &recent_product_ids,
                    &newest_product_ids,
                )
            })
            .filter(|entry| !entry.title_id.is_empty())
            .collect::<Vec<_>>();

        let cached_base_snapshot = cache_repository
            .get_xcloud_catalog_base_snapshot(scope)?
            .filter(|snapshot| !snapshot.entries.is_empty())
            .unwrap_or_default();
        let missing_product_ids = if cached_base_snapshot.entries.is_empty() {
            product_ids.clone()
        } else {
            product_ids
                .iter()
                .filter(|product_id| !cached_base_snapshot.entries.contains_key(*product_id))
                .cloned()
                .collect::<Vec<_>>()
        };

        let fetched_base_entries = if missing_product_ids.is_empty() {
            HashMap::new()
        } else {
            self.load_catalog_base_entries(scope, &missing_product_ids)
                .await?
        };

        let mut merged_base_entries = cached_base_snapshot.entries;
        for (product_id, entry) in fetched_base_entries {
            merged_base_entries.insert(product_id, entry);
        }

        cache_repository.save_xcloud_catalog_base(scope, merged_base_entries)?;
        cache_repository.save_xcloud_catalog_overlay(scope, overlay_entries)?;

        let loaded_snapshot = cache_repository.load_xcloud_catalog(scope)?;
        Ok(XcloudCatalogRefreshOutcome {
            payload: DataXcloudCatalogPayload {
                titles: loaded_snapshot.titles,
                cache_state: loaded_snapshot.cache_state,
                updated_at: loaded_snapshot.updated_at,
                refreshing: false,
            },
            missing_product_count: loaded_snapshot.missing_product_ids.len(),
        })
    }

    async fn load_catalog_base_entries(
        &self,
        scope: &XcloudCatalogCacheScope,
        product_ids: &[String],
    ) -> Result<HashMap<String, CachedXcloudCatalogBaseEntry>, String> {
        if product_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let chunks = Self::chunk_values(product_ids, 75);
        let mut entries = HashMap::new();

        for chunk in chunks {
            let response = self
                .fetch_json_or_fallback(
                    &format!(
                        "https://catalog.gamepass.com/v3/products?market={}&language={}&hydration=RemoteLowJade0",
                        scope.market, scope.language
                    ),
                    json!({ "Products": {} }),
                    Some(Self::catalog_headers()?),
                    Some(json!({ "Products": chunk })),
                )
                .await;

            if let Some(products) = response.get("Products").and_then(|value| value.as_object()) {
                for (product_id, value) in products {
                    entries.insert(
                        product_id.to_uppercase(),
                        CachedXcloudCatalogBaseEntry {
                            product_id: product_id.to_uppercase(),
                            name: value
                                .get("ProductTitle")
                                .and_then(|entry| entry.as_str())
                                .unwrap_or_default()
                                .to_string(),
                            publisher_name: value
                                .get("PublisherName")
                                .and_then(|entry| entry.as_str())
                                .unwrap_or_default()
                                .to_string(),
                            description: value
                                .get("ProductDescription")
                                .and_then(|entry| entry.as_str())
                                .unwrap_or_default()
                                .to_string(),
                            tile_image_url: Self::resolve_image_url(
                                value
                                    .get("Image_Tile")
                                    .and_then(|entry| entry.get("URL"))
                                    .and_then(|entry| entry.as_str()),
                            ),
                            poster_image_url: Self::resolve_image_url(
                                value
                                    .get("Image_Poster")
                                    .and_then(|entry| entry.get("URL"))
                                    .and_then(|entry| entry.as_str()),
                            ),
                            categories: value
                                .get("LocalizedCategories")
                                .and_then(|entry| entry.as_array())
                                .or_else(|| {
                                    value.get("Categories").and_then(|entry| entry.as_array())
                                })
                                .map(|entries| {
                                    Self::unique_strings(
                                        entries
                                            .iter()
                                            .filter_map(|entry| {
                                                entry.as_str().map(|value| value.trim().to_string())
                                            })
                                            .filter(|value| !value.is_empty())
                                            .collect(),
                                    )
                                })
                                .unwrap_or_default(),
                        },
                    );
                }
            }
        }

        Ok(entries)
    }

    fn build_live_title_map(streaming_titles: &[Value]) -> HashMap<String, Value> {
        let mut live_title_map = HashMap::new();
        for title in streaming_titles {
            if let Some(product_id) = Self::normalize_product_id(
                title
                    .get("details")
                    .and_then(|value| value.get("productId"))
                    .and_then(|value| value.as_str()),
            ) {
                live_title_map.insert(product_id, title.clone());
            }
        }
        live_title_map
    }

    fn build_overlay_entry(
        product_id: &str,
        live_title: Option<&Value>,
        recent_product_ids: &HashSet<String>,
        newest_product_ids: &HashSet<String>,
    ) -> CachedXcloudCatalogOverlayEntry {
        CachedXcloudCatalogOverlayEntry {
            product_id: product_id.to_string(),
            title_id: Self::as_non_empty_string(
                live_title
                    .and_then(|title| title.get("titleId"))
                    .and_then(|value| value.as_str()),
            )
            .unwrap_or_default(),
            xbox_title_id: live_title
                .and_then(|title| title.get("details"))
                .and_then(|details| details.get("xboxTitleId"))
                .and_then(Self::resolve_xbox_title_id),
            fallback_name: Self::as_non_empty_string(
                live_title
                    .and_then(|title| title.get("details"))
                    .and_then(|details| details.get("titleName"))
                    .and_then(|value| value.as_str()),
            )
            .or_else(|| {
                Self::as_non_empty_string(
                    live_title
                        .and_then(|title| title.get("titleId"))
                        .and_then(|value| value.as_str()),
                )
            })
            .unwrap_or_else(|| product_id.to_string()),
            supported_input_types: live_title
                .and_then(|title| title.get("details"))
                .and_then(|details| details.get("supportedInputTypes"))
                .and_then(|value| value.as_array())
                .map(|values| {
                    Self::unique_strings(
                        values
                            .iter()
                            .filter_map(|value| value.as_str().map(|text| text.to_string()))
                            .collect(),
                    )
                })
                .unwrap_or_default(),
            has_entitlement: live_title
                .and_then(|title| title.get("details"))
                .and_then(|details| details.get("hasEntitlement"))
                .and_then(|value| value.as_bool())
                .unwrap_or(true),
            is_recently_played: recent_product_ids.contains(product_id),
            is_new: newest_product_ids.contains(product_id),
        }
    }

    async fn fetch_json(
        &self,
        url: &str,
        headers: Option<HeaderMap>,
        body: Option<Value>,
    ) -> Result<Value, String> {
        let mut request = if body.is_some() {
            self.client.post(url)
        } else {
            self.client.get(url)
        };

        if let Some(headers) = headers {
            request = request.headers(headers);
        }

        if let Some(payload) = body {
            request = request.json(&payload);
        }

        let response = request.send().await.map_err(|error| error.to_string())?;
        if !response.status().is_success() {
            let status = response.status();
            let body_text = response.text().await.unwrap_or_default();
            return Err(format!("HTTP {status} for {url}: {body_text}"));
        }

        response
            .json::<Value>()
            .await
            .map_err(|error| error.to_string())
    }

    async fn fetch_json_or_fallback(
        &self,
        url: &str,
        fallback: Value,
        headers: Option<HeaderMap>,
        body: Option<Value>,
    ) -> Value {
        match self.fetch_json(url, headers, body).await {
            Ok(payload) => payload,
            Err(_) => fallback,
        }
    }

    fn resolve_xcloud_region(session: &DataSessionContext) -> Option<ResolvedXcloudRegion> {
        let token = session
            .streaming_tokens
            .get("xCloudToken")
            .or_else(|| session.streaming_tokens.get("xcloudToken"))?;
        let data = token.get("data").unwrap_or(token);

        let bearer_token = data.get("gsToken").and_then(|value| value.as_str())?;
        let regions = data
            .get("offeringSettings")
            .and_then(|value| value.get("regions"))
            .and_then(|value| value.as_array())?;

        let region = regions
            .iter()
            .find(|item| item.get("isDefault").and_then(|value| value.as_bool()) == Some(true))
            .or_else(|| regions.first())?;

        let base_uri = region.get("baseUri").and_then(|value| value.as_str())?;
        let host = base_uri
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_end_matches('/');

        if host.is_empty() {
            return None;
        }

        Some(ResolvedXcloudRegion {
            host: host.to_string(),
            bearer_token: bearer_token.to_string(),
        })
    }

    fn extract_streaming_titles(raw_response: &Value) -> Vec<Value> {
        raw_response
            .get("results")
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|item| {
                Self::normalize_product_id(
                    item.get("details")
                        .and_then(|value| value.get("productId"))
                        .and_then(|value| value.as_str()),
                )
                .is_some()
            })
            .collect()
    }

    fn extract_recent_product_ids(raw_response: &Value) -> HashSet<String> {
        let mut result = HashSet::new();
        let Some(items) = raw_response
            .get("results")
            .and_then(|value| value.as_array())
        else {
            return result;
        };

        for item in items {
            if let Some(product_id) = Self::normalize_product_id(
                item.get("details")
                    .and_then(|value| value.get("productId"))
                    .and_then(|value| value.as_str()),
            ) {
                result.insert(product_id);
            }
        }

        result
    }

    fn extract_newest_product_ids(raw_response: &Value) -> HashSet<String> {
        let mut result = HashSet::new();
        let Some(items) = raw_response.as_array() else {
            return result;
        };

        for item in items {
            if let Some(product_id) =
                Self::normalize_product_id(item.get("id").and_then(|value| value.as_str()))
            {
                result.insert(product_id);
            }
        }

        result
    }

    fn normalize_product_id(value: Option<&str>) -> Option<String> {
        value
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_uppercase())
    }

    fn as_non_empty_string(value: Option<&str>) -> Option<String> {
        value
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string())
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
        if let Some(number) = value.as_u64() {
            return Some(number);
        }

        value
            .as_str()
            .and_then(|raw| raw.parse::<u64>().ok())
            .filter(|value| *value > 0)
    }

    fn unique_strings(values: Vec<String>) -> Vec<String> {
        let mut set = HashSet::new();
        let mut result = Vec::new();

        for value in values {
            if set.insert(value.clone()) {
                result.push(value);
            }
        }

        result
    }

    fn chunk_values(values: &[String], size: usize) -> Vec<Vec<String>> {
        let mut chunks = Vec::new();
        let mut index = 0;

        while index < values.len() {
            let end = std::cmp::min(index + size, values.len());
            chunks.push(values[index..end].to_vec());
            index += size;
        }

        chunks
    }

    fn resolve_catalog_language() -> String {
        match std::env::var("LANG") {
            Ok(lang) if lang.to_lowercase().starts_with("zh") => "zh-TW".to_string(),
            _ => "en-US".to_string(),
        }
    }

    fn catalog_headers() -> Result<HeaderMap, String> {
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert("ms-cv", HeaderValue::from_static("0"));
        headers.insert(
            "calling-app-name",
            HeaderValue::from_static("Xbox Cloud Gaming Web"),
        );
        headers.insert("calling-app-version", HeaderValue::from_static("24.17.63"));
        Ok(headers)
    }
}

#[derive(Debug, Clone)]
struct ResolvedXcloudRegion {
    host: String,
    bearer_token: String,
}
