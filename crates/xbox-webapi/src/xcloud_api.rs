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

        let url = format!("https://{}/v2/titles", self.host);
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

        let url = format!("https://{}/v2/titles/mru?mr={}", self.host, max_results);
        self.transport.get(&url, Some(headers)).await
    }
}
