use crate::error::{AppError, AppResult};
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

use super::channel::UpdateChannel;
use super::service::channel_to_json;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetChannelParams {
    pub channel: String,
}

pub async fn handle_rpc(
    method: &str,
    params: Option<Value>,
    app_handle: AppHandle,
) -> AppResult<Value> {
    let service = app_handle
        .state::<crate::shell::state::AppState>()
        .updater
        .clone();

    match method {
        "getChannel" => {
            let channel = service.get_channel().map_err(|error| {
                AppError::Internal(format!("Failed to read update channel: {error}"))
            })?;
            Ok(channel_to_json(channel))
        }
        "setChannel" => {
            let params = params.ok_or_else(|| {
                AppError::InvalidParams("Missing updater.setChannel params".to_string())
            })?;
            let params: SetChannelParams = serde_json::from_value(params).map_err(|error| {
                AppError::InvalidParams(format!("Invalid updater.setChannel params: {error}"))
            })?;
            let channel = UpdateChannel::parse(params.channel.as_str()).ok_or_else(|| {
                AppError::InvalidParams(format!("Invalid update channel: {}", params.channel))
            })?;
            service.set_channel(channel).map_err(|error| {
                AppError::Internal(format!("Failed to save update channel: {error}"))
            })?;
            Ok(channel_to_json(channel))
        }
        "check" => {
            let result = service
                .check()
                .await
                .map_err(|error| AppError::Internal(format!("Updater check failed: {error}")))?;
            Ok(serde_json::to_value(result)?)
        }
        "downloadAndInstall" => {
            service.download_and_install().await.map_err(|error| {
                AppError::Internal(format!("Updater download/install failed: {error}"))
            })?;
            Ok(json!({ "accepted": true }))
        }
        "relaunch" => {
            service
                .relaunch()
                .map_err(|error| AppError::Internal(format!("Updater relaunch failed: {error}")))?;
            Ok(json!({ "accepted": true }))
        }
        other => Err(AppError::Internal(format!(
            "Unknown updater method: {other}"
        ))),
    }
}
