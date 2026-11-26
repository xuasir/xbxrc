//! 生产事件的准入、元数据与安全数值投影；沿用 schema v3。
use serde_json::{json, Map, Value};

use super::policy::{RuntimeTraceProfile, TraceDimension, TraceImportance, TraceMetadata};

pub(super) const PRODUCTION_CONTRACT_VERSION: u32 = 1;

pub(super) struct EventContract {
    pub event: &'static str,
    pub dimension: TraceDimension,
    pub importance: TraceImportance,
    pub category: &'static str,
    pub interval_ms: u64,
    pub max_row_bytes: usize,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SharedEventContract {
    event: String,
    dimension: String,
    importance: String,
    category: String,
    interval_ms: u64,
    max_row_bytes: usize,
}

// 准入与前端投影从同一份描述表装配，新增执行域事件自动沿用相同预算。
fn events() -> &'static [EventContract] {
    static EVENTS: std::sync::OnceLock<Vec<EventContract>> = std::sync::OnceLock::new();
    EVENTS.get_or_init(|| {
        payload_contract()
            .events
            .iter()
            .map(|event| EventContract {
                event: event.event.as_str(),
                dimension: TraceDimension::ALL
                    .into_iter()
                    .find(|dimension| dimension.as_str() == event.dimension)
                    .expect("validated trace dimension"),
                importance: TraceImportance::parse(&event.importance)
                    .expect("validated trace importance"),
                category: event.category.as_str(),
                interval_ms: event.interval_ms,
                max_row_bytes: event.max_row_bytes,
            })
            .collect()
    })
}

pub(super) fn contract(event: &str) -> Option<&'static EventContract> {
    events().iter().find(|contract| contract.event == event)
}

pub(super) fn metadata(event: &str, payload: &Value) -> Option<TraceMetadata> {
    contract(event).map(|contract| {
        let terminal = event == "playbackSessionSummary" && payload["ended"] == true;
        let failure = matches!(
            event,
            "inputUiInteractionObserved" | "browserPlaybackAnomalyObserved"
        ) && matches!(
            payload["outcome"].as_str(),
            Some("failed" | "blocked" | "hardFault")
        );
        TraceMetadata::new(
            contract.dimension,
            if terminal || failure {
                TraceImportance::Essential
            } else {
                contract.importance
            },
        )
    })
}

pub(super) fn producer_policy(profile: RuntimeTraceProfile) -> Value {
    json!({
        "profile": profile.as_str(), "productionContractVersion": PRODUCTION_CONTRACT_VERSION,
        "budget": { "maxInFlight": 4, "maxPendingRows": 32, "maxPendingBytes": 262144,
            "normalBytesPerMinute": 327680, "inputBytesPerMinute": 65536, "anomalyBytesPerMinute": 131072 },
        "events": events().iter().map(|event| json!({
            "event": event.event, "dimension": event.dimension.as_str(), "importance": event.importance.as_str(),
            "category": event.category, "intervalMs": event.interval_ms, "maxRowBytes": event.max_row_bytes,
        })).collect::<Vec<_>>()
    })
}

#[derive(serde::Deserialize)]
struct PayloadContract {
    events: Vec<SharedEventContract>,
    common: Vec<String>,
    roots: std::collections::HashMap<String, Vec<String>>,
    children: std::collections::HashMap<String, Vec<String>>,
    enums: Vec<String>,
}

fn payload_contract() -> &'static PayloadContract {
    static CONTRACT: std::sync::OnceLock<PayloadContract> = std::sync::OnceLock::new();
    CONTRACT.get_or_init(|| {
        serde_json::from_str(include_str!(
            "../../../../src/shared/diagnostics/production-trace-contract.json"
        ))
        .expect("validated production trace contract")
    })
}

fn child_fields(parent: &str) -> &[String] {
    payload_contract()
        .children
        .get(parent)
        .map_or(&[], Vec::as_slice)
}

fn safe_string(key: &str, value: &str) -> Option<Value> {
    if payload_contract()
        .enums
        .iter()
        .any(|allowed| allowed == value)
    {
        return Some(Value::String(value.to_owned()));
    }
    let numeric_id = matches!(
        key,
        "sampleId"
            | "firstSampleId"
            | "lastSampleId"
            | "episodeId"
            | "operationId"
            | "target"
            | "payloadType"
            | "selectedPayloadType"
            | "packetizationMode"
    ) && value.len() <= 64
        && !value.is_empty()
        && value
            .chars()
            .all(|ch| ch.is_ascii_digit() || matches!(ch, ':' | '-'));
    let profile = matches!(key, "profileLevelId" | "selectedProfileLevelId")
        && value.len() == 6
        && value.chars().all(|ch| ch.is_ascii_hexdigit());
    if profile {
        Some(Value::String(value.to_ascii_lowercase()))
    } else if key == "decoderImplementation" {
        let name = [
            "D3D11VideoDecoder",
            "D3D12VideoDecoder",
            "FFmpegVideoDecoder",
            "VideoToolbox",
            "MediaFoundation",
            "ExternalDecoder",
            "dav1d",
            "libvpx",
            "OpenH264",
        ]
        .into_iter()
        .find(|name| value.contains(name))
        .unwrap_or("other");
        Some(Value::String(name.into()))
    } else if numeric_id {
        Some(Value::String(value.to_owned()))
    } else {
        None
    }
}

fn project_value(key: &str, value: &Value, fields: &[String], depth: usize) -> Option<Value> {
    if depth > 6 {
        return None;
    }
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) => Some(value.clone()),
        Value::String(value) => safe_string(key, value),
        Value::Object(object) => {
            let mut projected = Map::new();
            for (key, value) in object {
                if fields.iter().any(|allowed| allowed == key) {
                    if let Some(value) = project_value(key, value, child_fields(key), depth + 1) {
                        projected.insert(key.clone(), value);
                    }
                }
            }
            Some(Value::Object(projected))
        }
        Value::Array(array) => Some(Value::Array(
            array
                .iter()
                .take(8)
                .filter_map(|value| project_value(key, value, child_fields(key), depth + 1))
                .collect(),
        )),
    }
}

pub(super) fn project_payload(event: &str, payload: &Value) -> Value {
    let contract = payload_contract();
    let mut fields = contract.common.clone();
    fields.extend(
        contract
            .roots
            .get(event)
            .or_else(|| contract.roots.get("wire"))
            .into_iter()
            .flatten()
            .cloned(),
    );
    let mut projected = project_value("", payload, &fields, 0).unwrap_or_else(|| json!({}));
    if let Some(object) = projected.as_object_mut() {
        object.insert(
            "productionContractVersion".into(),
            json!(PRODUCTION_CONTRACT_VERSION),
        );
    }
    projected
}

#[cfg(test)]
#[path = "trace_contract.test.rs"]
mod tests;
