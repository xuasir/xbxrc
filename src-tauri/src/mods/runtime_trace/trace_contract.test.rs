use super::*;
use crate::mods::runtime_trace::{
    RuntimeTraceCategory, RuntimeTraceRecorder, TraceRecordDisposition,
};

#[test]
fn shared_fixture_and_descriptors_match_the_frontend_contract() {
    let fixtures: Value = serde_json::from_str(include_str!(
        "../../../../src/shared/diagnostics/production-trace.fixture.json"
    ))
    .unwrap();
    for fixture in fixtures.as_array().unwrap() {
        assert_eq!(
            project_payload(fixture["event"].as_str().unwrap(), &fixture["input"]),
            fixture["expected"]
        );
    }
    let shared: Value = serde_json::from_str(include_str!(
        "../../../../src/shared/diagnostics/production-trace-contract.json"
    ))
    .unwrap();
    let policy = producer_policy(RuntimeTraceProfile::Production);
    for descriptor in shared["events"].as_array().unwrap() {
        assert!(
            policy["events"].as_array().unwrap().contains(descriptor),
            "missing descriptor: {descriptor}"
        );
    }
    assert_eq!(
        shared["events"].as_array().unwrap().len(),
        policy["events"].as_array().unwrap().len()
    );
}

#[test]
fn production_projection_retains_numeric_evidence_and_filters_private_fields() {
    let value = project_payload(
        "browserPlaybackHealthObserved",
        &json!({
            "healthSequence": 1, "sampleId": "2:4", "url": "https://private.example/?token=secret",
            "frameSupply": { "receivedFps": 60, "decodeFps": 45, "account": "private", "windows": {
                "short": { "sampleCount": 12, "durationMs": 3000, "decodeFpsAvg": 45, "token": "secret" }
            } },
            "renderer": { "timing": { "upload": { "count": 60, "p95Ms": 23.5, "message": "secret" } } },
            "recovery": { "decoderImplementation": "D3D11VideoDecoder", "error": "private", "framesDecoded": 100 },
        }),
    );
    assert_eq!(value["frameSupply"]["decodeFps"], 45);
    assert_eq!(value["renderer"]["timing"]["upload"]["p95Ms"], 23.5);
    assert_eq!(
        value["recovery"]["decoderImplementation"],
        "D3D11VideoDecoder"
    );
    assert!(!value.to_string().contains("private"));
    assert!(!value.to_string().contains("secret"));
}

#[test]
fn recorder_returns_actual_profile_filter_and_enqueue_results() {
    let recorder = RuntimeTraceRecorder::new_with_mode("production").unwrap();
    let metadata = TraceMetadata::new(TraceDimension::Frontend, TraceImportance::Debug);
    assert_eq!(
        recorder.try_record_value_with_metadata(
            RuntimeTraceCategory::Snapshot,
            "test",
            "detail",
            None,
            json!({}),
            metadata
        ),
        TraceRecordDisposition::Filtered
    );
    assert_eq!(
        recorder.try_record_value_with_metadata(
            RuntimeTraceCategory::Snapshot,
            "test",
            "browserPlaybackHealthObserved",
            Some("session"),
            json!({ "healthSequence": 1 }),
            metadata
        ),
        TraceRecordDisposition::Enqueued
    );
    recorder.apply_trace_mode("off").unwrap();
    assert_eq!(
        recorder.try_record_value_with_metadata(
            RuntimeTraceCategory::Snapshot,
            "test",
            "browserPlaybackHealthObserved",
            Some("session"),
            json!({}),
            metadata
        ),
        TraceRecordDisposition::Filtered
    );
}

#[test]
fn terminal_and_failed_results_keep_essential_priority() {
    assert_eq!(
        metadata("playbackSessionSummary", &json!({"ended": true}))
            .unwrap()
            .importance,
        TraceImportance::Essential
    );
    assert_eq!(
        metadata("inputUiInteractionObserved", &json!({"outcome": "blocked"}))
            .unwrap()
            .importance,
        TraceImportance::Essential
    );
    let policy = producer_policy(RuntimeTraceProfile::Production);
    assert_eq!(policy["budget"]["maxPendingBytes"], 262144);
    assert!(policy["events"]
        .as_array()
        .unwrap()
        .iter()
        .any(
            |event| event["event"] == "inputUiInteractionObserved" && event["dimension"] == "input"
        ));
}
