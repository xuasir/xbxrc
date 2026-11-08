use super::{
    record_data_http_result, DataXcloudCatalogCacheState, DataXcloudCatalogPayload,
    DataXcloudCatalogUpdatedEvent,
};
use crate::mods::runtime_trace::{RuntimeTraceRecorder, RuntimeTraceRecorderRef};
use serde_json::Value;
use std::sync::Arc;
use std::time::Instant;
use xbox_webapi::WebApiError;

#[test]
fn production_and_dev_retain_safe_http_failure_diagnostics() {
    for profile in ["production", "dev"] {
        let recorder: RuntimeTraceRecorderRef =
            Arc::new(RuntimeTraceRecorder::new_with_mode(profile).unwrap());
        record_data_http_result(
            &recorder,
            "achievements",
            Instant::now(),
            Some(&WebApiError::http_with_code(
                403,
                "UnsupportedCountry",
                "secret-token-and-full-url",
            )),
        );
        record_data_http_result(&recorder, "profile", Instant::now(), None);
        let contents = std::fs::read_to_string(recorder.path().unwrap()).unwrap();
        let rows: Vec<Value> = contents
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .filter(|row| row["domain"] == "data")
            .collect();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["importance"], "key");
        assert_eq!(rows[0]["payload"]["error"]["statusCode"], 403);
        assert_eq!(
            rows[0]["payload"]["error"]["serviceCode"],
            "UnsupportedCountry"
        );
        assert_eq!(rows[1]["payload"]["outcome"], "success");
        assert!(!contents.contains("secret"));
    }
}

#[test]
fn cloud_catalog_rpc_and_event_match_optional_timestamp_contract() {
    for updated_at in [None, Some(0), Some(1_790_000_000_000)] {
        let payload = serde_json::to_value(DataXcloudCatalogPayload {
            scope_key: None,
            titles: vec![],
            cache_state: DataXcloudCatalogCacheState::Miss,
            updated_at,
            refreshing: false,
        })
        .unwrap();
        let event = serde_json::to_value(DataXcloudCatalogUpdatedEvent {
            scope_key: "account-region-language-market".to_string(),
            titles: vec![],
            cache_state: DataXcloudCatalogCacheState::Miss,
            updated_at,
            refreshing: false,
            reason: "refreshed".to_string(),
        })
        .unwrap();

        assert!(payload.get("scopeKey").is_none());
        for value in [payload, event] {
            assert_eq!(value.get("updatedAt"), updated_at.map(Value::from).as_ref());
        }
    }
}
