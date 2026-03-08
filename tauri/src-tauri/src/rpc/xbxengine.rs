use crate::AppState;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

pub async fn handle_xbxengine_rpc(
    method: &str,
    params: Option<Value>,
    app_handle: AppHandle,
) -> Result<Value, String> {
    let state = app_handle.state::<AppState>();
    let service = state.xbxengine.clone();

    match method {
        "startRuntime" => {
            service.dispatch_control("StartRuntime", params).await?;
            Ok(json!({ "accepted": true }))
        }
        "requestReconnect" => {
            service.dispatch_control("RequestReconnect", params).await?;
            Ok(json!({ "accepted": true }))
        }
        "stopRuntime" => {
            service.dispatch_control("StopRuntime", None).await?;
            Ok(json!({ "accepted": true }))
        }
        "attachViewport" => {
            service.dispatch_control("AttachViewport", params).await?;
            Ok(json!({ "accepted": true }))
        }
        "detachViewport" => {
            service.dispatch_control("DetachViewport", None).await?;
            Ok(json!({ "accepted": true }))
        }
        "applyDisplayState" => {
            service
                .dispatch_control("ApplyDisplayState", params)
                .await?;
            Ok(json!({ "accepted": true }))
        }
        "pressControllerButton" => {
            service
                .dispatch_control("PressControllerButton", params)
                .await?;
            Ok(json!({ "accepted": true }))
        }
        "setKeyboardPointerEnabled" => {
            service
                .dispatch_control("SetKeyboardPointerEnabled", params)
                .await?;
            Ok(json!({ "accepted": true }))
        }
        "pushKeyboardPointerInput" => {
            service
                .dispatch_control("PushKeyboardPointerInput", params)
                .await?;
            Ok(json!({ "accepted": true }))
        }
        "setAudioVolume" => {
            service.dispatch_control("SetAudioVolume", params).await?;
            Ok(json!({ "accepted": true }))
        }
        "startMicrophone" => {
            service.dispatch_control("StartMicrophone", None).await?;
            Ok(json!({ "accepted": true }))
        }
        "stopMicrophone" => {
            service.dispatch_control("StopMicrophone", None).await?;
            Ok(json!({ "accepted": true }))
        }
        "snapshotStats" => Ok(service.snapshot_stats().await),
        "getLastRuntimeEvent" => service.get_last_runtime_event(),
        _ => Err(format!("Unknown method in xbxEngine: {}", method)),
    }
}
