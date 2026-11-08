use crate::mods;
use serde::{Deserialize, Serialize};
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
                message: match &error {
                    crate::error::AppError::Data(message) => message.clone(),
                    _ => error.to_string(),
                },
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
        "data" => {
            mods::data::rpc::handle_rpc(payload.method.as_str(), payload.params, app_handle.clone())
                .await
                .map_err(|error| match error {
                    crate::error::AppError::Internal(message) => {
                        crate::error::AppError::Data(message)
                    }
                    other => other,
                })
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
        "system" => mods::app_state::rpc::handle_system_rpc(
            payload.method.as_str(),
            payload.params,
            app_handle.clone(),
        )
        .await
        .map_err(Into::into),
        "updater" => mods::updater::rpc::handle_rpc(
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
            // log::info!(
            //     "[RPC][OUT] {}.{} ok duration={}ms",
            //     namespace,
            //     method,
            //     duration_ms
            // );
            RpcEnvelope::success(data)
        }
        Err(error) => {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_error_preserves_user_message_without_internal_prefix() {
        let envelope =
            RpcEnvelope::fail(crate::error::AppError::Data("查询超时，请稍后刷新".into()));
        let error = envelope.error.unwrap();
        assert_eq!(error.code, "DATA_ERROR");
        assert_eq!(error.message, "查询超时，请稍后刷新");
    }

    #[test]
    fn streaming_details_and_other_error_codes_keep_their_contract() {
        let details = serde_json::json!({"reason": "disconnected"});
        let envelope = RpcEnvelope::fail(crate::error::AppError::streaming_detailed(
            "连接中断",
            details.clone(),
        ));
        let error = envelope.error.unwrap();
        assert_eq!(error.code, "STREAMING_ERROR");
        assert_eq!(error.message, "Streaming error: 连接中断");
        assert_eq!(error.details, Some(details));
        let invalid = RpcEnvelope::fail(crate::error::AppError::InvalidParams(
            "missing parameter".into(),
        ));
        assert_eq!(invalid.error.unwrap().code, "APP_INVALID_PARAMS");
    }
}
