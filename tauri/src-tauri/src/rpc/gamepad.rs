use crate::AppState;
use ohmygamepad_protocol::{
    LogicalPadBindingDto, MultiControllerSamplingStrategyDto, OhMyGamepadRouteTargetDto,
    OhMyGamepadRumbleRequestDto, OhMyGamepadRumbleTargetDto, OhMyGamepadSamplingConfigDto,
};
use serde_json::Value;
use tauri::{AppHandle, Manager};

pub async fn handle_gamepad_rpc(
    method: &str,
    params: Option<Value>,
    app_handle: AppHandle,
) -> Result<Value, String> {
    let state = app_handle.state::<AppState>();
    let service = state.gamepad.clone();

    match method {
        "getRuntimeSnapshot" => {
            let snapshot = service.get_runtime_snapshot()?;
            serde_json::to_value(snapshot).map_err(|error| error.to_string())
        }
        "setRouteTarget" => {
            let target: OhMyGamepadRouteTargetDto = params
                .as_ref()
                .and_then(|payload| payload.get("target"))
                .and_then(|value| serde_json::from_value(value.clone()).ok())
                .ok_or_else(|| "Missing or invalid target parameter".to_string())?;

            let snapshot = service.set_route_target(target)?;
            serde_json::to_value(snapshot).map_err(|error| error.to_string())
        }
        "updateSampling" => {
            let sampling: OhMyGamepadSamplingConfigDto = params
                .as_ref()
                .and_then(|payload| payload.get("sampling"))
                .and_then(|value| serde_json::from_value(value.clone()).ok())
                .ok_or_else(|| "Missing or invalid sampling parameter".to_string())?;

            let snapshot = service.update_sampling(sampling)?;
            serde_json::to_value(snapshot).map_err(|error| error.to_string())
        }
        "rebindLogicalPad" => {
            let binding: LogicalPadBindingDto = params
                .as_ref()
                .and_then(|payload| payload.get("binding"))
                .and_then(|value| serde_json::from_value(value.clone()).ok())
                .ok_or_else(|| "Missing or invalid binding parameter".to_string())?;

            let snapshot = service.rebind_logical_pad(binding)?;
            serde_json::to_value(snapshot).map_err(|error| error.to_string())
        }
        "setSamplingStrategy" => {
            let strategy: MultiControllerSamplingStrategyDto = params
                .as_ref()
                .and_then(|payload| payload.get("strategy"))
                .and_then(|value| serde_json::from_value(value.clone()).ok())
                .ok_or_else(|| "Missing or invalid strategy parameter".to_string())?;

            let snapshot = service.set_sampling_strategy(strategy)?;
            serde_json::to_value(snapshot).map_err(|error| error.to_string())
        }
        "setPrimarySamplingDevice" => {
            let device_id: Option<String> = params
                .as_ref()
                .and_then(|payload| payload.get("deviceId"))
                .and_then(|value| serde_json::from_value(value.clone()).ok());

            let snapshot = service.set_primary_sampling_device(device_id)?;
            serde_json::to_value(snapshot).map_err(|error| error.to_string())
        }
        "pauseSamplingDevice" => {
            let device_id: String = params
                .as_ref()
                .and_then(|payload| payload.get("deviceId"))
                .and_then(Value::as_str)
                .map(|value| value.to_string())
                .ok_or_else(|| "Missing deviceId parameter".to_string())?;

            let snapshot = service.pause_sampling_device(&device_id)?;
            serde_json::to_value(snapshot).map_err(|error| error.to_string())
        }
        "resumeSamplingDevice" => {
            let device_id: String = params
                .as_ref()
                .and_then(|payload| payload.get("deviceId"))
                .and_then(Value::as_str)
                .map(|value| value.to_string())
                .ok_or_else(|| "Missing deviceId parameter".to_string())?;

            let snapshot = service.resume_sampling_device(&device_id)?;
            serde_json::to_value(snapshot).map_err(|error| error.to_string())
        }
        "playRumble" => {
            let request: OhMyGamepadRumbleRequestDto = params
                .as_ref()
                .and_then(|payload| payload.get("request"))
                .and_then(|value| serde_json::from_value(value.clone()).ok())
                .ok_or_else(|| "Missing or invalid request parameter".to_string())?;

            let result = service.play_rumble(request)?;
            serde_json::to_value(result).map_err(|error| error.to_string())
        }
        "stopRumble" => {
            let target: OhMyGamepadRumbleTargetDto = params
                .as_ref()
                .and_then(|payload| payload.get("target"))
                .and_then(|value| serde_json::from_value(value.clone()).ok())
                .ok_or_else(|| "Missing or invalid target parameter".to_string())?;

            let result = service.stop_rumble(target)?;
            serde_json::to_value(result).map_err(|error| error.to_string())
        }
        _ => Err(format!("Unknown method in gamepad: {}", method)),
    }
}
