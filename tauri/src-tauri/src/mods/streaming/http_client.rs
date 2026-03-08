use reqwest::header::{HeaderMap, HeaderName, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use reqwest::Client;
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct StreamingHttpError {
    pub status: Option<u16>,
    pub body: Option<String>,
    pub message: String,
}

impl std::fmt::Display for StreamingHttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for StreamingHttpError {}

#[derive(Clone)]
pub struct StreamingHttpClient {
    host: String,
    bearer_token: String,
    client: Client,
}

impl StreamingHttpClient {
    pub fn new(host: String, bearer_token: String) -> Self {
        Self {
            host,
            bearer_token,
            client: Client::new(),
        }
    }

    pub async fn request_json(
        &self,
        method: &str,
        path: &str,
        body: Option<Value>,
        extra_headers: &[(&str, String)],
    ) -> Result<Value, StreamingHttpError> {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", self.bearer_token)).map_err(|error| {
                StreamingHttpError {
                    status: None,
                    body: None,
                    message: error.to_string(),
                }
            })?,
        );
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        for (key, value) in extra_headers {
            let name =
                HeaderName::from_bytes(key.as_bytes()).map_err(|error| StreamingHttpError {
                    status: None,
                    body: None,
                    message: error.to_string(),
                })?;
            headers.insert(
                name,
                HeaderValue::from_str(value).map_err(|error| StreamingHttpError {
                    status: None,
                    body: None,
                    message: error.to_string(),
                })?,
            );
        }

        let url = format!("https://{}{}", self.host, path);
        let mut request = match method {
            "GET" => self.client.get(&url),
            "POST" => self.client.post(&url),
            "DELETE" => self.client.delete(&url),
            _ => {
                return Err(StreamingHttpError {
                    status: None,
                    body: None,
                    message: format!("Unsupported method: {method}"),
                })
            }
        };

        request = request.headers(headers);
        if let Some(payload) = body {
            request = request.json(&payload);
        }

        let response = request.send().await.map_err(|error| StreamingHttpError {
            status: None,
            body: None,
            message: error.to_string(),
        })?;

        let status = response.status();
        let text = response.text().await.map_err(|error| StreamingHttpError {
            status: Some(status.as_u16()),
            body: None,
            message: error.to_string(),
        })?;

        if !status.is_success() {
            return Err(StreamingHttpError {
                status: Some(status.as_u16()),
                body: Some(text.clone()),
                message: format!("HTTP {} for {}", status.as_u16(), url),
            });
        }

        if text.trim().is_empty() {
            return Ok(Value::String(String::new()));
        }

        match serde_json::from_str::<Value>(&text) {
            Ok(value) => Ok(value),
            Err(_) => Ok(Value::String(text)),
        }
    }
}
