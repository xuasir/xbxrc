use crate::error::{AppError, AppResult};
use crate::mods::runtime_trace::{
    RuntimeTraceCategory, RuntimeTraceRecorderRef, TraceDimension, TraceImportance, TraceMetadata,
};
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordEventParams {
    event: String,
    payload: Value,
    session_id: Option<String>,
    category: Option<String>,
    dimension: Option<String>,
    importance: Option<String>,
}

pub async fn handle_rpc(
    method: &str,
    params: Option<Value>,
    runtime_trace: RuntimeTraceRecorderRef,
) -> AppResult<Value> {
    match method {
        "recordEvent" => {
            let params = params.ok_or_else(|| {
                AppError::InvalidParams("Missing runtimeTrace.recordEvent params".to_string())
            })?;
            let params: RecordEventParams = serde_json::from_value(params).map_err(|e| {
                AppError::InvalidParams(format!("Invalid runtimeTrace recordEvent params: {}", e))
            })?;
            let category = params
                .category
                .as_deref()
                .and_then(RuntimeTraceCategory::parse)
                .unwrap_or(RuntimeTraceCategory::Event);
            let metadata = params
                .dimension
                .as_deref()
                .and_then(TraceDimension::parse)
                .zip(
                    params
                        .importance
                        .as_deref()
                        .and_then(TraceImportance::parse),
                )
                .map(|(dimension, importance)| TraceMetadata::new(dimension, importance));
            if let Some(metadata) = metadata {
                runtime_trace.record_with_metadata(
                    category,
                    "streaming-runtime-host",
                    &params.event,
                    params.session_id.as_deref(),
                    params.payload,
                    metadata,
                );
            } else {
                runtime_trace.record_with_category(
                    category,
                    "streaming-runtime-host",
                    &params.event,
                    params.session_id.as_deref(),
                    params.payload,
                );
            }
            Ok(json!({ "accepted": true }))
        }
        other => Err(AppError::Internal(format!(
            "Unknown runtimeTrace method: {}",
            other
        ))),
    }
}
