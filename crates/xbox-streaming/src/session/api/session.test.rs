use super::*;
use crate::policy::config::Config;
use crate::policy::context::Context;
use crate::policy::runtime::RuntimeMode;
use crate::policy::session::{
    compiler::compile_session, ResolutionPreference, ResolvedSessionAccess,
};
use crate::policy::types::Target;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// 本地 HTTP 回放验证网关实际发送的设备头与设置，凭证和目标均使用测试值。
#[tokio::test]
async fn creation_request_preserves_compiled_resolution_and_home_cloud_isolation() {
    for (target, resolution, width, height, os) in [
        (
            Target::Home,
            ResolutionPreference::P720,
            1280,
            720,
            "android",
        ),
        (
            Target::Home,
            ResolutionPreference::P1080,
            1920,
            1080,
            "windows",
        ),
        (
            Target::Home,
            ResolutionPreference::P1080Hq,
            1920,
            1080,
            "tizen",
        ),
        (
            Target::Home,
            ResolutionPreference::P1440,
            2560,
            1440,
            "tizen",
        ),
        (
            Target::Cloud,
            ResolutionPreference::P1080,
            4096,
            2160,
            "tizen",
        ),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let capture = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            loop {
                let mut bytes = [0; 4096];
                let size = socket.read(&mut bytes).await.unwrap();
                assert!(size > 0);
                request.extend_from_slice(&bytes[..size]);
                assert!(request.len() < 32768);
                if let Some(end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&request[..end]).to_ascii_lowercase();
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            line.strip_prefix("content-length:")
                                .and_then(|value| value.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if request.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            let body = r#"{"sessionPath":"/sessions/test"}"#;
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            String::from_utf8(request).unwrap()
        });
        let mut config = Config::default();
        config.session.cloud_resolution = ResolutionPreference::P1440;
        config.session.home_resolution = resolution;
        let context = Context {
            target,
            target_id: "test-target".into(),
            ..Default::default()
        };
        let session = compile_session(
            &config,
            &context,
            &ResolvedSessionAccess {
                gs_token: "test-token".into(),
                base_url,
                region: None,
            },
            RuntimeMode::WebRtcDirect,
        );
        let user_agent = session.headers["User-Agent"].clone();
        let device_header = session.headers["x-ms-device-info"].clone();
        let plan = Plan {
            session,
            ..Default::default()
        };
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        let gateway = WebApiSessionGateway::with_transport(
            HttpTransport::with_client(client),
            plan,
            StreamingToken {
                gs_token: "test-token".into(),
            },
        );
        assert_eq!(gateway.start_stream().await.unwrap(), "/sessions/test");
        let request = tokio::time::timeout(Duration::from_secs(2), capture)
            .await
            .unwrap()
            .unwrap();
        let (headers, body) = request.split_once("\r\n\r\n").unwrap();
        assert!(headers.starts_with(&format!(
            "POST /v5/sessions/{}/play HTTP/1.1",
            target.as_str()
        )));
        let header = |name: &str| {
            headers
                .lines()
                .find_map(|line| {
                    let (key, value) = line.split_once(':')?;
                    key.eq_ignore_ascii_case(name)
                        .then(|| value.trim().to_owned())
                })
                .unwrap()
        };
        assert_eq!(header("user-agent"), user_agent);
        assert_eq!(header("x-ms-device-info"), device_header);
        let device: Value = serde_json::from_str(&device_header).unwrap();
        assert_eq!(
            device["dev"]["displayInfo"]["dimensions"]["widthInPixels"],
            width
        );
        assert_eq!(
            device["dev"]["displayInfo"]["dimensions"]["heightInPixels"],
            height
        );
        let payload: Value = serde_json::from_str(body).unwrap();
        assert_eq!(payload["settings"]["osName"], os);
        assert_eq!(payload["settings"]["sdkType"], "web");
        assert_eq!(
            payload["serverId"],
            if target.is_home() { "test-target" } else { "" }
        );
        assert_eq!(
            payload["titleId"],
            if target.is_home() { "" } else { "test-target" }
        );
    }
}
