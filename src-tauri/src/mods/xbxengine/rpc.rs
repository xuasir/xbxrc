use crate::error::AppResult;
use crate::AppState;
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

#[derive(Debug, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "camelCase")]
pub enum XbxEngineCommand {
    StartRuntime(Option<Value>),
    RequestReconnect(Option<Value>),
    StopRuntime,
    AttachViewport(Option<Value>),
    DetachViewport,
    ApplyDisplayState(Option<Value>),
    PressControllerButton(Option<Value>),
    SetKeyboardPointerEnabled(Option<Value>),
    PushKeyboardPointerInput(Option<Value>),
    SetAudioVolume(Option<Value>),
    StartMicrophone,
    StopMicrophone,
    SnapshotStats,
    GetLastRuntimeEvent,
}

pub async fn handle_rpc(
    method: &str,
    params: Option<Value>,
    app_handle: AppHandle,
) -> AppResult<Value> {
    let state = app_handle.state::<AppState>();
    let service = state.xbxengine.clone();

    let json_cmd = match params {
        Some(p) => json!({ "method": method, "params": p }),
        None => json!({ "method": method }),
    };

    let command: XbxEngineCommand = serde_json::from_value(json_cmd).map_err(|e| {
        crate::error::AppError::InvalidParams(format!("Invalid xbxEngine command params: {}", e))
    })?;

    match command {
        XbxEngineCommand::StartRuntime(p) => {
            service.dispatch_control("StartRuntime", p).await?;
            Ok(json!({ "accepted": true }))
        }
        XbxEngineCommand::RequestReconnect(p) => {
            service.dispatch_control("RequestReconnect", p).await?;
            Ok(json!({ "accepted": true }))
        }
        XbxEngineCommand::StopRuntime => {
            service.dispatch_control("StopRuntime", None).await?;
            Ok(json!({ "accepted": true }))
        }
        XbxEngineCommand::AttachViewport(p) => {
            service.dispatch_control("AttachViewport", p).await?;
            Ok(json!({ "accepted": true }))
        }
        XbxEngineCommand::DetachViewport => {
            service.dispatch_control("DetachViewport", None).await?;
            Ok(json!({ "accepted": true }))
        }
        XbxEngineCommand::ApplyDisplayState(p) => {
            service.dispatch_control("ApplyDisplayState", p).await?;
            Ok(json!({ "accepted": true }))
        }
        XbxEngineCommand::PressControllerButton(p) => {
            service.dispatch_control("PressControllerButton", p).await?;
            Ok(json!({ "accepted": true }))
        }
        XbxEngineCommand::SetKeyboardPointerEnabled(p) => {
            service
                .dispatch_control("SetKeyboardPointerEnabled", p)
                .await?;
            Ok(json!({ "accepted": true }))
        }
        XbxEngineCommand::PushKeyboardPointerInput(p) => {
            service
                .dispatch_control("PushKeyboardPointerInput", p)
                .await?;
            Ok(json!({ "accepted": true }))
        }
        XbxEngineCommand::SetAudioVolume(p) => {
            service.dispatch_control("SetAudioVolume", p).await?;
            Ok(json!({ "accepted": true }))
        }
        XbxEngineCommand::StartMicrophone => {
            service.dispatch_control("StartMicrophone", None).await?;
            Ok(json!({ "accepted": true }))
        }
        XbxEngineCommand::StopMicrophone => {
            service.dispatch_control("StopMicrophone", None).await?;
            Ok(json!({ "accepted": true }))
        }
        XbxEngineCommand::SnapshotStats => Ok(service.snapshot_stats().await?),
        XbxEngineCommand::GetLastRuntimeEvent => Ok(service.get_last_runtime_event()?),
    }
}
