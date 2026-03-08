use crate::AppState;
use serde_json::Value;
use tauri::Manager;

pub async fn handle_data_rpc(
    method: &str,
    params: Option<Value>,
    app_handle: tauri::AppHandle,
) -> Result<Value, String> {
    let state = app_handle.state::<AppState>();

    match method {
        "getUserProfile" => {
            let mut data = state.data.write().await;
            let profile = data.get_user_profile().await?;
            serde_json::to_value(profile).map_err(|error| error.to_string())
        }
        "getHosts" => {
            let mut data = state.data.write().await;
            let hosts = data.get_hosts().await?;
            serde_json::to_value(hosts).map_err(|error| error.to_string())
        }
        "getRemoteConsoles" => {
            let mut data = state.data.write().await;
            let consoles = data.get_remote_consoles().await?;
            serde_json::to_value(consoles).map_err(|error| error.to_string())
        }
        "getStreamingTitleInputConfig" => {
            let xbox_title_id = params
                .as_ref()
                .and_then(|payload| payload.get("xboxTitleId"))
                .and_then(|value| value.as_str())
                .ok_or("Missing xboxTitleId parameter")?;
            if xbox_title_id.trim().is_empty() {
                return Err("xboxTitleId must not be empty".to_string());
            }

            let mut data = state.data.write().await;
            let config = data.get_streaming_title_input_config(xbox_title_id).await?;
            serde_json::to_value(config).map_err(|error| error.to_string())
        }
        "powerOnConsole" => {
            let console_id = params
                .as_ref()
                .and_then(|payload| payload.get("consoleId"))
                .and_then(|value| value.as_str())
                .ok_or("Missing consoleId parameter")?;
            if console_id.trim().is_empty() {
                return Err("consoleId must not be empty".to_string());
            }

            let mut data = state.data.write().await;
            let result = data.power_on_console(console_id).await?;
            serde_json::to_value(result).map_err(|error| error.to_string())
        }
        "powerOffConsole" => {
            let console_id = params
                .as_ref()
                .and_then(|payload| payload.get("consoleId"))
                .and_then(|value| value.as_str())
                .ok_or("Missing consoleId parameter")?;
            if console_id.trim().is_empty() {
                return Err("consoleId must not be empty".to_string());
            }

            let mut data = state.data.write().await;
            let result = data.power_off_console(console_id).await?;
            serde_json::to_value(result).map_err(|error| error.to_string())
        }
        "sendTextToConsole" => {
            let console_id = params
                .as_ref()
                .and_then(|payload| payload.get("consoleId"))
                .and_then(|value| value.as_str())
                .ok_or("Missing consoleId parameter")?;
            if console_id.trim().is_empty() {
                return Err("consoleId must not be empty".to_string());
            }
            let text = params
                .as_ref()
                .and_then(|payload| payload.get("text"))
                .and_then(|value| value.as_str())
                .ok_or("Missing text parameter")?;

            let mut data = state.data.write().await;
            let result = data.send_text_to_console(console_id, text).await?;
            serde_json::to_value(result).map_err(|error| error.to_string())
        }
        "getXcloudTitles" => {
            let mut data = state.data.write().await;
            let titles = data.get_xcloud_titles().await?;
            serde_json::to_value(titles).map_err(|error| error.to_string())
        }
        _ => Err(format!("Unknown method in data: {}", method)),
    }
}
