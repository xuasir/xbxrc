use crate::error::WebApiError;
use crate::transport::HttpTransport;
use serde_json::{json, Value};

const GAMEPASS_CATALOG_BASE_URL: &str = "https://catalog.gamepass.com";
const DEFAULT_PRODUCTS_HYDRATION: &str = "RemoteLowJade0";

pub struct GamePassApi {
    transport: HttpTransport,
}

impl GamePassApi {
    pub fn new() -> Self {
        Self {
            transport: HttpTransport::new(),
        }
    }

    pub fn with_transport(transport: HttpTransport) -> Self {
        Self { transport }
    }

    pub async fn get_sigl(
        &self,
        sigl_id: &str,
        market: &str,
        language: &str,
    ) -> Result<Value, WebApiError> {
        let url = sigl_url(sigl_id, market, language)?;
        self.transport.get(&url, Some(catalog_headers()?)).await
    }

    pub async fn get_products(
        &self,
        product_ids: &[String],
        market: &str,
        language: &str,
    ) -> Result<Value, WebApiError> {
        self.get_products_with_hydration(product_ids, market, language, DEFAULT_PRODUCTS_HYDRATION)
            .await
    }

    pub async fn get_products_with_hydration(
        &self,
        product_ids: &[String],
        market: &str,
        language: &str,
        hydration: &str,
    ) -> Result<Value, WebApiError> {
        let url = products_url(market, language, hydration)?;
        self.transport
            .post(
                &url,
                json!({ "Products": product_ids }),
                Some(catalog_headers()?),
            )
            .await
    }
}

impl Default for GamePassApi {
    fn default() -> Self {
        Self::new()
    }
}

fn sigl_url(sigl_id: &str, market: &str, language: &str) -> Result<String, WebApiError> {
    validate_query_value(sigl_id, "sigl id")?;
    validate_query_value(market, "market")?;
    validate_query_value(language, "language")?;
    Ok(format!(
        "{GAMEPASS_CATALOG_BASE_URL}/sigls/v2?id={sigl_id}&market={market}&language={language}"
    ))
}

fn products_url(market: &str, language: &str, hydration: &str) -> Result<String, WebApiError> {
    validate_query_value(market, "market")?;
    validate_query_value(language, "language")?;
    validate_query_value(hydration, "hydration")?;
    Ok(format!(
        "{GAMEPASS_CATALOG_BASE_URL}/v3/products?market={market}&language={language}&hydration={hydration}"
    ))
}

fn validate_query_value(value: &str, label: &str) -> Result<(), WebApiError> {
    let value = value.trim();
    if value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(WebApiError::parse(format!("invalid {label}")));
    }
    Ok(())
}

fn catalog_headers() -> Result<reqwest::header::HeaderMap, WebApiError> {
    HttpTransport::create_header_map(&[
        ("Accept", "application/json"),
        ("Content-Type", "application/json"),
        ("ms-cv", "0"),
        ("calling-app-name", "Xbox Cloud Gaming Web"),
        ("calling-app-version", "24.17.63"),
    ])
}

#[cfg(test)]
mod tests {
    use super::{products_url, sigl_url};

    #[test]
    fn builds_sigl_url() {
        assert_eq!(
            sigl_url("f13cf6b4-57e6-4459-89df-6aec18cf0538", "US", "zh-TW")
                .expect("sigl url"),
            "https://catalog.gamepass.com/sigls/v2?id=f13cf6b4-57e6-4459-89df-6aec18cf0538&market=US&language=zh-TW"
        );
    }

    #[test]
    fn builds_products_url() {
        assert_eq!(
            products_url("US", "en-US", "RemoteLowJade0").expect("products url"),
            "https://catalog.gamepass.com/v3/products?market=US&language=en-US&hydration=RemoteLowJade0"
        );
    }

    #[test]
    fn rejects_unsafe_query_values() {
        assert!(sigl_url("id&market=CN", "US", "en-US").is_err());
        assert!(products_url("US", "en-US?x=1", "RemoteLowJade0").is_err());
    }
}
