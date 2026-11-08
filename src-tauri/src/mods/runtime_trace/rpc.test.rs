use super::*;
use crate::mods::runtime_trace::RuntimeTraceRecorder;
use std::sync::Arc;

#[tokio::test]
async fn rpc_returns_real_admission_flush_and_export_results() {
    let recorder = Arc::new(RuntimeTraceRecorder::new_with_mode("production").unwrap());
    let result = handle_rpc("recordEvent", Some(json!({"event": "browserPlaybackHealthObserved", "sessionId": "session", "payload": {"healthSequence": 1, "frameSupply": {"receivedFps": 60}, "secret": "private"}})), recorder.clone()).await.unwrap();
    assert_eq!(result, json!({"accepted": true, "disposition": "enqueued"}));
    let policy = handle_rpc("getPolicy", None, recorder.clone())
        .await
        .unwrap();
    assert_eq!(policy["productionContractVersion"], 1);
    let flush = handle_rpc("flush", None, recorder.clone()).await.unwrap();
    assert_eq!(flush["status"], "flushed");
    assert!(flush["backend"]["writtenRows"].as_u64().unwrap() >= 2);
    let export = handle_rpc("exportBundle", None, recorder.clone())
        .await
        .unwrap();
    assert_eq!(export["manifest"]["flushStatus"], "flushed");
    assert!(export["manifest"]["rowCount"].as_u64().unwrap() >= 2);
    assert!(export["dataBase64"].as_str().is_some());
    recorder.apply_trace_mode("off").unwrap();
    let filtered = handle_rpc(
        "recordEvent",
        Some(json!({"event": "browserPlaybackHealthObserved", "payload": {}})),
        recorder.clone(),
    )
    .await
    .unwrap();
    assert_eq!(
        filtered,
        json!({"accepted": false, "disposition": "filtered"})
    );
    assert_eq!(
        handle_rpc("flush", None, recorder).await.unwrap()["status"],
        "off"
    );
}

#[tokio::test]
async fn rpc_reports_oversized_rows_and_rejects_invalid_arguments() {
    let recorder = Arc::new(RuntimeTraceRecorder::new_with_mode("production").unwrap());
    let oversized = handle_rpc("recordEvent", Some(json!({"event": "rendererFailed", "payload": {"values": vec![123456789u64; 10000]}, "dimension": "core", "importance": "essential"})), recorder.clone()).await.unwrap();
    assert_eq!(
        oversized,
        json!({"accepted": false, "disposition": "rowOversized"})
    );
    assert!(handle_rpc("recordEvent", None, recorder.clone())
        .await
        .is_err());
    assert!(handle_rpc("unknown", None, recorder).await.is_err());
}
