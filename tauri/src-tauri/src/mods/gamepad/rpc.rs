use crate::error::AppResult;
use crate::AppState;
use ohmygamepad_protocol::{
    LogicalPadBindingDto, MultiControllerSamplingStrategyDto, OhMyGamepadRouteTargetDto,
    OhMyGamepadRumbleRequestDto, OhMyGamepadRumbleTargetDto, OhMyGamepadSamplingConfigDto,
};
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

#[derive(Debug, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "camelCase")]
pub enum GamepadCommand {
    GetRuntimeSnapshot,
    SetRouteTarget {
        target: OhMyGamepadRouteTargetDto,
    },
    UpdateSampling {
        sampling: OhMyGamepadSamplingConfigDto,
    },
    RebindLogicalPad {
        binding: LogicalPadBindingDto,
    },
    SetSamplingStrategy {
        strategy: MultiControllerSamplingStrategyDto,
    },
    SetPrimarySamplingDevice {
        device_id: Option<String>,
    },
    PauseSamplingDevice {
        device_id: String,
    },
    ResumeSamplingDevice {
        device_id: String,
    },
    PlayRumble {
        request: OhMyGamepadRumbleRequestDto,
    },
    StopRumble {
        target: OhMyGamepadRumbleTargetDto,
    },
}

pub async fn handle_rpc(
    method: &str,
    params: Option<Value>,
    app_handle: AppHandle,
) -> AppResult<Value> {
    let state = app_handle.state::<AppState>();
    let service = state.gamepad.clone();

    let json_cmd = match params {
        Some(p) => json!({ "method": method, "params": p }),
        None => json!({ "method": method }),
    };

    let command: GamepadCommand = serde_json::from_value(json_cmd).map_err(|e| {
        crate::error::AppError::InvalidParams(format!("Invalid gamepad command params: {}", e))
    })?;

    match command {
        GamepadCommand::GetRuntimeSnapshot => {
            Ok(serde_json::to_value(service.get_runtime_snapshot()?)?)
        }
        GamepadCommand::SetRouteTarget { target } => {
            Ok(serde_json::to_value(service.set_route_target(target)?)?)
        }
        GamepadCommand::UpdateSampling { sampling } => {
            Ok(serde_json::to_value(service.update_sampling(sampling)?)?)
        }
        GamepadCommand::RebindLogicalPad { binding } => {
            Ok(serde_json::to_value(service.rebind_logical_pad(binding)?)?)
        }
        GamepadCommand::SetSamplingStrategy { strategy } => Ok(serde_json::to_value(
            service.set_sampling_strategy(strategy)?,
        )?),
        GamepadCommand::SetPrimarySamplingDevice { device_id } => Ok(serde_json::to_value(
            service.set_primary_sampling_device(device_id)?,
        )?),
        GamepadCommand::PauseSamplingDevice { device_id } => Ok(serde_json::to_value(
            service.pause_sampling_device(&device_id)?,
        )?),
        GamepadCommand::ResumeSamplingDevice { device_id } => Ok(serde_json::to_value(
            service.resume_sampling_device(&device_id)?,
        )?),
        GamepadCommand::PlayRumble { request } => {
            Ok(serde_json::to_value(service.play_rumble(request)?)?)
        }
        GamepadCommand::StopRumble { target } => {
            Ok(serde_json::to_value(service.stop_rumble(target)?)?)
        }
    }
}
