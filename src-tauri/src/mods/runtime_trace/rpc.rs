use crate::error::{AppError, AppResult};
use crate::mods::runtime_trace::{
    RuntimeTraceCategory, RuntimeTraceRecorderRef, TraceDimension, TraceImportance, TraceMetadata,
    TraceRecordDisposition,
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

#[cfg(test)]
#[path = "rpc.test.rs"]
mod tests;

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
            let metadata = metadata.unwrap_or_else(|| {
                super::policy::default_trace_metadata(
                    params.category.as_deref().unwrap_or("event"),
                    "streaming-runtime-host",
                    &params.event,
                    &params.payload,
                )
            });
            let disposition = runtime_trace.try_record_value_with_metadata(
                category,
                "streaming-runtime-host",
                &params.event,
                params.session_id.as_deref(),
                params.payload,
                metadata,
            );
            Ok(
                json!({ "accepted": disposition == TraceRecordDisposition::Enqueued, "disposition": disposition }),
            )
        }
        "getPolicy" => Ok(runtime_trace.producer_policy()),
        "getStatus" => Ok(runtime_trace.status()),
        "exportBundle" => tokio::task::spawn_blocking(move || runtime_trace.export_bundle())
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?
            .map_err(|error| AppError::Internal(error.to_string())),
        "flush" => {
            let result = tokio::task::spawn_blocking(move || {
                let status = runtime_trace.flush();
                json!({ "status": status, "backend": runtime_trace.status()["backend"] })
            })
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?;
            Ok(result)
        }
        other => Err(AppError::Internal(format!(
            "Unknown runtimeTrace method: {}",
            other
        ))),
    }
}
