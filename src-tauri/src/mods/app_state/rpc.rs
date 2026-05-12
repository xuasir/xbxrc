use crate::error::AppResult;
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

#[derive(Debug, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "camelCase")]
pub enum AppCommand {
    GetVersion,
    Ping {
        message: String,
    },
    IsFullscreen,
    ToggleFullscreen,
    EnterFullscreen,
    ExitFullscreen,
    GetStartupFlags,
    ResetAutoConnect,
    ClearUserData,
    ClearData,
    #[serde(rename_all = "camelCase")]
    SaveBinaryFile {
        suggested_name: String,
        data_base64: String,
    },
    Restart,
    Quit,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "camelCase")]
pub enum SystemCommand {
    OpenExternal { url: String },
}

pub async fn handle_rpc(
    method: &str,
    params: Option<Value>,
    app_handle: AppHandle,
) -> AppResult<Value> {
    let state = app_handle.state::<crate::AppState>();
    let service = state.app_state.clone();

    let json_cmd = match params {
        Some(p) => json!({ "method": method, "params": p }),
        None => json!({ "method": method }),
    };

    let command: AppCommand = serde_json::from_value(json_cmd).map_err(|e| {
        crate::error::AppError::InvalidParams(format!("Invalid app command params: {}", e))
    })?;

    match command {
        AppCommand::GetVersion => Ok(json!(service.get_version())),
        AppCommand::Ping { message } => Ok(serde_json::to_value(service.ping(&message))?),
        AppCommand::IsFullscreen => Ok(json!(service.is_fullscreen())),
        AppCommand::ToggleFullscreen => Ok(json!(service
            .toggle_fullscreen()
            .map_err(|e| e.to_string())?)),
        AppCommand::EnterFullscreen => Ok(json!(service
            .enter_fullscreen()
            .map_err(|e| e.to_string())?)),
        AppCommand::ExitFullscreen => Ok(json!(service
            .exit_fullscreen()
            .map_err(|e| e.to_string())?)),
        AppCommand::GetStartupFlags => Ok(serde_json::to_value(service.get_startup_flags().await)?),
        AppCommand::ResetAutoConnect => Ok(json!({ "reset": service.reset_auto_connect().await })),
        AppCommand::ClearUserData => {
            let result = service.clear_user_data().await.map_err(|e| e.to_string())?;
            Ok(serde_json::to_value(result)?)
        }
        AppCommand::ClearData => {
            let result = service.clear_data().await.map_err(|e| e.to_string())?;
            service.restart_delayed(10).await;
            Ok(json!({
                "cleared": result.cleared,
                "legacyStateCleared": result.legacy_state_cleared,
                "restarted": true
            }))
        }
        AppCommand::SaveBinaryFile {
            suggested_name,
            data_base64,
        } => Ok(serde_json::to_value(
            service.save_binary_file(&suggested_name, &data_base64)?,
        )?),
        AppCommand::Restart => {
            service.restart().await;
            Ok(json!({ "accepted": true }))
        }
        AppCommand::Quit => {
            service.quit().await;
            Ok(json!({ "accepted": true }))
        }
    }
}

pub async fn handle_system_rpc(
    method: &str,
    params: Option<Value>,
    app_handle: AppHandle,
) -> AppResult<Value> {
    let state = app_handle.state::<crate::AppState>();
    let service = state.app_state.clone();

    let json_cmd = match params {
        Some(p) => json!({ "method": method, "params": p }),
        None => json!({ "method": method }),
    };

    let command: SystemCommand = serde_json::from_value(json_cmd).map_err(|e| {
        crate::error::AppError::InvalidParams(format!("Invalid system command params: {}", e))
    })?;

    match command {
        SystemCommand::OpenExternal { url } => {
            service.open_external(&url).map_err(|e| e.to_string())?;
            Ok(json!({}))
        }
    }
}
