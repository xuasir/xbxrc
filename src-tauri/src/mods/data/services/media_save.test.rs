use super::*;
use std::time::Duration;
use tokio::io::AsyncReadExt;

async fn serve(
    status: u16,
    content_type: Option<&str>,
    body: Vec<u8>,
    length: Option<u64>,
    stall: bool,
) -> (reqwest::Url, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = reqwest::Url::parse(&format!(
        "http://{}/signed?secret=private",
        listener.local_addr().unwrap()
    ))
    .unwrap();
    let content_type = content_type
        .map(|value| format!("Content-Type: {value}\r\n"))
        .unwrap_or_default();
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buffer = [0; 2048];
        let _ = socket.read(&mut buffer).await;
        let header = format!("HTTP/1.1 {status} Status\r\n{content_type}Content-Length: {}\r\nConnection: close\r\n\r\n", length.unwrap_or(body.len() as u64));
        socket.write_all(header.as_bytes()).await.unwrap();
        socket.write_all(&body).await.unwrap();
        if stall {
            tokio::time::sleep(Duration::from_secs(10)).await;
        }
    });
    (url, task)
}

#[tokio::test]
async fn successful_media_replaces_destination_after_validation() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("shot.png");
    std::fs::write(&path, "existing").unwrap();
    let body = b"\x89PNG\r\n\x1a\nvalid-media-payload".to_vec();
    let (url, task) = serve(200, Some("image/png"), body.clone(), None, false).await;
    download_media(
        &reqwest::Client::builder().no_proxy().build().unwrap(),
        url,
        &path,
        MediaKind::Screenshot,
    )
    .await
    .unwrap();
    task.await.unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), body);
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
}

#[tokio::test]
async fn failed_or_invalid_download_preserves_existing_file_and_cleans_temporary_output() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("shot.jpg");
    std::fs::write(&path, "existing").unwrap();
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    for (status, mime, body, length) in [
        (
            403,
            Some("application/json"),
            b"private-secret".to_vec(),
            None,
        ),
        (
            200,
            Some("text/html"),
            b"<html>private-secret</html>".to_vec(),
            None,
        ),
        (200, None, b"<html>private-secret</html>".to_vec(), None),
        (200, Some("image/jpeg"), Vec::new(), None),
        (200, Some("image/jpeg"), b"\xff\xd8\xff".to_vec(), Some(999)),
        (
            200,
            Some("image/jpeg"),
            Vec::new(),
            Some(MAX_MEDIA_BYTES + 1),
        ),
    ] {
        let (url, task) = serve(status, mime, body, length, false).await;
        let error = download_media(&client, url, &path, MediaKind::Screenshot)
            .await
            .unwrap_err();
        assert!(!error.contains("private-secret"));
        task.abort();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "existing");
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
    }
}

#[tokio::test]
async fn cancelling_download_removes_partial_file() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("shot.jpg");
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let (url, task) = serve(
        200,
        Some("image/jpeg"),
        b"\xff\xd8\xff".to_vec(),
        Some(999),
        true,
    )
    .await;
    let result = tokio::time::timeout(
        Duration::from_millis(100),
        download_media(&client, url, &path, MediaKind::Screenshot),
    )
    .await;
    assert!(result.is_err());
    task.abort();
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn media_signatures_distinguish_clip_from_screenshot_payloads() {
    assert!(matches_media_signature(
        MediaKind::Clip,
        b"\0\0\0\x18ftypisom"
    ));
    assert!(matches_media_signature(
        MediaKind::Screenshot,
        b"\0\0\0\x18ftypavif"
    ));
    assert!(!matches_media_signature(
        MediaKind::Clip,
        b"\0\0\0\x18ftypavif"
    ));
    assert!(!matches_media_signature(
        MediaKind::Screenshot,
        b"\0\0\0\x18ftypisom"
    ));
    assert!(!matches_media_signature(
        MediaKind::Screenshot,
        b"<html>error</html>"
    ));
}

#[tokio::test]
async fn download_failures_distinguish_rate_limit_service_error_and_timeout() {
    let root = tempfile::tempdir().unwrap();
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_millis(60))
        .build()
        .unwrap();
    for (status, hint) in [(429, "频繁"), (503, "暂不可用")] {
        let (url, task) = serve(
            status,
            Some("application/json"),
            b"private-secret".to_vec(),
            None,
            false,
        )
        .await;
        let error = download_media(
            &client,
            url,
            &root.path().join("shot.jpg"),
            MediaKind::Screenshot,
        )
        .await
        .unwrap_err();
        assert!(error.contains(hint));
        assert!(!error.contains("private"));
        task.await.unwrap();
    }
    let (url, task) = serve(
        200,
        Some("image/jpeg"),
        b"\xff\xd8\xff".to_vec(),
        Some(999),
        true,
    )
    .await;
    let error = download_media(
        &client,
        url,
        &root.path().join("shot.jpg"),
        MediaKind::Screenshot,
    )
    .await
    .unwrap_err();
    assert!(error.contains("超时"));
    task.abort();
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}
