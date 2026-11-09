use crate::error::{NetworkErrorKind, WebApiError};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use reqwest::{Client, Method};
use serde_json::Value;
use std::time::Duration;
use std::{future::Future, pin::Pin, sync::Arc};
use tokio::time::{sleep, timeout_at, Instant};

const DEFAULT_TIMEOUT_SECS: u64 = 12;
const MAX_RETRY_ATTEMPTS: usize = 3;
const BASE_RETRY_DELAY_MS: u64 = 250;
const MAX_RETRY_DELAY_MS: u64 = 1500;
const QUERY_TOTAL_TIMEOUT_SECS: u64 = 18;
const QUERY_MAX_BODY_BYTES: usize = 16 * 1024 * 1024;
const ERROR_MAX_BODY_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Copy)]
struct RequestPolicy {
    total_timeout: Duration,
    max_body_bytes: usize,
}

#[derive(Default, Clone)]
struct RequestOptions {
    query_post: bool,
    paginated: bool,
    allow_empty: bool,
    command: bool,
}

pub type BearerRecoveryFuture = Pin<Box<dyn Future<Output = Result<String, WebApiError>> + Send>>;

pub trait BearerTokenRecovery: std::fmt::Debug + Send + Sync {
    fn recover(&self, rejected_token: &str) -> BearerRecoveryFuture;
}

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
    client: Result<Client, WebApiError>,
    retry_mode: RetryMode,
    policy: Option<RequestPolicy>,
    bearer_recovery: Option<Arc<dyn BearerTokenRecovery>>,
}

impl HttpTransport {
    pub fn with_client(client: Client) -> Self {
        Self {
            client: Ok(client),
            retry_mode: RetryMode::SafeOnly,
            policy: None,
            bearer_recovery: None,
        }
    }

    pub fn new() -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
                .build()
                .map_err(WebApiError::from_reqwest_safe),
            retry_mode: RetryMode::SafeOnly,
            policy: None,
            bearer_recovery: None,
        }
    }

    pub fn with_timeout(timeout: Duration) -> Self {
        Self {
            client: Client::builder()
                .timeout(timeout)
                .build()
                .map_err(WebApiError::from_reqwest_safe),
            retry_mode: RetryMode::SafeOnly,
            policy: None,
            bearer_recovery: None,
        }
    }

    pub fn with_retry_mode(retry_mode: RetryMode) -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
                .build()
                .map_err(WebApiError::from_reqwest_safe),
            retry_mode,
            policy: None,
            bearer_recovery: None,
        }
    }

    /// 非串流 API 的重试、响应体与总时长共用一份有限预算。
    pub fn for_queries() -> Self {
        Self {
            client: Client::builder()
                .connect_timeout(Duration::from_secs(5))
                .timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
                .build()
                .map_err(WebApiError::from_reqwest_safe),
            retry_mode: RetryMode::SafeOnly,
            policy: Some(RequestPolicy {
                total_timeout: Duration::from_secs(QUERY_TOTAL_TIMEOUT_SECS),
                max_body_bytes: QUERY_MAX_BODY_BYTES,
            }),
            bearer_recovery: None,
        }
    }

    pub fn for_commands() -> Self {
        let mut transport = Self::for_queries();
        transport.retry_mode = RetryMode::Never;
        transport
    }

    /// 仅为当前 Bearer 请求装配恢复；明确 401 拒绝后重新提交一次。
    pub fn with_bearer_recovery(mut self, recovery: Arc<dyn BearerTokenRecovery>) -> Self {
        self.bearer_recovery = Some(recovery);
        self
    }

    pub async fn request(
        &self,
        method: Method,
        url: &str,
        headers: Option<HeaderMap>,
        body: Option<Value>,
    ) -> Result<Value, WebApiError> {
        self.request_with_options(method, url, headers, body, RequestOptions::default())
            .await
    }

    async fn request_with_options(
        &self,
        method: Method,
        url: &str,
        headers: Option<HeaderMap>,
        body: Option<Value>,
        options: RequestOptions,
    ) -> Result<Value, WebApiError> {
        let deadline = self
            .policy
            .map(|policy| Instant::now() + policy.total_timeout);
        let request = async {
            let result = self
                .request_once_with_options(
                    method.clone(),
                    url,
                    headers.clone(),
                    body.clone(),
                    options.clone(),
                )
                .await;
            if !result
                .as_ref()
                .err()
                .is_some_and(|error| error.to_status_code() == Some(401))
            {
                return result;
            }
            let Some(recovery) = &self.bearer_recovery else {
                return result;
            };
            let Some(rejected) = headers
                .as_ref()
                .and_then(|headers| headers.get(AUTHORIZATION))
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.strip_prefix("Bearer "))
                .filter(|value| !value.trim().is_empty())
            else {
                return result;
            };
            let token = recovery.recover(rejected).await?;
            if token.trim().is_empty() {
                return Err(WebApiError::parse(
                    "credential renewal returned an empty token",
                ));
            }
            let authorization = HeaderValue::from_str(&format!("Bearer {token}"))
                .map_err(|_| WebApiError::parse("invalid renewed authorization header"))?;
            let mut headers = headers.unwrap_or_default();
            headers.insert(AUTHORIZATION, authorization);
            self.request_once_with_options(method, url, Some(headers), body, options)
                .await
        };
        match deadline {
            Some(deadline) => timeout_at(deadline, request)
                .await
                .unwrap_or_else(|_| Err(WebApiError::network_kind(NetworkErrorKind::Timeout))),
            None => request.await,
        }
    }

    async fn request_once_with_options(
        &self,
        method: Method,
        url: &str,
        headers: Option<HeaderMap>,
        body: Option<Value>,
        options: RequestOptions,
    ) -> Result<Value, WebApiError> {
        let client = self.client.as_ref().map_err(Clone::clone)?;
        if let Some(policy) = self.policy {
            return self
                .request_bounded(client, method, url, headers, body, options, policy)
                .await;
        }
        let retry_mode = if options.command {
            RetryMode::Never
        } else {
            self.retry_mode
        };
        let content_type = headers
            .as_ref()
            .and_then(|header_map| header_map.get(CONTENT_TYPE))
            .and_then(|header_value| header_value.to_str().ok())
            .map(|value| value.to_ascii_lowercase());

        let method_clone = method.clone();
        let mut last_error: Option<WebApiError> = None;
        for attempt in 1..=MAX_RETRY_ATTEMPTS {
            let mut request = client.request(method.clone(), url);
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
                    if should_retry(&method_clone, &web_error, attempt, retry_mode) {
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
                    if should_retry(&method_clone, &web_error, attempt, retry_mode) {
                        sleep(Duration::from_millis(compute_retry_delay_ms(attempt))).await;
                        last_error = Some(web_error);
                        continue;
                    }
                    return Err(web_error);
                }
            };

            if !status.is_success() {
                let web_error = WebApiError::http(status.as_u16(), text);
                if should_retry(&method_clone, &web_error, attempt, retry_mode) {
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

    async fn request_bounded(
        &self,
        client: &Client,
        method: Method,
        url: &str,
        headers: Option<HeaderMap>,
        body: Option<Value>,
        options: RequestOptions,
        policy: RequestPolicy,
    ) -> Result<Value, WebApiError> {
        let deadline = Instant::now() + policy.total_timeout;
        let retry_mode = if options.command {
            RetryMode::Never
        } else if options.query_post && self.retry_mode != RetryMode::Never {
            RetryMode::Always
        } else {
            self.retry_mode
        };
        for attempt in 1..=MAX_RETRY_ATTEMPTS {
            let mut request = client.request(method.clone(), url);
            if let Some(headers) = &headers {
                request = request.headers(headers.clone());
            }
            if let Some(body) = &body {
                let raw_body = body.is_string()
                    && headers
                        .as_ref()
                        .and_then(|headers| headers.get(CONTENT_TYPE))
                        .and_then(|value| value.to_str().ok())
                        .is_some_and(|value| !value.to_ascii_lowercase().contains("json"));
                request = if raw_body {
                    request.body(body.as_str().unwrap_or_default().to_string())
                } else {
                    request.json(body)
                };
            }
            let response = timeout_at(deadline, request.send()).await;
            let (result, retry_after) = match response {
                Err(_) => (
                    Err(WebApiError::network_kind(NetworkErrorKind::Timeout)),
                    None,
                ),
                Ok(Err(error)) => (Err(WebApiError::from_reqwest_safe(error)), None),
                Ok(Ok(response)) => {
                    let status = response.status();
                    let response_headers = response.headers().clone();
                    let retry_after = response_headers
                        .get("retry-after")
                        .and_then(|value| value.to_str().ok())
                        .and_then(|value| parse_retry_after(value, chrono::Utc::now()));
                    let limit = if status.is_success() {
                        policy.max_body_bytes
                    } else {
                        ERROR_MAX_BODY_BYTES
                    };
                    let bytes = timeout_at(deadline, read_bounded_body(response, limit))
                        .await
                        .unwrap_or_else(|_| {
                            Err(WebApiError::network_kind(NetworkErrorKind::Timeout))
                        });
                    let result = if status.is_success() {
                        bytes.and_then(|bytes| {
                            if bytes.iter().all(u8::is_ascii_whitespace) {
                                return if options.allow_empty {
                                    Ok(Value::Null)
                                } else {
                                    Err(WebApiError::parse("empty JSON response"))
                                };
                            }
                            let mut value: Value = serde_json::from_slice(&bytes)
                                .map_err(|_| WebApiError::parse("invalid JSON response"))?;
                            if options.paginated {
                                merge_continuation_header(&mut value, &response_headers);
                            }
                            Ok(value)
                        })
                    } else {
                        // 已取得的 HTTP 状态优先于错误正文读取失败，保证 401/403 可准确处理。
                        let body = bytes
                            .ok()
                            .and_then(|bytes| String::from_utf8(bytes).ok())
                            .unwrap_or_default();
                        Err(WebApiError::http_response(status.as_u16(), &body))
                    };
                    (result, retry_after)
                }
            };
            match result {
                Ok(value) => return Ok(value),
                Err(error) => {
                    if !should_retry(&method, &error, attempt, retry_mode) {
                        return Err(error);
                    }
                    let delay = retry_after
                        .unwrap_or_default()
                        .max(Duration::from_millis(compute_retry_delay_ms(attempt)));
                    if delay >= deadline.saturating_duration_since(Instant::now()) {
                        return Err(error);
                    }
                    sleep(delay).await;
                }
            }
        }
        Err(WebApiError::network_kind(NetworkErrorKind::Timeout))
    }

    pub async fn get_paginated(
        &self,
        url: &str,
        headers: Option<HeaderMap>,
    ) -> Result<Value, WebApiError> {
        self.request_with_options(
            Method::GET,
            url,
            headers,
            None,
            RequestOptions {
                paginated: true,
                ..Default::default()
            },
        )
        .await
    }

    /// 仅用于统计、商品和媒体搜索等无副作用的 POST 查询。
    pub async fn post_query(
        &self,
        url: &str,
        body: Value,
        headers: Option<HeaderMap>,
    ) -> Result<Value, WebApiError> {
        self.request_with_options(
            Method::POST,
            url,
            headers,
            Some(body),
            RequestOptions {
                query_post: true,
                ..Default::default()
            },
        )
        .await
    }

    pub async fn post_paginated(
        &self,
        url: &str,
        body: Value,
        headers: Option<HeaderMap>,
    ) -> Result<Value, WebApiError> {
        self.request_with_options(
            Method::POST,
            url,
            headers,
            Some(body),
            RequestOptions {
                query_post: true,
                paginated: true,
                ..Default::default()
            },
        )
        .await
    }

    pub async fn post_command(
        &self,
        url: &str,
        body: Value,
        headers: Option<HeaderMap>,
    ) -> Result<Value, WebApiError> {
        self.request_with_options(
            Method::POST,
            url,
            headers,
            Some(body),
            RequestOptions {
                allow_empty: true,
                command: true,
                ..Default::default()
            },
        )
        .await
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

async fn read_bounded_body(
    mut response: reqwest::Response,
    limit: usize,
) -> Result<Vec<u8>, WebApiError> {
    if response
        .content_length()
        .is_some_and(|size| size > limit as u64)
    {
        return Err(WebApiError::parse("response exceeds size limit"));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(WebApiError::from_reqwest_safe)?
    {
        if bytes.len().saturating_add(chunk.len()) > limit {
            return Err(WebApiError::parse("response exceeds size limit"));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn parse_retry_after(value: &str, now: chrono::DateTime<chrono::Utc>) -> Option<Duration> {
    let value = value.trim();
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    let date = chrono::DateTime::parse_from_rfc2822(value).ok()?;
    Some(
        (date.with_timezone(&chrono::Utc) - now)
            .to_std()
            .unwrap_or_default(),
    )
}

fn merge_continuation_header(value: &mut Value, headers: &HeaderMap) {
    let token = [
        "x-continuation-token",
        "x-xbl-continuation-token",
        "x-ms-continuation",
    ]
    .into_iter()
    .find_map(|name| headers.get(name).and_then(|value| value.to_str().ok()))
    .filter(|value| !value.is_empty());
    let Some(token) = token else {
        return;
    };
    let root = if value.get("data").is_some_and(Value::is_object) {
        value.get_mut("data").unwrap()
    } else {
        value
    };
    if crate::library::library_continuation(root).is_none() {
        if let Some(root) = root.as_object_mut() {
            root.insert(
                "continuationToken".to_string(),
                Value::String(token.to_string()),
            );
        }
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
#[path = "transport.test.rs"]
mod request_tests;

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
