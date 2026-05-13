use crate::error::AppResult;
use crate::mods::gamepad::GamepadDeviceProfileDto;
use crate::AppState;
use ohmygamepad_protocol::{
    MultiControllerSamplingStrategyDto, OhMyGamepadKeyboardMappingDto, OhMyGamepadRumbleRequestDto,
    OhMyGamepadRumbleTargetDto, OhMyGamepadSamplingConfigDto,
};
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

#[derive(Debug, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "camelCase")]
pub enum GamepadCommand {
    GetRuntimeSnapshot,
    SetStreamPadForwarding {
        enabled: bool,
    },
    ActivateSampling,
    ResumeShellSampling,
    UpdateSampling {
        sampling: OhMyGamepadSamplingConfigDto,
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
    ReplaceDeviceProfiles {
        profiles: Vec<GamepadDeviceProfileDto>,
    },
    ReplaceKeyboardMapping {
        mapping: OhMyGamepadKeyboardMappingDto,
    },
    ResetDeviceProfiles,
    ResetKeyboardMapping,
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
        GamepadCommand::SetStreamPadForwarding { enabled } => Ok(serde_json::to_value(
            service.set_stream_pad_forwarding(enabled)?,
        )?),
        GamepadCommand::ActivateSampling => Ok(serde_json::to_value(service.activate_sampling()?)?),
        GamepadCommand::ResumeShellSampling => {
            Ok(serde_json::to_value(service.resume_shell_sampling()?)?)
        }
        GamepadCommand::UpdateSampling { sampling } => {
            Ok(serde_json::to_value(service.update_sampling(sampling)?)?)
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
        GamepadCommand::ReplaceDeviceProfiles { profiles } => Ok(serde_json::to_value(
            service.replace_device_profiles(profiles)?,
        )?),
        GamepadCommand::ReplaceKeyboardMapping { mapping } => Ok(serde_json::to_value(
            service.replace_keyboard_mapping(mapping)?,
        )?),
        GamepadCommand::ResetDeviceProfiles => {
            Ok(serde_json::to_value(service.reset_device_profiles()?)?)
        }
        GamepadCommand::ResetKeyboardMapping => {
            Ok(serde_json::to_value(service.reset_keyboard_mapping()?)?)
        }
    }
}
