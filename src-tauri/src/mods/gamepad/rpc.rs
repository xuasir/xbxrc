use crate::{error::AppResult, shell::state::AppState};
use ohmygamepad_protocol::{OhMyGamepadRumbleRequestDto, OhMyGamepadRumbleTargetDto};
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

#[derive(Debug, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "camelCase")]
pub enum GamepadCommand {
    StartBackend,
    StopBackend,
    GetRuntimeSnapshot,
    PlayRumble {
        request: OhMyGamepadRumbleRequestDto,
    },
    StopRumble {
        target: OhMyGamepadRumbleTargetDto,
    },
    #[serde(rename_all = "camelCase")]
    SubmitStreamState {
        session_id: String,
        sequence: u64,
        state: ohmygamepad_protocol::LogicalPadStateDto,
    },
}

pub async fn handle_rpc(
    method: &str,
    params: Option<Value>,
    app_handle: AppHandle,
) -> AppResult<Value> {
    let service = app_handle.state::<AppState>().gamepad.clone();
    let command: GamepadCommand = serde_json::from_value(match params {
        Some(p) => json!({ "method": method, "params": p }),
        None => json!({ "method": method }),
    })
    .map_err(|e| crate::error::AppError::InvalidParams(format!("Invalid gamepad command: {e}")))?;
    // SDL 初始化、关闭与设备命令在阻塞池执行。
    tauri::async_runtime::spawn_blocking(move || -> AppResult<Value> {
        match command {
            GamepadCommand::StartBackend => Ok(serde_json::to_value(service.start_backend()?)?),
            GamepadCommand::StopBackend => {
                service.stop_backend()?;
                Ok(Value::Null)
            }
            GamepadCommand::GetRuntimeSnapshot => {
                Ok(serde_json::to_value(service.get_runtime_snapshot()?)?)
            }
            GamepadCommand::PlayRumble { request } => {
                Ok(serde_json::to_value(service.play_rumble(request)?)?)
            }
            GamepadCommand::StopRumble { target } => {
                Ok(serde_json::to_value(service.stop_rumble(target)?)?)
            }
            GamepadCommand::SubmitStreamState {
                session_id,
                sequence,
                state,
            } => Ok(json!(xbxengine::submit_application_gamepad_state(
                &session_id,
                sequence,
                state
            ))),
        }
    })
    .await
    .map_err(|e| crate::error::AppError::Internal(e.to_string()))?
}
