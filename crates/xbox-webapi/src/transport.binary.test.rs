use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

async fn server(
    status: u16,
    body: Vec<u8>,
    declared_length: usize,
) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/bundle", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut bytes = [0; 4096];
        let _ = stream.read(&mut bytes).await;
        stream.write_all(format!("HTTP/1.1 {status} Status\r\nContent-Length: {declared_length}\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
        let _ = stream.write_all(&body).await;
    });
    (url, task)
}

fn client() -> HttpTransport {
    HttpTransport::with_client(Client::builder().no_proxy().build().unwrap())
}

#[tokio::test]
async fn keeps_binary_bytes_without_json_or_utf8_conversion() {
    let bytes = vec![b'P', b'K', 0, 255, 1];
    let (url, task) = server(200, bytes.clone(), bytes.len()).await;
    assert_eq!(client().get_bytes(&url, None, 16).await.unwrap(), bytes);
    task.await.unwrap();
}

#[tokio::test]
async fn rejects_oversized_and_truncated_binary_responses() {
    for (body, length, limit) in [(vec![0; 20], 20, 8), (vec![0; 4], 10, 16)] {
        let (url, task) = server(200, body, length).await;
        let mut client = client();
        client.retry_mode = RetryMode::Never;
        assert!(client.get_bytes(&url, None, limit).await.is_err());
        task.await.unwrap();
    }
}

#[tokio::test]
async fn preserves_http_status_and_sanitizes_error_body() {
    let bytes = b"private bearer response".to_vec();
    let (url, task) = server(403, bytes.clone(), bytes.len()).await;
    let error = client().get_bytes(&url, None, 16).await.unwrap_err();
    assert_eq!(error.to_status_code(), Some(403));
    assert!(!error.to_string().contains("private"));
    task.await.unwrap();
}

#[tokio::test]
async fn rejects_invalid_binary_budgets_before_network_access() {
    for limit in [0, QUERY_MAX_BODY_BYTES + 1] {
        assert!(client()
            .get_bytes("https://example.com", None, limit)
            .await
            .is_err());
    }
}
