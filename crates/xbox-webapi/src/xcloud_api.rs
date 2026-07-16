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
            transport: HttpTransport::new(),
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

fn normalized_host(host: &str) -> Result<&str, WebApiError> {
    let host = host
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/');
    if host.is_empty()
        || host.contains('/')
        || host.contains('?')
        || host.contains('#')
        || host.chars().any(char::is_whitespace)
    {
        return Err(WebApiError::parse("invalid xCloud region host"));
    }
    Ok(host)
}

fn titles_url(host: &str) -> Result<String, WebApiError> {
    Ok(format!("https://{}/v2/titles", normalized_host(host)?))
}

fn recent_titles_url(host: &str, max_results: u32) -> Result<String, WebApiError> {
    Ok(format!(
        "https://{}/v2/titles/mru?mr={max_results}",
        normalized_host(host)?
    ))
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
        assert!(titles_url("").is_err());
        assert!(titles_url("example.com/path").is_err());
    }
}
