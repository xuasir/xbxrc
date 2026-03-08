use serde::{Deserialize, Serialize};
use std::time::Instant;

#[derive(Debug, Deserialize)]
pub struct RpcInvokePayload {
    pub namespace: String,
    pub method: String,
    pub params: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct RpcResponse {
    pub success: bool,
    pub data: Option<serde_json::Value>,
    pub error: Option<String>,
}

#[tauri::command]
pub async fn rpc_invoke(
    payload: RpcInvokePayload,
    app_handle: tauri::AppHandle,
) -> Result<serde_json::Value, String> {
    let start = Instant::now();
    let namespace = payload.namespace.clone();
    let method = payload.method.clone();
    let has_params = payload.params.is_some();
    eprintln!("[rpc][in] {}.{} params={}", namespace, method, has_params);

    let result = match payload.namespace.as_str() {
        "app" => app::handle_app_rpc(payload.method.as_str(), payload.params, app_handle).await,
        "config" => {
            config::handle_config_rpc(payload.method.as_str(), payload.params, app_handle).await
        }
        "auth" => auth::handle_auth_rpc(payload.method.as_str(), payload.params, app_handle).await,
        "gamepad" => {
            gamepad::handle_gamepad_rpc(payload.method.as_str(), payload.params, app_handle).await
        }
        "data" => data::handle_data_rpc(payload.method.as_str(), payload.params, app_handle).await,
        "streaming" => {
            streaming::handle_streaming_rpc(payload.method.as_str(), payload.params, app_handle)
                .await
        }
        "streamHost" => {
            stream_host::handle_stream_host_rpc(payload.method.as_str(), payload.params, app_handle)
                .await
        }
        "xbxEngine" => {
            xbxengine::handle_xbxengine_rpc(payload.method.as_str(), payload.params, app_handle)
                .await
        }
        "system" => {
            system::handle_system_rpc(payload.method.as_str(), payload.params, app_handle).await
        }
        _ => Err(format!("Unknown namespace: {}", payload.namespace)),
    };

    match &result {
        Ok(_) => eprintln!(
            "[rpc][out] {}.{} ok durationMs={}",
            namespace,
            method,
            start.elapsed().as_millis()
        ),
        Err(error) => eprintln!(
            "[rpc][out] {}.{} err durationMs={} err={}",
            namespace,
            method,
            start.elapsed().as_millis(),
            error
        ),
    }

    result
}

pub mod app;
pub mod auth;
pub mod config;
pub mod data;
pub mod gamepad;
pub mod stream_host;
pub mod streaming;
pub mod system;
pub mod xbxengine;
