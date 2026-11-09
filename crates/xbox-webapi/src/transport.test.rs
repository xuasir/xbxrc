use super::*;
use serde_json::json;
use std::collections::VecDeque;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[derive(Clone)]
struct Reply {
    status: u16,
    body: String,
    headers: String,
    delay: Duration,
    length: Option<usize>,
}

impl Reply {
    fn new(status: u16, body: &str) -> Self {
        Self {
            status,
            body: body.into(),
            headers: String::new(),
            delay: Duration::ZERO,
            length: None,
        }
    }
    fn header(mut self, header: &str) -> Self {
        self.headers = format!("{header}\r\n");
        self
    }
}

struct Server {
    url: String,
    count: Arc<AtomicUsize>,
    requests: Arc<Mutex<Vec<String>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Server {
    async fn start(replies: Vec<Reply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!(
            "http://{}/query?private=secret",
            listener.local_addr().unwrap()
        );
        let count = Arc::new(AtomicUsize::new(0));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let replies = Arc::new(Mutex::new(VecDeque::from(replies)));
        let task_count = count.clone();
        let task_requests = requests.clone();
        let task = tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let replies = replies.clone();
                let count = task_count.clone();
                let requests = task_requests.clone();
                tokio::spawn(async move {
                    let mut request = Vec::new();
                    loop {
                        let mut chunk = [0; 4096];
                        let Ok(size) = socket.read(&mut chunk).await else {
                            return;
                        };
                        if size == 0 {
                            return;
                        }
                        request.extend_from_slice(&chunk[..size]);
                        if let Some(end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n")
                        {
                            let headers =
                                String::from_utf8_lossy(&request[..end]).to_ascii_lowercase();
                            let length = headers
                                .lines()
                                .find_map(|line| {
                                    line.strip_prefix("content-length:")
                                        .and_then(|v| v.trim().parse::<usize>().ok())
                                })
                                .unwrap_or(0);
                            if request.len() >= end + 4 + length {
                                break;
                            }
                        }
                    }
                    requests
                        .lock()
                        .unwrap()
                        .push(String::from_utf8_lossy(&request).into());
                    count.fetch_add(1, Ordering::SeqCst);
                    let reply = replies
                        .lock()
                        .unwrap()
                        .pop_front()
                        .unwrap_or_else(|| Reply::new(500, "{}"));
                    let header = format!("HTTP/1.1 {} Status\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n{}\r\n", reply.status, reply.length.unwrap_or(reply.body.len()), reply.headers);
                    let _ = socket.write_all(header.as_bytes()).await;
                    sleep(reply.delay).await;
                    let _ = socket.write_all(reply.body.as_bytes()).await;
                });
            }
        });
        Self {
            url,
            count,
            requests,
            task,
        }
    }
    fn attempts(&self) -> usize {
        self.count.load(Ordering::SeqCst)
    }
}

fn transport(total_ms: u64, limit: usize) -> HttpTransport {
    HttpTransport {
        client: Ok(Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap()),
        retry_mode: RetryMode::SafeOnly,
        policy: Some(RequestPolicy {
            total_timeout: Duration::from_millis(total_ms),
            max_body_bytes: limit,
        }),
        bearer_recovery: None,
    }
}

struct Renewal {
    calls: Arc<AtomicUsize>,
    rejected: Arc<Mutex<Vec<String>>>,
    result: Result<String, WebApiError>,
    delay: Duration,
}

impl std::fmt::Debug for Renewal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TestRenewal")
    }
}

impl BearerTokenRecovery for Renewal {
    fn recover(&self, token: &str) -> BearerRecoveryFuture {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.rejected.lock().unwrap().push(token.to_string());
        let result = self.result.clone();
        let delay = self.delay;
        Box::pin(async move {
            sleep(delay).await;
            result
        })
    }
}

fn renewal(result: Result<&str, WebApiError>, delay: Duration) -> Arc<Renewal> {
    Arc::new(Renewal {
        calls: Arc::new(AtomicUsize::new(0)),
        rejected: Arc::new(Mutex::new(Vec::new())),
        result: result.map(str::to_string),
        delay,
    })
}

fn bearer_headers() -> HeaderMap {
    HttpTransport::create_header_map(&[
        ("Authorization", "Bearer old-token"),
        ("X-Request-Id", "same-operation"),
    ])
    .unwrap()
}

#[tokio::test]
async fn rejected_bearer_command_renews_once_and_preserves_method_headers_and_body() {
    let server = Server::start(vec![Reply::new(401, "{}"), Reply::new(200, "{}")]).await;
    let recovery = renewal(Ok("new-token"), Duration::ZERO);
    let api = transport(1000, 1024).with_bearer_recovery(recovery.clone());
    let body = json!({"session": "same-session", "payload": {"text": "exact value"}});
    api.post_command(&server.url, body.clone(), Some(bearer_headers()))
        .await
        .unwrap();
    assert_eq!(server.attempts(), 2);
    assert_eq!(recovery.calls.load(Ordering::SeqCst), 1);
    assert_eq!(&*recovery.rejected.lock().unwrap(), &["old-token"]);
    let requests = server.requests.lock().unwrap();
    for (index, request) in requests.iter().enumerate() {
        assert!(request.starts_with("POST /query?private=secret HTTP/1.1"));
        let (headers, request_body) = request.split_once("\r\n\r\n").unwrap();
        let headers = headers.to_ascii_lowercase();
        assert!(headers.contains("x-request-id: same-operation"));
        assert!(headers.contains(if index == 0 {
            "authorization: bearer old-token"
        } else {
            "authorization: bearer new-token"
        }));
        assert_eq!(serde_json::from_str::<Value>(request_body).unwrap(), body);
    }
}

#[tokio::test]
async fn bearer_recovery_preserves_raw_form_payload() {
    let server = Server::start(vec![Reply::new(401, "{}"), Reply::new(200, "{}")]).await;
    let recovery = renewal(Ok("new-token"), Duration::ZERO);
    let mut headers = bearer_headers();
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/x-www-form-urlencoded"),
    );
    transport(1000, 1024)
        .with_bearer_recovery(recovery)
        .post(
            &server.url,
            json!("value=exact%20value&request=unchanged"),
            Some(headers),
        )
        .await
        .unwrap();
    for request in server.requests.lock().unwrap().iter() {
        assert_eq!(
            request.split_once("\r\n\r\n").unwrap().1,
            "value=exact%20value&request=unchanged"
        );
    }
}

#[tokio::test]
async fn repeated_unauthorized_or_followup_command_failure_stops_after_one_renewal() {
    for status in [401, 503] {
        let server = Server::start(vec![Reply::new(401, "{}"), Reply::new(status, "{}")]).await;
        let recovery = renewal(Ok("new-token"), Duration::ZERO);
        let error = transport(1000, 1024)
            .with_bearer_recovery(recovery.clone())
            .post_command(&server.url, json!({}), Some(bearer_headers()))
            .await
            .unwrap_err();
        assert_eq!(error.to_status_code(), Some(status));
        assert_eq!(server.attempts(), 2);
        assert_eq!(recovery.calls.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn permission_and_transport_errors_keep_their_category_and_skip_auth_recovery() {
    for status in [403, 429, 503] {
        let server = Server::start(vec![Reply::new(status, "{}")]).await;
        let recovery = renewal(Ok("new-token"), Duration::ZERO);
        let mut api = transport(1000, 1024).with_bearer_recovery(recovery.clone());
        api.retry_mode = RetryMode::Never;
        assert_eq!(
            api.get(&server.url, Some(bearer_headers()))
                .await
                .unwrap_err()
                .to_status_code(),
            Some(status)
        );
        assert_eq!(recovery.calls.load(Ordering::SeqCst), 0);
        assert_eq!(server.attempts(), 1);
    }
    let server = Server::start(vec![]).await;
    let url = server.url.clone();
    drop(server);
    let recovery = renewal(Ok("new-token"), Duration::ZERO);
    let mut api = transport(1000, 1024).with_bearer_recovery(recovery.clone());
    api.retry_mode = RetryMode::Never;
    assert!(matches!(
        api.get(&url, Some(bearer_headers())).await,
        Err(WebApiError::Network { .. })
    ));
    assert_eq!(recovery.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn recovery_failure_preserves_auth_network_and_rate_limit_results() {
    for error in [
        WebApiError::auth("renewal expired"),
        WebApiError::network_kind(NetworkErrorKind::Timeout),
        WebApiError::http(429, "limited"),
    ] {
        let server = Server::start(vec![Reply::new(401, "{}")]).await;
        let recovery = renewal(Err(error.clone()), Duration::ZERO);
        let result = transport(1000, 1024)
            .with_bearer_recovery(recovery.clone())
            .get(&server.url, Some(bearer_headers()))
            .await
            .unwrap_err();
        assert_eq!(result.safe_summary(), error.safe_summary());
        assert_eq!(recovery.calls.load(Ordering::SeqCst), 1);
        assert_eq!(server.attempts(), 1);
    }
}

#[tokio::test]
async fn recovery_and_replayed_response_share_the_original_total_budget() {
    let mut first = Reply::new(401, "{}");
    first.delay = Duration::from_millis(50);
    let mut second = Reply::new(200, "{}");
    second.delay = Duration::from_millis(250);
    let server = Server::start(vec![first, second]).await;
    let recovery = renewal(Ok("new-token"), Duration::from_millis(50));
    let started = Instant::now();
    let error = transport(200, 1024)
        .with_bearer_recovery(recovery.clone())
        .get(&server.url, Some(bearer_headers()))
        .await
        .unwrap_err();
    assert_eq!(error.network_error_kind(), Some(NetworkErrorKind::Timeout));
    assert!(started.elapsed() < Duration::from_millis(330));
    assert_eq!(server.attempts(), 2);
    assert_eq!(recovery.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_missing_bearer_header_and_invalid_renewed_payload_avoid_replaying() {
    for header in [
        None,
        Some(
            HttpTransport::create_header_map(&[("Authorization", "XBL3.0 unrelated-token")])
                .unwrap(),
        ),
    ] {
        let server = Server::start(vec![Reply::new(401, "{}")]).await;
        let recovery = renewal(Ok("new-token"), Duration::ZERO);
        assert_eq!(
            transport(1000, 1024)
                .with_bearer_recovery(recovery.clone())
                .get(&server.url, header)
                .await
                .unwrap_err()
                .to_status_code(),
            Some(401)
        );
        assert_eq!(recovery.calls.load(Ordering::SeqCst), 0);
    }
    let server = Server::start(vec![Reply::new(401, "{}")]).await;
    let error = transport(1000, 1024)
        .with_bearer_recovery(renewal(Ok(" "), Duration::ZERO))
        .get(&server.url, Some(bearer_headers()))
        .await
        .unwrap_err();
    assert!(matches!(error, WebApiError::Parse { .. }));
    assert_eq!(server.attempts(), 1);
}

#[tokio::test]
async fn service_rejections_preserve_status_and_safe_reason_with_one_attempt() {
    for status in [401, 403, 404] {
        let server = Server::start(vec![Reply::new(status, r#"{"error":{"code":"Forbidden","message":"UnsupportedCountry"},"token":"private-secret"}"#)]).await;
        let error = transport(1500, 1024)
            .get(&server.url, None)
            .await
            .unwrap_err();
        assert_eq!(error.to_status_code(), Some(status));
        assert_eq!(
            error.safe_service_code().as_deref(),
            Some("UnsupportedCountry")
        );
        assert_eq!(server.attempts(), 1);
        assert!(!format!("{error:?} {error}").contains("private-secret"));
        assert!(!error.to_string().contains("http://"));
    }
}

#[tokio::test]
async fn retry_after_preserves_rate_limit_when_server_delay_exceeds_budget() {
    let server = Server::start(vec![Reply::new(429, "{}").header("Retry-After: 60")]).await;
    let start = Instant::now();
    let error = transport(1000, 1024)
        .get(&server.url, None)
        .await
        .unwrap_err();
    assert_eq!(error.to_status_code(), Some(429));
    assert_eq!(server.attempts(), 1);
    assert!(start.elapsed() < Duration::from_millis(500));
    // 极大值同样只返回限流事实，避免计算 Instant 时溢出。
    assert_eq!(
        parse_retry_after("18446744073709551615", chrono::Utc::now()),
        Some(Duration::from_secs(u64::MAX))
    );
}

#[tokio::test]
async fn honors_server_retry_after_before_retrying_safe_query() {
    let server = Server::start(vec![
        Reply::new(429, "{}").header("Retry-After: 1"),
        Reply::new(200, "{\"ok\":true}"),
    ])
    .await;
    let start = Instant::now();
    assert_eq!(
        transport(2000, 1024).get(&server.url, None).await.unwrap(),
        json!({"ok":true})
    );
    assert!(start.elapsed() >= Duration::from_secs(1));
    assert_eq!(server.attempts(), 2);
}

#[tokio::test]
async fn safe_post_retries_while_commands_and_auth_posts_submit_once() {
    let server = Server::start(vec![Reply::new(503, "{}"), Reply::new(200, "{}")]).await;
    transport(1500, 1024)
        .post_query(&server.url, json!({"query":"test"}), None)
        .await
        .unwrap();
    assert_eq!(server.attempts(), 2);
    assert!(server
        .requests
        .lock()
        .unwrap()
        .iter()
        .all(|request| request.starts_with("POST ") && request.ends_with(r#"{"query":"test"}"#)));
    for command in [false, true] {
        let server = Server::start(vec![Reply::new(503, "{}"), Reply::new(200, "{}")]).await;
        let mut client = transport(1500, 1024);
        if command {
            client.retry_mode = RetryMode::Always;
        }
        let result = if command {
            client.post_command(&server.url, json!({}), None).await
        } else {
            client.post(&server.url, json!({}), None).await
        };
        assert_eq!(result.unwrap_err().to_status_code(), Some(503));
        assert_eq!(server.attempts(), 1);
    }
}

#[tokio::test]
async fn retry_and_body_share_total_timeout_and_keep_received_unauthorized_status() {
    let mut slow = Reply::new(200, "{}");
    slow.delay = Duration::from_secs(1);
    let server = Server::start(vec![Reply::new(503, "{}"), slow]).await;
    let start = Instant::now();
    let error = transport(450, 1024)
        .get(&server.url, None)
        .await
        .unwrap_err();
    assert_eq!(error.network_error_kind(), Some(NetworkErrorKind::Timeout));
    assert_eq!(server.attempts(), 2);
    assert!(start.elapsed() < Duration::from_millis(700));

    let mut interrupted = Reply::new(401, "{");
    interrupted.length = Some(500);
    let server = Server::start(vec![interrupted]).await;
    assert_eq!(
        transport(500, 1024)
            .get(&server.url, None)
            .await
            .unwrap_err()
            .to_status_code(),
        Some(401)
    );
    assert_eq!(server.attempts(), 1);
    let mut slow_rejection = Reply::new(403, "{}");
    slow_rejection.delay = Duration::from_secs(1);
    let server = Server::start(vec![slow_rejection]).await;
    assert_eq!(
        transport(100, 1024)
            .get(&server.url, None)
            .await
            .unwrap_err()
            .to_status_code(),
        Some(403)
    );
}

#[tokio::test]
async fn json_queries_reject_empty_invalid_and_oversized_responses() {
    for body in [
        "",
        "private-secret html",
        r#"{"large":"over-budget-value"}"#,
    ] {
        let server = Server::start(vec![Reply::new(200, body)]).await;
        let error = transport(1000, 16)
            .get(&server.url, None)
            .await
            .unwrap_err();
        assert!(matches!(error, WebApiError::Parse { .. }));
        assert!(!error.to_string().contains("private-secret"));
        assert_eq!(server.attempts(), 1);
    }
    let server = Server::start(vec![Reply::new(202, "")]).await;
    assert_eq!(
        transport(1000, 1024)
            .post_command(&server.url, json!({}), None)
            .await
            .unwrap(),
        Value::Null
    );
}

#[tokio::test]
async fn pagination_preserves_body_tokens_and_falls_back_to_response_header() {
    let server = Server::start(vec![
        Reply::new(200, r#"{"values":[]}"#).header("x-continuation-token: next/+"),
        Reply::new(200, r#"{"pagingInfo":{"continuationToken":"body"}}"#)
            .header("x-continuation-token: header"),
        Reply::new(200, r#"{"data":{"values":[]}}"#).header("x-continuation-token: wrapped"),
    ])
    .await;
    let client = transport(1000, 1024);
    let first = client
        .post_paginated(&server.url, json!({}), None)
        .await
        .unwrap();
    assert_eq!(
        crate::library_continuation(&first).as_deref(),
        Some("next/+")
    );
    let second = client.get_paginated(&server.url, None).await.unwrap();
    assert_eq!(
        crate::library_continuation(&second).as_deref(),
        Some("body")
    );
    let third = client.get_paginated(&server.url, None).await.unwrap();
    assert_eq!(
        crate::library_continuation(crate::library_root(&third)).as_deref(),
        Some("wrapped")
    );
}

#[test]
fn retry_after_accepts_http_date_and_past_date() {
    let now = chrono::DateTime::parse_from_rfc3339("2026-11-08T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    assert_eq!(
        parse_retry_after("Sun, 08 Nov 2026 00:00:02 GMT", now),
        Some(Duration::from_secs(2))
    );
    assert_eq!(
        parse_retry_after("Sat, 07 Nov 2026 00:00:00 GMT", now),
        Some(Duration::ZERO)
    );
    assert_eq!(parse_retry_after("invalid", now), None);
}

#[tokio::test]
async fn legacy_streaming_empty_response_contract_is_preserved() {
    let server = Server::start(vec![Reply::new(202, "")]).await;
    let client = HttpTransport::with_client(Client::builder().no_proxy().build().unwrap());
    assert_eq!(
        client.post(&server.url, json!({}), None).await.unwrap(),
        Value::String(String::new())
    );
}
