// 追加到临时 xbox-streaming 源码副本的 session/flow.test.rs 后运行。
// 这些用例断言审查时的缺陷行为，修复后的正式回归应断言安全结果。
#[tokio::test]
async fn audit_common_startup_restart_submit_readmits_previous_ice_snapshot() {
    let server = TestServer::new(|request| {
        let body = if request.starts_with("GET") {
            let candidates = serde_json::json!([
                {"candidate":"candidate:1 1 UDP 1 192.0.2.1 5000 typ host", "sdpMid":"0", "sdpMLineIndex":0, "usernameFragment":"old-generation"},
                {"candidate":"end-of-candidates", "sdpMid":"0", "sdpMLineIndex":0}
            ]);
            serde_json::json!({"exchangeResponse": candidates.to_string()})
        } else {
            serde_json::json!({})
        };
        (200, body, 0)
    }).await;
    let flow = SessionFlowService::<Snapshot, _>::new(HttpTestProvider::default());
    install_http_record(&flow, &server.base_url).await;
    let initial = flow.poll_ice("session", false).await.unwrap();
    assert_eq!(initial.len(), 2);
    assert!(flow.poll_ice("session", true).await.unwrap().is_empty());
    let local = IceCandidate {
        candidate: "candidate:2 1 UDP 1 192.0.2.2 5001 typ host".to_owned(),
        sdp_mid: Some("0".to_owned()),
        sdp_m_line_index: Some(0),
        username_fragment: Some("new-generation".to_owned()),
        message_type: None,
    };
    flow.submit_ice("session", &[local], true).await.unwrap();
    let replayed = flow.poll_ice("session", true).await.unwrap();
    assert_eq!(replayed.len(), 2);
    assert_eq!(replayed[0].username_fragment.as_deref(), Some("old-generation"));
    assert_eq!(replayed[1].candidate, "a=end-of-candidates");
    println!("AUDIT restart-submit: previous snapshot returned again; old ufrag + EOC; candidates={}", replayed.len());
    flow.close_session("session").await.unwrap();
}

#[tokio::test]
async fn audit_common_startup_rebuild_consumes_previous_sdp_answer() {
    let server = TestServer::new(|request| {
        let body = if request.starts_with("GET") {
            serde_json::json!({"exchangeResponse": serde_json::json!({
                "sdp":"v=0\r\na=ice-ufrag:old-generation\r\n", "messageType":"answer"
            }).to_string()})
        } else {
            serde_json::json!({})
        };
        (200, body, 0)
    }).await;
    let flow = SessionFlowService::<Snapshot, _>::new(HttpTestProvider::default());
    install_http_record(&flow, &server.base_url).await;
    let first = flow.exchange_offer("session", None, "old-offer", false).await.unwrap();
    let rebuilt = flow.exchange_offer("session", None, "new-peer-offer", false).await.unwrap();
    assert_eq!(rebuilt.sdp, first.sdp);
    assert_eq!(server.count("GET /v5/sessions/home/session/sdp"), 2);
    println!("AUDIT peer-rebuild: second offer accepted old answer on first GET; SDP={:?}", rebuilt.sdp);
    flow.close_session("session").await.unwrap();
}
