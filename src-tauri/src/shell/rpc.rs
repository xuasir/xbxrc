use crate::mods;
use serde::{Deserialize, Serialize};
use std::time::Instant;

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

    // log::info!("[RPC][IN] {}.{}", namespace, method);

    let result = match payload.namespace.as_str() {
        "app" => {
            mods::app_state::rpc::handle_rpc(payload.method.as_str(), payload.params, app_handle)
                .await
                .map_err(Into::into)
        }
        "config" => {
            mods::config::rpc::handle_rpc(payload.method.as_str(), payload.params, app_handle)
                .await
                .map_err(Into::into)
        }
        "auth" => mods::auth::rpc::handle_rpc(payload.method.as_str(), payload.params, app_handle)
            .await
            .map_err(Into::into),
        "gamepad" => {
            mods::gamepad::rpc::handle_rpc(payload.method.as_str(), payload.params, app_handle)
                .await
                .map_err(Into::into)
        }
        "data" => mods::data::rpc::handle_rpc(payload.method.as_str(), payload.params, app_handle)
            .await
            .map_err(Into::into),
        "streaming" => {
            mods::streaming::rpc::handle_rpc(payload.method.as_str(), payload.params, app_handle)
                .await
                .map_err(Into::into)
        }
        "xbxEngine" => {
            mods::xbxengine::rpc::handle_rpc(payload.method.as_str(), payload.params, app_handle)
                .await
                .map_err(Into::into)
        }
        "system" => mods::app_state::rpc::handle_system_rpc(
            payload.method.as_str(),
            payload.params,
            app_handle,
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
