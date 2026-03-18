use crate::error::AppResult;
use crate::AppState;
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::Manager;

#[derive(Debug, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "camelCase")]
pub enum DataCommand {
    GetUserProfile,
    GetHosts,
    GetRemoteConsoles,
    #[serde(rename_all = "camelCase")]
    GetStreamingTitleInputConfig {
        xbox_title_id: String,
    },
    #[serde(rename_all = "camelCase")]
    PowerOnConsole {
        console_id: String,
    },
    #[serde(rename_all = "camelCase")]
    PowerOffConsole {
        console_id: String,
    },
    #[serde(rename_all = "camelCase")]
    SendTextToConsole {
        console_id: String,
        text: String,
    },
    GetXcloudTitles,
    RefreshXcloudTitles,
    PrimeXcloudTitles,
}

pub async fn handle_rpc(
    method: &str,
    params: Option<Value>,
    app_handle: tauri::AppHandle,
) -> AppResult<Value> {
    let state = app_handle.state::<AppState>();
    let data = state.data.clone();

    let json_cmd = match params {
        Some(p) => json!({ "method": method, "params": p }),
        None => json!({ "method": method }),
    };

    let command: DataCommand = serde_json::from_value(json_cmd).map_err(|e| {
        crate::error::AppError::InvalidParams(format!("Invalid data command params: {}", e))
    })?;

    match command {
        DataCommand::GetUserProfile => Ok(serde_json::to_value(
            data.get_user_profile().await.map_err(|e| e.to_string())?,
        )?),
        DataCommand::GetHosts => Ok(serde_json::to_value(
            data.get_hosts().await.map_err(|e| e.to_string())?,
        )?),
        DataCommand::GetRemoteConsoles => Ok(serde_json::to_value(
            data.get_remote_consoles()
                .await
                .map_err(|e| e.to_string())?,
        )?),
        DataCommand::GetStreamingTitleInputConfig { xbox_title_id } => {
            if xbox_title_id.trim().is_empty() {
                return Err(crate::error::AppError::Internal(
                    "xboxTitleId must not be empty".to_string(),
                ));
            }
            Ok(serde_json::to_value(
                data.get_streaming_title_input_config(&xbox_title_id)
                    .await
                    .map_err(|e| e.to_string())?,
            )?)
        }
        DataCommand::PowerOnConsole { console_id } => {
            if console_id.trim().is_empty() {
                return Err(crate::error::AppError::Internal(
                    "console_id must not be empty".to_string(),
                ));
            }
            Ok(serde_json::to_value(
                data.power_on_console(&console_id)
                    .await
                    .map_err(|e| e.to_string())?,
            )?)
        }
        DataCommand::PowerOffConsole { console_id } => {
            if console_id.trim().is_empty() {
                return Err(crate::error::AppError::Internal(
                    "console_id must not be empty".to_string(),
                ));
            }
            Ok(serde_json::to_value(
                data.power_off_console(&console_id)
                    .await
                    .map_err(|e| e.to_string())?,
            )?)
        }
        DataCommand::SendTextToConsole { console_id, text } => {
            if console_id.trim().is_empty() {
                return Err(crate::error::AppError::Internal(
                    "console_id must not be empty".to_string(),
                ));
            }
            Ok(serde_json::to_value(
                data.send_text_to_console(&console_id, &text)
                    .await
                    .map_err(|e| e.to_string())?,
            )?)
        }
        DataCommand::GetXcloudTitles => Ok(serde_json::to_value(
            data.get_xcloud_titles().await.map_err(|e| e.to_string())?,
        )?),
        DataCommand::RefreshXcloudTitles => Ok(serde_json::to_value(
            data.refresh_xcloud_titles()
                .await
                .map_err(|e| e.to_string())?,
        )?),
        DataCommand::PrimeXcloudTitles => Ok(serde_json::to_value(
            data.prime_xcloud_titles()
                .await
                .map_err(|e| e.to_string())?,
        )?),
    }
}
