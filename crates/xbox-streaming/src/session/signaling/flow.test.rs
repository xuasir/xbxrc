use super::*;

fn ice_response(ufrag: &str) -> Value {
    serde_json::json!({"exchangeResponse": serde_json::json!([
        {"candidate":"candidate:1 1 UDP 1 192.0.2.1 5000 typ host", "sdpMid":"0", "sdpMLineIndex":0, "usernameFragment":ufrag},
        {"candidate":"end-of-candidates", "sdpMid":"0", "sdpMLineIndex":0}
    ]).to_string()})
}
fn sdp_response(ufrag: &str) -> Value {
    serde_json::json!({"exchangeResponse": serde_json::json!({
        "sdp":format!("v=0\r\na=ice-ufrag:{ufrag}\r\n"), "messageType":"answer"
    }).to_string()})
}
async fn install_signaling_record(
    flow: &SessionFlowService<Snapshot, HttpTestProvider>,
    url: &str,
) {
    install_http_record(flow, url).await;
    let mut sessions = flow.inner.sessions.write().await;
    let mut record = sessions.get("session").unwrap();
    record.plan.session.schedule.offer_exchange_timeout_ms = 5000;
    record.plan.session.schedule.ice_exchange_timeout_ms = 5000;
    record.plan.session.schedule.offer_poll_interval_ms = 100;
    sessions.upsert_if_same("session".to_owned(), record);
}

#[tokio::test]
async fn restart_posts_preserve_snapshot_dedup_across_multiple_local_batches() {
    let server = TestServer::new(|request| {
        (
            200,
            if request.starts_with("GET") {
                ice_response("old")
            } else {
                serde_json::json!({})
            },
            0,
        )
    })
    .await;
    let flow = SessionFlowService::<Snapshot, _>::new(HttpTestProvider::default());
    install_signaling_record(&flow, &server.base_url).await;
    assert_eq!(flow.poll_ice("session", false).await.unwrap().len(), 2);
    for port in [5001, 5002] {
        let local = IceCandidate {
            candidate: format!("candidate:2 1 UDP 1 192.0.2.2 {port} typ host"),
            ..Default::default()
        };
        flow.submit_ice("session", &[local], true).await.unwrap();
        assert!(flow.poll_ice("session", true).await.unwrap().is_empty());
    }
    assert_eq!(server.count("POST /v5/sessions/home/session/ice"), 2);
    flow.close_session("session").await.unwrap();
}

#[tokio::test]
async fn new_peer_and_restart_wait_for_current_sdp_and_filter_previous_ice() {
    for restart in [false, true] {
        let posts = Arc::new(AtomicUsize::new(0));
        let stale_reads = Arc::new(AtomicUsize::new(0));
        let fresh_ice = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (server_posts, server_reads, server_ice) =
            (posts.clone(), stale_reads.clone(), fresh_ice.clone());
        let server = TestServer::new(move |request| {
            let body = if request.starts_with("POST /v5/sessions/home/session/sdp") {
                server_posts.fetch_add(1, Ordering::SeqCst);
                serde_json::json!({})
            } else if request.starts_with("GET /v5/sessions/home/session/sdp") {
                let fresh = server_posts.load(Ordering::SeqCst) >= 2
                    && server_reads.fetch_add(1, Ordering::SeqCst) >= 1;
                sdp_response(if fresh { "remote-new" } else { "remote-old" })
            } else if request.starts_with("GET /v5/sessions/home/session/ice") {
                ice_response(if server_ice.load(Ordering::SeqCst) {
                    "remote-new"
                } else {
                    "remote-old"
                })
            } else {
                serde_json::json!({})
            };
            (200, body, 0)
        })
        .await;
        let flow = SessionFlowService::<Snapshot, _>::new(HttpTestProvider::default());
        install_signaling_record(&flow, &server.base_url).await;
        let first = flow
            .exchange_offer("session", None, "v=0\r\na=ice-ufrag:local-old\r\n", false)
            .await
            .unwrap();
        assert!(first.sdp.contains("remote-old"));
        assert_eq!(flow.poll_ice("session", false).await.unwrap().len(), 2);
        let second = flow
            .exchange_offer("session", None, "v=0\r\na=ice-ufrag:local-new\r\n", restart)
            .await
            .unwrap();
        assert!(second.sdp.contains("remote-new"));
        assert_eq!(posts.load(Ordering::SeqCst), 2);
        assert_eq!(stale_reads.load(Ordering::SeqCst), 2);
        for port in [5001, 5002] {
            let local = IceCandidate {
                candidate: format!("candidate:2 1 UDP 1 192.0.2.2 {port} typ host"),
                ..Default::default()
            };
            flow.submit_ice("session", &[local], restart).await.unwrap();
            assert!(flow.poll_ice("session", restart).await.unwrap().is_empty());
        }
        fresh_ice.store(true, Ordering::SeqCst);
        let candidates = flow.poll_ice("session", restart).await.unwrap();
        assert_eq!(candidates.len(), 2);
        assert_eq!(
            candidates[0].username_fragment.as_deref(),
            Some("remote-new")
        );
        assert_eq!(candidates[1].candidate, "a=end-of-candidates");
        flow.close_session("session").await.unwrap();
    }
}

#[tokio::test]
async fn chat_and_quality_offers_preserve_the_media_ice_generation() {
    let chat = Arc::new(AtomicUsize::new(0));
    let server_chat = chat.clone();
    let server = TestServer::new(move |request| {
        let body = if request.starts_with("GET") && request.contains("/sdp") {
            sdp_response(if server_chat.load(Ordering::SeqCst) >= 3 {
                "chat"
            } else {
                "media"
            })
        } else if request.starts_with("POST") && request.contains("/sdp") {
            server_chat.fetch_add(1, Ordering::SeqCst);
            serde_json::json!({})
        } else if request.starts_with("GET") && request.contains("/ice") {
            ice_response("media")
        } else {
            serde_json::json!({})
        };
        (200, body, 0)
    })
    .await;
    let flow = SessionFlowService::<Snapshot, _>::new(HttpTestProvider::default());
    install_signaling_record(&flow, &server.base_url).await;
    let offer = "v=0\r\na=ice-ufrag:local\r\na=ice-pwd:password\r\n";
    flow.exchange_offer("session", None, offer, false)
        .await
        .unwrap();
    assert_eq!(flow.poll_ice("session", false).await.unwrap().len(), 2);
    let before = flow
        .inner
        .signaling
        .read()
        .await
        .get("session")
        .unwrap()
        .clone();
    flow.exchange_offer("session", None, &format!("{offer}b=AS:10000\r\n"), false)
        .await
        .unwrap();
    flow.exchange_offer("session", Some("chat"), offer, false)
        .await
        .unwrap();
    let after = flow
        .inner
        .signaling
        .read()
        .await
        .get("session")
        .unwrap()
        .clone();
    assert_eq!(before.generation, after.generation);
    assert_eq!(before.last_polled_ice, after.last_polled_ice);
    assert!(flow.poll_ice("session", false).await.unwrap().is_empty());
    flow.close_session("session").await.unwrap();
}

#[tokio::test]
async fn queued_ice_requests_cannot_consume_or_submit_into_a_new_negotiation() {
    for submit in [false, true] {
        let server = TestServer::new(|_| (200, ice_response("old"), 0)).await;
        let flow = SessionFlowService::<Snapshot, _>::new(HttpTestProvider::default());
        install_signaling_record(&flow, &server.base_url).await;
        let mutation = flow
            .inner
            .mutations
            .read()
            .await
            .get("session")
            .unwrap()
            .clone();
        let lock = mutation.ice.lock().await;
        let pending = if submit {
            let flow = flow.clone();
            tokio::spawn(async move { flow.submit_ice("session", &[], false).await })
        } else {
            let flow = flow.clone();
            tokio::spawn(async move { flow.poll_ice("session", false).await.map(|_| ()) })
        };
        // 单线程 Tokio 上让请求进入 ICE 锁等待，随后模拟锁内的 offer 代际推进。
        tokio::task::yield_now().await;
        flow.inner
            .signaling
            .write()
            .await
            .entry("session".to_owned())
            .or_default()
            .begin_media_ice(vec!["new".to_owned()], None);
        drop(lock);
        assert_eq!(
            pending.await.unwrap().unwrap_err().message,
            "signalingSuperseded"
        );
        assert_eq!(server.count("GET"), 0);
        assert_eq!(server.count("POST"), 0);
        flow.close_session("session").await.unwrap();
    }
}
