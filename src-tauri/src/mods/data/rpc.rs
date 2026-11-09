use crate::error::AppResult;
use crate::AppState;
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::Manager;

#[derive(Debug, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "camelCase")]
pub enum DataCommand {
    #[serde(rename_all = "camelCase")]
    GetPersonalAccount {
        target_xuid: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    GetGameHistory {
        target_xuid: Option<String>,
        continuation: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    GetAchievements {
        target_xuid: Option<String>,
        xbox_title_id: String,
        continuation: Option<String>,
        language: String,
    },
    #[serde(rename_all = "camelCase")]
    GetPlaytimes {
        target_xuid: Option<String>,
        xbox_title_ids: Vec<String>,
    },
    #[serde(rename_all = "camelCase")]
    GetConsoleLibrary {
        console_id: String,
    },
    #[serde(rename_all = "camelCase")]
    GetMedia {
        kind: super::types::MediaKind,
        continuation: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    ResolveMedia {
        kind: super::types::MediaKind,
        id: String,
        continuation: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    SaveMedia {
        kind: super::types::MediaKind,
        id: String,
        continuation: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    GetLibraryGame {
        target_xuid: Option<String>,
        xbox_title_id: String,
    },
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
        DataCommand::GetLibraryGame {
            target_xuid,
            xbox_title_id,
        } => Ok(serde_json::to_value(
            data.get_library_game(target_xuid, xbox_title_id).await?,
        )?),
        DataCommand::GetPersonalAccount { target_xuid } => Ok(serde_json::to_value(
            data.get_personal_account(target_xuid).await?,
        )?),
        DataCommand::GetGameHistory {
            target_xuid,
            continuation,
        } => Ok(serde_json::to_value(
            data.get_game_history(target_xuid, continuation).await?,
        )?),
        DataCommand::GetAchievements {
            target_xuid,
            xbox_title_id,
            continuation,
            language,
        } => Ok(serde_json::to_value(
            data.get_achievements(target_xuid, xbox_title_id, continuation, language)
                .await?,
        )?),
        DataCommand::GetPlaytimes {
            target_xuid,
            xbox_title_ids,
        } => Ok(serde_json::to_value(
            data.get_playtimes(target_xuid, xbox_title_ids).await?,
        )?),
        DataCommand::GetConsoleLibrary { console_id } => Ok(serde_json::to_value(
            data.get_console_library(console_id).await?,
        )?),
        DataCommand::GetMedia { kind, continuation } => Ok(serde_json::to_value(
            data.get_media(kind, continuation).await?,
        )?),
        DataCommand::ResolveMedia {
            kind,
            id,
            continuation,
        } => Ok(serde_json::to_value(
            data.resolve_media(kind, id, continuation).await?,
        )?),
        DataCommand::SaveMedia {
            kind,
            id,
            continuation,
        } => Ok(serde_json::to_value(
            data.save_media(kind, id, continuation).await?,
        )?),
        DataCommand::GetUserProfile => Ok(serde_json::to_value(data.get_user_profile().await?)?),
        DataCommand::GetHosts => Ok(serde_json::to_value(data.get_hosts().await?)?),
        DataCommand::GetRemoteConsoles => {
            Ok(serde_json::to_value(data.get_remote_consoles().await?)?)
        }
        DataCommand::GetStreamingTitleInputConfig { xbox_title_id } => {
            if xbox_title_id.trim().is_empty() {
                return Err(crate::error::AppError::Internal(
                    "xboxTitleId must not be empty".to_string(),
                ));
            }
            Ok(serde_json::to_value(
                data.get_streaming_title_input_config(&xbox_title_id)
                    .await?,
            )?)
        }
        DataCommand::PowerOnConsole { console_id } => {
            if console_id.trim().is_empty() {
                return Err(crate::error::AppError::Internal(
                    "console_id must not be empty".to_string(),
                ));
            }
            Ok(serde_json::to_value(
                data.power_on_console(&console_id).await?,
            )?)
        }
        DataCommand::PowerOffConsole { console_id } => {
            if console_id.trim().is_empty() {
                return Err(crate::error::AppError::Internal(
                    "console_id must not be empty".to_string(),
                ));
            }
            Ok(serde_json::to_value(
                data.power_off_console(&console_id).await?,
            )?)
        }
        DataCommand::SendTextToConsole { console_id, text } => {
            if console_id.trim().is_empty() {
                return Err(crate::error::AppError::Internal(
                    "console_id must not be empty".to_string(),
                ));
            }
            Ok(serde_json::to_value(
                data.send_text_to_console(&console_id, &text).await?,
            )?)
        }
        DataCommand::GetXcloudTitles => Ok(serde_json::to_value(data.get_xcloud_titles().await?)?),
        DataCommand::RefreshXcloudTitles => {
            Ok(serde_json::to_value(data.refresh_xcloud_titles().await?)?)
        }
        DataCommand::PrimeXcloudTitles => {
            Ok(serde_json::to_value(data.prime_xcloud_titles().await?)?)
        }
    }
}
