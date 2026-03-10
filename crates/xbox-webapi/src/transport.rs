use crate::error::WebApiError;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, CONTENT_TYPE};
use reqwest::{Client, Method};
use serde_json::Value;
use std::time::Duration;
use tokio::time::sleep;

const DEFAULT_TIMEOUT_SECS: u64 = 12;
const MAX_RETRY_ATTEMPTS: usize = 3;
const BASE_RETRY_DELAY_MS: u64 = 250;
const MAX_RETRY_DELAY_MS: u64 = 1500;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RetryMode {
    Never,
    SafeOnly,
    Always,
}

impl Default for RetryMode {
    fn default() -> Self {
        RetryMode::SafeOnly
    }
}

#[derive(Debug, Clone)]
pub struct HttpTransport {
    client: Client,
    retry_mode: RetryMode,
}

impl HttpTransport {
    pub fn new() -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
                .build()
                .unwrap(),
            retry_mode: RetryMode::SafeOnly,
        }
    }

    pub fn with_timeout(timeout: Duration) -> Self {
        Self {
            client: Client::builder().timeout(timeout).build().unwrap(),
            retry_mode: RetryMode::SafeOnly,
        }
    }

    pub fn with_retry_mode(retry_mode: RetryMode) -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
                .build()
                .unwrap(),
            retry_mode,
        }
    }

    pub async fn request(
        &self,
        method: Method,
        url: &str,
        headers: Option<HeaderMap>,
        body: Option<Value>,
    ) -> Result<Value, WebApiError> {
        let content_type = headers
            .as_ref()
            .and_then(|header_map| header_map.get(CONTENT_TYPE))
            .and_then(|header_value| header_value.to_str().ok())
            .map(|value| value.to_ascii_lowercase());

        let method_clone = method.clone();
        let mut last_error: Option<WebApiError> = None;
        for attempt in 1..=MAX_RETRY_ATTEMPTS {
            let mut request = self.client.request(method.clone(), url);
            if let Some(request_headers) = &headers {
                request = request.headers(request_headers.clone());
            }

            if let Some(payload) = body.as_ref() {
                let is_non_json_string_body = payload.is_string()
                    && content_type
                        .as_deref()
                        .map(|value| !value.contains("json"))
                        .unwrap_or(false);

                if is_non_json_string_body {
                    let raw = payload.as_str().unwrap_or_default().to_string();
                    request = request.body(raw);
                } else {
                    request = request.json(payload);
                }
            }

            let response = match request.send().await {
                Ok(response) => response,
                Err(error) => {
                    let web_error = WebApiError::from(error);
                    if should_retry(&method_clone, &web_error, attempt, self.retry_mode) {
                        sleep(Duration::from_millis(compute_retry_delay_ms(attempt))).await;
                        last_error = Some(web_error);
                        continue;
                    }
                    return Err(web_error);
                }
            };

            let status = response.status();
            let text = match response.text().await {
                Ok(text) => text,
                Err(error) => {
                    let web_error = WebApiError::from(error);
                    if should_retry(&method_clone, &web_error, attempt, self.retry_mode) {
                        sleep(Duration::from_millis(compute_retry_delay_ms(attempt))).await;
                        last_error = Some(web_error);
                        continue;
                    }
                    return Err(web_error);
                }
            };

            if !status.is_success() {
                let web_error = WebApiError::http(status.as_u16(), text);
                if should_retry(&method_clone, &web_error, attempt, self.retry_mode) {
                    sleep(Duration::from_millis(compute_retry_delay_ms(attempt))).await;
                    last_error = Some(web_error);
                    continue;
                }
                return Err(web_error);
            }

            if text.trim().is_empty() {
                return Ok(Value::String(String::new()));
            }

            return serde_json::from_str(&text)
                .map_err(|error| WebApiError::parse(error.to_string()));
        }

        Err(last_error.unwrap_or_else(|| WebApiError::network("request failed after retries")))
    }

    pub async fn get(&self, url: &str, headers: Option<HeaderMap>) -> Result<Value, WebApiError> {
        self.request(Method::GET, url, headers, None).await
    }

    pub async fn post(
        &self,
        url: &str,
        body: Value,
        headers: Option<HeaderMap>,
    ) -> Result<Value, WebApiError> {
        self.request(Method::POST, url, headers, Some(body)).await
    }

    pub async fn delete(
        &self,
        url: &str,
        headers: Option<HeaderMap>,
    ) -> Result<Value, WebApiError> {
        self.request(Method::DELETE, url, headers, None).await
    }

    pub fn create_header_map(pairs: &[(&str, &str)]) -> Result<HeaderMap, WebApiError> {
        let mut headers = HeaderMap::new();
        for (key, value) in pairs {
            let header_name = HeaderName::from_bytes(key.as_bytes())
                .map_err(|e| WebApiError::parse(format!("Invalid header name: {}", e)))?;
            let header_value = HeaderValue::from_str(value)
                .map_err(|e| WebApiError::parse(format!("Invalid header value: {}", e)))?;
            headers.insert(header_name, header_value);
        }
        Ok(headers)
    }
}

fn should_retry(
    method: &Method,
    error: &WebApiError,
    attempt: usize,
    retry_mode: RetryMode,
) -> bool {
    if attempt >= MAX_RETRY_ATTEMPTS {
        return false;
    }

    match retry_mode {
        RetryMode::Never => false,
        RetryMode::Always => error.is_retriable(),
        RetryMode::SafeOnly => {
            if *method == Method::GET {
                error.is_retriable()
            } else {
                false
            }
        }
    }
}

fn compute_retry_delay_ms(attempt: usize) -> u64 {
    let exp = 1_u64 << attempt.saturating_sub(1);
    let delay = BASE_RETRY_DELAY_MS.saturating_mul(exp);
    delay.min(MAX_RETRY_DELAY_MS)
}

impl Default for HttpTransport {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{should_retry, RetryMode, MAX_RETRY_ATTEMPTS};
    use crate::error::WebApiError;
    use reqwest::Method;

    #[test]
    fn safe_only_retries_get_requests() {
        let error = WebApiError::network("timeout");

        assert!(should_retry(&Method::GET, &error, 1, RetryMode::SafeOnly));
        assert!(!should_retry(&Method::POST, &error, 1, RetryMode::SafeOnly));
        assert!(!should_retry(
            &Method::DELETE,
            &error,
            1,
            RetryMode::SafeOnly
        ));
    }

    #[test]
    fn never_mode_disables_retry() {
        let error = WebApiError::network("timeout");

        assert!(!should_retry(&Method::GET, &error, 1, RetryMode::Never));
        assert!(!should_retry(&Method::POST, &error, 1, RetryMode::Never));
    }

    #[test]
    fn always_mode_still_respects_retriable_flag() {
        let retriable = WebApiError::network("timeout");
        let non_retriable = WebApiError::parse("invalid json");

        assert!(should_retry(
            &Method::POST,
            &retriable,
            1,
            RetryMode::Always
        ));
        assert!(!should_retry(
            &Method::POST,
            &non_retriable,
            1,
            RetryMode::Always
        ));
    }

    #[test]
    fn retry_stops_at_max_attempts() {
        let error = WebApiError::network("timeout");

        assert!(!should_retry(
            &Method::GET,
            &error,
            MAX_RETRY_ATTEMPTS,
            RetryMode::SafeOnly
        ));
    }
}
