use crate::error::WebApiError;
use crate::transport::HttpTransport;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde_json::Value;

pub struct XcloudApi {
    transport: HttpTransport,
    host: String,
    bearer_token: String,
}

impl XcloudApi {
    pub fn new(host: String, bearer_token: String) -> Self {
        Self {
            transport: HttpTransport::for_queries(),
            host,
            bearer_token,
        }
    }

    pub fn with_transport(transport: HttpTransport, host: String, bearer_token: String) -> Self {
        Self {
            transport,
            host,
            bearer_token,
        }
    }

    fn authorization_header(&self) -> String {
        format!("Bearer {}", self.bearer_token)
    }

    pub async fn get_titles(&self) -> Result<Value, WebApiError> {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&self.authorization_header())
                .map_err(|e| WebApiError::parse(e.to_string()))?,
        );
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let url = titles_url(&self.host)?;
        self.transport.get(&url, Some(headers)).await
    }

    pub async fn get_recent_titles(&self, max_results: u32) -> Result<Value, WebApiError> {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&self.authorization_header())
                .map_err(|e| WebApiError::parse(e.to_string()))?,
        );
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let url = recent_titles_url(&self.host, max_results)?;
        self.transport.get(&url, Some(headers)).await
    }
}

fn normalized_origin(host: &str) -> Result<reqwest::Url, WebApiError> {
    let host = host.trim();
    if host.is_empty() || host.contains('\\') || host.chars().any(char::is_whitespace) {
        return Err(WebApiError::parse("invalid xCloud region host"));
    }
    let origin = if host.contains("://") {
        host.to_string()
    } else {
        format!("https://{host}")
    };
    let mut url = reqwest::Url::parse(&origin)
        .map_err(|_| WebApiError::parse("invalid xCloud region host"))?;
    if !matches!(url.scheme(), "https" | "http")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(WebApiError::parse("invalid xCloud region host"));
    }
    url.set_scheme("https")
        .map_err(|_| WebApiError::parse("invalid xCloud region scheme"))?;
    Ok(url)
}

fn titles_url(host: &str) -> Result<String, WebApiError> {
    let mut url = normalized_origin(host)?;
    url.set_path("/v2/titles");
    Ok(url.into())
}

fn recent_titles_url(host: &str, max_results: u32) -> Result<String, WebApiError> {
    let mut url = normalized_origin(host)?;
    url.set_path("/v2/titles/mru");
    url.query_pairs_mut()
        .append_pair("mr", &max_results.to_string());
    Ok(url.into())
}

#[cfg(test)]
mod tests {
    use super::{recent_titles_url, titles_url};

    #[test]
    fn builds_xcloud_catalog_urls() {
        assert_eq!(
            titles_url("https://wus.core.gssv-play-prod.xboxlive.com/").expect("titles url"),
            "https://wus.core.gssv-play-prod.xboxlive.com/v2/titles"
        );
        assert_eq!(
            recent_titles_url("wus.core.gssv-play-prod.xboxlive.com", 25)
                .expect("recent titles url"),
            "https://wus.core.gssv-play-prod.xboxlive.com/v2/titles/mru?mr=25"
        );
    }

    #[test]
    fn rejects_invalid_region_hosts() {
        for host in [
            "",
            "example.com/path",
            "example.com?token=x",
            "example.com#fragment",
            "https://user:secret@example.com",
            "user@example.com",
            "example.com\\path",
            "https://example.com\\@other.example",
            "ftp://example.com",
            "example .com",
        ] {
            assert!(titles_url(host).is_err(), "unexpected valid host: {host}");
        }
    }

    #[test]
    fn normalizes_supported_origin_and_builds_only_https_requests() {
        assert_eq!(
            titles_url(" http://example.com/ ").unwrap(),
            "https://example.com/v2/titles"
        );
        assert_eq!(
            titles_url("https://example.com:8443").unwrap(),
            "https://example.com:8443/v2/titles"
        );
    }
}
