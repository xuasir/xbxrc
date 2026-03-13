use crate::mods::gamepad::events;
use crate::mods::gamepad::GamepadProvider;
use ohmygamepad_host::GamepadRuntimeHost;
use ohmygamepad_protocol::{
    LogicalPadBindingDto, MultiControllerSamplingStrategyDto, OhMyGamepadRouteTargetDto,
    OhMyGamepadRumbleRejectionReasonDto, OhMyGamepadRumbleRequestDto, OhMyGamepadRumbleResultDto,
    OhMyGamepadRumbleTargetDto, OhMyGamepadRuntimeSnapshotDto, OhMyGamepadSamplingConfigDto,
};
use tauri::AppHandle;

pub struct GamepadService {
    app_handle: AppHandle,
    host: GamepadRuntimeHost,
}

impl GamepadProvider for GamepadService {
    fn get_runtime_snapshot(&self) -> Result<OhMyGamepadRuntimeSnapshotDto, String> {
        self.host.snapshot().map_err(|error| format!("{:?}", error))
    }

    fn set_route_target(
        &self,
        target: OhMyGamepadRouteTargetDto,
    ) -> Result<OhMyGamepadRuntimeSnapshotDto, String> {
        self.host
            .set_route_target(target)
            .map_err(|error| format!("{:?}", error))?;
        let _ = self.emit_runtime_events();
        self.get_runtime_snapshot()
    }

    fn update_sampling(
        &self,
        sampling: OhMyGamepadSamplingConfigDto,
    ) -> Result<OhMyGamepadRuntimeSnapshotDto, String> {
        self.host
            .set_sampling(sampling)
            .map_err(|error| format!("{:?}", error))?;
        let _ = self.emit_runtime_events();
        self.get_runtime_snapshot()
    }

    fn rebind_logical_pad(
        &self,
        binding: LogicalPadBindingDto,
    ) -> Result<OhMyGamepadRuntimeSnapshotDto, String> {
        self.host
            .rebind_logical_pad(binding)
            .map_err(|error| format!("{:?}", error))?;
        let _ = self.emit_runtime_events();
        self.get_runtime_snapshot()
    }

    fn set_sampling_strategy(
        &self,
        strategy: MultiControllerSamplingStrategyDto,
    ) -> Result<OhMyGamepadRuntimeSnapshotDto, String> {
        self.host
            .set_sampling_strategy(strategy)
            .map_err(|error| format!("{:?}", error))?;
        let _ = self.emit_runtime_events();
        self.get_runtime_snapshot()
    }

    fn set_primary_sampling_device(
        &self,
        device_id: Option<String>,
    ) -> Result<OhMyGamepadRuntimeSnapshotDto, String> {
        self.host
            .set_primary_sampling_device(device_id)
            .map_err(|error| format!("{:?}", error))?;
        let _ = self.emit_runtime_events();
        self.get_runtime_snapshot()
    }

    fn pause_sampling_device(
        &self,
        device_id: &str,
    ) -> Result<OhMyGamepadRuntimeSnapshotDto, String> {
        self.host
            .pause_sampling_device(device_id)
            .map_err(|error| format!("{:?}", error))?;
        let _ = self.emit_runtime_events();
        self.get_runtime_snapshot()
    }

    fn resume_sampling_device(
        &self,
        device_id: &str,
    ) -> Result<OhMyGamepadRuntimeSnapshotDto, String> {
        self.host
            .resume_sampling_device(device_id)
            .map_err(|error| format!("{:?}", error))?;
        let _ = self.emit_runtime_events();
        self.get_runtime_snapshot()
    }

    fn set_suspended(&self, suspended: bool) -> Result<(), String> {
        self.host
            .set_suspended(suspended)
            .map_err(|error| format!("{:?}", error))
    }

    fn play_rumble(
        &self,
        request: OhMyGamepadRumbleRequestDto,
    ) -> Result<OhMyGamepadRumbleResultDto, String> {
        self.host
            .play_rumble(request)
            .or_else(|error| map_rumble_runtime_error(error, Vec::new()))
    }

    fn stop_rumble(
        &self,
        target: OhMyGamepadRumbleTargetDto,
    ) -> Result<OhMyGamepadRumbleResultDto, String> {
        self.host
            .stop_rumble(target)
            .or_else(|error| map_rumble_runtime_error(error, Vec::new()))
    }

    fn shutdown(&self) {
        let _ = self
            .host
            .set_route_target(OhMyGamepadRouteTargetDto::ShellUi);
    }
}

fn map_rumble_runtime_error(
    error: impl std::fmt::Debug,
    resolved_device_ids: Vec<String>,
) -> Result<OhMyGamepadRumbleResultDto, String> {
    let error_message = format!("{:?}", error);

    // 震动属于增强体验。当前设备/系统暂时不支持时，返回结构化拒绝结果，
    // 避免 shell RPC 把它记成真正的调用失败。
    if matches!(
        error_message.as_str(),
        "HapticsUnavailable" | "NotImplemented"
    ) {
        return Ok(OhMyGamepadRumbleResultDto::rejected(
            OhMyGamepadRumbleRejectionReasonDto::Unsupported,
            resolved_device_ids,
        ));
    }

    Err(error_message)
}

impl GamepadService {
    pub fn new(app_handle: AppHandle, host: GamepadRuntimeHost) -> Self {
        Self { app_handle, host }
    }

    fn emit_runtime_events(&self) -> Result<(), String> {
        let snapshot = self.get_runtime_snapshot()?;
        let snapshot_value = serde_json::to_value(&snapshot).map_err(|error| error.to_string())?;

        events::emit_runtime_snapshot(&self.app_handle, &snapshot_value)?;

        let devices_value =
            serde_json::to_value(&snapshot.devices).map_err(|error| error.to_string())?;
        events::emit_devices_changed(&self.app_handle, &devices_value)?;

        let route_value =
            serde_json::to_value(&snapshot.route_target).map_err(|error| error.to_string())?;
        events::emit_route_changed(&self.app_handle, &route_value)?;

        for pad in &snapshot.pads {
            let pad_value = serde_json::to_value(pad).map_err(|error| error.to_string())?;
            events::emit_pad_snapshot(&self.app_handle, &pad_value)?;
        }

        Ok(())
    }
}
