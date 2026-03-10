use crate::mods::data::cache_repository::DataCacheRepository;
use crate::mods::data::types::{DataSessionContext, DataXcloudTitleSummary};
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, CONTENT_TYPE};
use reqwest::Client;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use xbox_webapi::XcloudApi;

const XCLOUD_TITLES_CACHE_TTL_MS: u64 = 10 * 60 * 1000;

pub struct XcloudService {
    client: Client,
}

impl XcloudService {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }

    pub async fn get_titles(
        &self,
        session: &DataSessionContext,
        cache_repository: &DataCacheRepository,
    ) -> Result<Vec<DataXcloudTitleSummary>, String> {
        if let Some(cached) = cache_repository.get_cached_xcloud_titles()? {
            let age = Self::now_ms().saturating_sub(cached.updated_at);
            if age <= XCLOUD_TITLES_CACHE_TTL_MS {
                return Ok(cached.titles);
            }

            match self.fetch_and_cache_titles(session, cache_repository).await {
                Ok(titles) if !titles.is_empty() => return Ok(titles),
                _ => return Ok(cached.titles),
            }
        }

        match self.fetch_and_cache_titles(session, cache_repository).await {
            Ok(titles) => Ok(titles),
            Err(_) => Ok(Vec::new()),
        }
    }

    async fn fetch_and_cache_titles(
        &self,
        session: &DataSessionContext,
        cache_repository: &DataCacheRepository,
    ) -> Result<Vec<DataXcloudTitleSummary>, String> {
        let Some(region) = Self::resolve_xcloud_region(session) else {
            return Ok(Vec::new());
        };
        // xCloud token 驱动的 region 请求复用 XcloudApi，避免 data 域重复实现。
        let xcloud_api = XcloudApi::new(region.host.clone(), region.bearer_token.clone());

        let streaming_titles_response = xcloud_api.get_titles().await.map_err(|e| e.to_string())?;

        let recent_titles_response = xcloud_api
            .get_recent_titles(25)
            .await
            .unwrap_or_else(|_| json!({ "results": [] }));

        let newest_titles_response = self
            .fetch_json_or_fallback(
                "https://catalog.gamepass.com/sigls/v2?id=f13cf6b4-57e6-4459-89df-6aec18cf0538&market=US&language=en-US",
                json!([]),
                None,
                None,
            )
            .await;

        let streaming_titles = Self::extract_streaming_titles(&streaming_titles_response);
        let mut live_title_map: HashMap<String, Value> = HashMap::new();
        for title in &streaming_titles {
            if let Some(product_id) = Self::normalize_product_id(
                title
                    .get("details")
                    .and_then(|value| value.get("productId"))
                    .and_then(|value| value.as_str()),
            ) {
                live_title_map.insert(product_id, title.clone());
            }
        }

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

        let catalog_products = self.load_catalog_products(&product_ids).await?;
        let recent_product_ids = Self::extract_recent_product_ids(&recent_titles_response);
        let newest_product_ids = Self::extract_newest_product_ids(&newest_titles_response);

        let mut titles = Vec::new();
        for product_id in product_ids {
            let live_title = live_title_map.get(&product_id);
            let catalog_product = catalog_products.get(&product_id);

            let resolved_title_id = Self::as_non_empty_string(
                live_title
                    .and_then(|title| title.get("titleId"))
                    .and_then(|value| value.as_str()),
            )
            .or_else(|| {
                Self::as_non_empty_string(
                    catalog_product
                        .and_then(|product| product.get("XCloudTitleId"))
                        .and_then(|value| value.as_str()),
                )
            })
            .unwrap_or_default();

            let resolved_name = Self::as_non_empty_string(
                catalog_product
                    .and_then(|product| product.get("ProductTitle"))
                    .and_then(|value| value.as_str()),
            )
            .or_else(|| {
                Self::as_non_empty_string(
                    live_title
                        .and_then(|title| title.get("titleId"))
                        .and_then(|value| value.as_str()),
                )
            })
            .unwrap_or_else(|| product_id.clone());

            if resolved_name.is_empty() || resolved_title_id.is_empty() {
                continue;
            }

            let categories = catalog_product
                .and_then(|product| {
                    product
                        .get("LocalizedCategories")
                        .and_then(|value| value.as_array())
                        .or_else(|| product.get("Categories").and_then(|value| value.as_array()))
                })
                .map(|values| {
                    Self::unique_strings(
                        values
                            .iter()
                            .filter_map(|value| value.as_str().map(|text| text.trim().to_string()))
                            .filter(|value| !value.is_empty())
                            .collect(),
                    )
                })
                .unwrap_or_default();

            let supported_input_types = live_title
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
                .unwrap_or_default();

            titles.push(DataXcloudTitleSummary {
                id: product_id.clone(),
                name: resolved_name,
                product_id: product_id.clone(),
                title_id: resolved_title_id,
                xbox_title_id: live_title
                    .and_then(|title| title.get("details"))
                    .and_then(|details| details.get("xboxTitleId"))
                    .and_then(Self::resolve_xbox_title_id)
                    .or_else(|| {
                        catalog_product
                            .and_then(|product| product.get("XboxTitleId"))
                            .and_then(Self::resolve_xbox_title_id)
                    }),
                publisher_name: catalog_product
                    .and_then(|product| product.get("PublisherName"))
                    .and_then(|value| value.as_str())
                    .unwrap_or_default()
                    .to_string(),
                description: catalog_product
                    .and_then(|product| product.get("ProductDescription"))
                    .and_then(|value| value.as_str())
                    .unwrap_or_default()
                    .to_string(),
                tile_image_url: Self::resolve_image_url(
                    catalog_product
                        .and_then(|product| product.get("Image_Tile"))
                        .and_then(|value| value.get("URL"))
                        .and_then(|value| value.as_str()),
                ),
                poster_image_url: Self::resolve_image_url(
                    catalog_product
                        .and_then(|product| product.get("Image_Poster"))
                        .and_then(|value| value.get("URL"))
                        .and_then(|value| value.as_str()),
                ),
                categories,
                supported_input_types,
                has_entitlement: live_title
                    .and_then(|title| title.get("details"))
                    .and_then(|details| details.get("hasEntitlement"))
                    .and_then(|value| value.as_bool())
                    .unwrap_or(true),
                is_recently_played: recent_product_ids.contains(&product_id),
                is_new: newest_product_ids.contains(&product_id),
            });
        }

        titles.sort_by(|left, right| left.name.cmp(&right.name));
        cache_repository.set_cached_xcloud_titles(titles.clone())?;
        Ok(titles)
    }

    async fn load_catalog_products(
        &self,
        product_ids: &[String],
    ) -> Result<HashMap<String, Value>, String> {
        if product_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let catalog_language = Self::resolve_catalog_language();
        let chunks = Self::chunk_values(product_ids, 75);
        let mut map = HashMap::new();

        for chunk in chunks {
            let response = self
                .fetch_json_or_fallback(
                    &format!(
                        "https://catalog.gamepass.com/v3/products?market=US&language={catalog_language}&hydration=RemoteLowJade0"
                    ),
                    json!({ "Products": {} }),
                    Some(Self::catalog_headers()?),
                    Some(json!({ "Products": chunk })),
                )
                .await;

            if let Some(products) = response.get("Products").and_then(|value| value.as_object()) {
                for (product_id, value) in products {
                    map.insert(product_id.to_uppercase(), value.clone());
                }
            }
        }

        Ok(map)
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

    fn now_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or(0)
    }
}

#[derive(Debug, Clone)]
struct ResolvedXcloudRegion {
    host: String,
    bearer_token: String,
}
