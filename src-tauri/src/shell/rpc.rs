use crate::mods;
use crate::shell::state::AppState;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Instant;
use tauri::Manager;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcInvokePayload {
    pub namespace: String,
    pub method: String,
    pub params: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct RpcError {
    pub code: String,
    pub message: String,
    pub details: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct RpcEnvelope {
    pub ok: bool,
    pub data: Option<serde_json::Value>,
    pub error: Option<RpcError>,
}

impl RpcEnvelope {
    pub fn success(data: serde_json::Value) -> Self {
        Self {
            ok: true,
            data: Some(data),
            error: None,
        }
    }

    pub fn fail(error: crate::error::AppError) -> Self {
        Self {
            ok: false,
            data: None,
            error: Some(RpcError {
                code: error.code().to_string(),
                message: error.to_string(),
                details: error.details().cloned(),
            }),
        }
    }
}

#[tauri::command]
pub async fn rpc_invoke(payload: RpcInvokePayload, app_handle: tauri::AppHandle) -> RpcEnvelope {
    let start = Instant::now();
    let namespace = payload.namespace.clone();
    let method = payload.method.clone();
    let xbxengine_trace = if namespace == "xbxEngine" {
        build_xbxengine_trace_summary(method.as_str(), payload.params.as_ref())
    } else {
        None
    };

    if let Some((event, session_id, trace_payload)) = xbxengine_trace.as_ref().map(|summary| {
        (
            summary.received_event,
            summary.session_id.clone(),
            summary.payload.clone(),
        )
    }) {
        app_handle.state::<AppState>().runtime_trace.record_event(
            "xbxengine-host",
            event,
            session_id.as_deref(),
            trace_payload,
        );
    }

    // log::info!("[RPC][IN] {}.{}", namespace, method);

    let result = match payload.namespace.as_str() {
        "app" => mods::app_state::rpc::handle_rpc(
            payload.method.as_str(),
            payload.params,
            app_handle.clone(),
        )
        .await
        .map_err(Into::into),
        "config" => mods::config::rpc::handle_rpc(
            payload.method.as_str(),
            payload.params,
            app_handle.clone(),
        )
        .await
        .map_err(Into::into),
        "auth" => {
            mods::auth::rpc::handle_rpc(payload.method.as_str(), payload.params, app_handle.clone())
                .await
                .map_err(Into::into)
        }
        "gamepad" => mods::gamepad::rpc::handle_rpc(
            payload.method.as_str(),
            payload.params,
            app_handle.clone(),
        )
        .await
        .map_err(Into::into),
        "data" => {
            mods::data::rpc::handle_rpc(payload.method.as_str(), payload.params, app_handle.clone())
                .await
                .map_err(Into::into)
        }
        "streaming" => mods::streaming::rpc::handle_rpc(
            payload.method.as_str(),
            payload.params,
            app_handle.clone(),
        )
        .await
        .map_err(Into::into),
        "runtimeTrace" => {
            let runtime_trace = app_handle
                .state::<crate::shell::state::AppState>()
                .runtime_trace
                .clone();
            mods::runtime_trace::rpc::handle_rpc(
                payload.method.as_str(),
                payload.params,
                runtime_trace,
            )
            .await
            .map_err(Into::into)
        }
        "xbxEngine" => mods::xbxengine::rpc::handle_rpc(
            payload.method.as_str(),
            payload.params,
            app_handle.clone(),
        )
        .await
        .map_err(Into::into),
        "system" => mods::app_state::rpc::handle_system_rpc(
            payload.method.as_str(),
            payload.params,
            app_handle.clone(),
        )
        .await
        .map_err(Into::into),
        _ => Err(crate::error::AppError::Internal(format!(
            "Unknown namespace: {}",
            payload.namespace
        ))),
    };

    let duration_ms = start.elapsed().as_millis();
    match result {
        Ok(data) => {
            if let Some(summary) = xbxengine_trace {
                app_handle.state::<AppState>().runtime_trace.record_event(
                    "xbxengine-host",
                    summary.completed_event,
                    summary.session_id.as_deref(),
                    json!({
                        "method": summary.method,
                        "sessionId": summary.session_id,
                        "viewportId": summary.viewport_id,
                        "targetType": summary.target_type,
                        "durationMs": duration_ms,
                        "ok": true,
                    }),
                );
            }
            // log::info!(
            //     "[RPC][OUT] {}.{} ok duration={}ms",
            //     namespace,
            //     method,
            //     duration_ms
            // );
            RpcEnvelope::success(data)
        }
        Err(error) => {
            if let Some(summary) = xbxengine_trace {
                app_handle.state::<AppState>().runtime_trace.record_event(
                    "xbxengine-host",
                    summary.completed_event,
                    summary.session_id.as_deref(),
                    json!({
                        "method": summary.method,
                        "sessionId": summary.session_id,
                        "viewportId": summary.viewport_id,
                        "targetType": summary.target_type,
                        "durationMs": duration_ms,
                        "ok": false,
                        "errorCode": error.code(),
                        "errorMessage": error.to_string(),
                    }),
                );
            }
            log::error!(
                "[RPC][OUT] {}.{} err code={} message={} duration={}ms",
                namespace,
                method,
                error.code(),
                error.to_string(),
                duration_ms
            );
            RpcEnvelope::fail(error)
        }
    }
}

struct XbxEngineRpcTraceSummary {
    method: &'static str,
    received_event: &'static str,
    completed_event: &'static str,
    session_id: Option<String>,
    viewport_id: Option<String>,
    target_type: Option<String>,
    payload: Value,
}

fn build_xbxengine_trace_summary(
    method: &str,
    params: Option<&Value>,
) -> Option<XbxEngineRpcTraceSummary> {
    match method {
        "AttachViewport" => {
            let viewport_id = params
                .and_then(|value| value.get("viewportId"))
                .and_then(Value::as_str)
                .map(str::to_string);
            Some(XbxEngineRpcTraceSummary {
                method: "AttachViewport",
                received_event: "runtimeAttachViewportRpcReceived",
                completed_event: "runtimeAttachViewportRpcCompleted",
                session_id: None,
                viewport_id: viewport_id.clone(),
                target_type: None,
                payload: json!({
                    "method": "AttachViewport",
                    "viewportId": viewport_id,
                }),
            })
        }
        "StartRuntime" => {
            let session = params.and_then(|value| value.get("session"));
            let viewport = params.and_then(|value| value.get("viewport"));
            let session_id = session
                .and_then(|value| value.get("sessionId"))
                .and_then(Value::as_str)
                .map(str::to_string);
            let target_type = session
                .and_then(|value| value.get("targetType"))
                .and_then(Value::as_str)
                .map(str::to_string);
            let viewport_id = viewport
                .and_then(|value| value.get("viewportId"))
                .and_then(Value::as_str)
                .map(str::to_string);
            Some(XbxEngineRpcTraceSummary {
                method: "StartRuntime",
                received_event: "runtimeStartRpcReceived",
                completed_event: "runtimeStartRpcCompleted",
                session_id: session_id.clone(),
                viewport_id: viewport_id.clone(),
                target_type: target_type.clone(),
                payload: json!({
                    "method": "StartRuntime",
                    "sessionId": session_id,
                    "viewportId": viewport_id,
                    "targetType": target_type,
                }),
            })
        }
        _ => None,
    }
}
