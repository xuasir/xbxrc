use crate::event_bridge;
use ohmygamepad_host::GamepadRuntimeHost;
use ohmygamepad_protocol::{
    LogicalPadBindingDto, MultiControllerSamplingStrategyDto, OhMyGamepadRouteTargetDto,
    OhMyGamepadRumbleRequestDto, OhMyGamepadRumbleResultDto, OhMyGamepadRumbleTargetDto,
    OhMyGamepadRuntimeSnapshotDto, OhMyGamepadSamplingConfigDto,
};
use tauri::AppHandle;

pub struct GamepadService {
    app_handle: AppHandle,
    host: GamepadRuntimeHost,
}

impl GamepadService {
    pub fn new(app_handle: AppHandle, host: GamepadRuntimeHost) -> Self {
        Self { app_handle, host }
    }

    pub fn get_runtime_snapshot(&self) -> Result<OhMyGamepadRuntimeSnapshotDto, String> {
        self.host.snapshot().map_err(|error| format!("{:?}", error))
    }

    pub fn set_route_target(
        &self,
        target: OhMyGamepadRouteTargetDto,
    ) -> Result<OhMyGamepadRuntimeSnapshotDto, String> {
        self.host
            .set_route_target(target)
            .map_err(|error| format!("{:?}", error))?;
        self.emit_runtime_events()?;
        self.get_runtime_snapshot()
    }

    pub fn update_sampling(
        &self,
        sampling: OhMyGamepadSamplingConfigDto,
    ) -> Result<OhMyGamepadRuntimeSnapshotDto, String> {
        self.host
            .set_sampling(sampling)
            .map_err(|error| format!("{:?}", error))?;
        self.emit_runtime_events()?;
        self.get_runtime_snapshot()
    }

    pub fn rebind_logical_pad(
        &self,
        binding: LogicalPadBindingDto,
    ) -> Result<OhMyGamepadRuntimeSnapshotDto, String> {
        self.host
            .rebind_logical_pad(binding)
            .map_err(|error| format!("{:?}", error))?;
        self.emit_runtime_events()?;
        self.get_runtime_snapshot()
    }

    pub fn set_sampling_strategy(
        &self,
        strategy: MultiControllerSamplingStrategyDto,
    ) -> Result<OhMyGamepadRuntimeSnapshotDto, String> {
        self.host
            .set_sampling_strategy(strategy)
            .map_err(|error| format!("{:?}", error))?;
        self.emit_runtime_events()?;
        self.get_runtime_snapshot()
    }

    pub fn set_primary_sampling_device(
        &self,
        device_id: Option<String>,
    ) -> Result<OhMyGamepadRuntimeSnapshotDto, String> {
        self.host
            .set_primary_sampling_device(device_id)
            .map_err(|error| format!("{:?}", error))?;
        self.emit_runtime_events()?;
        self.get_runtime_snapshot()
    }

    pub fn pause_sampling_device(
        &self,
        device_id: &str,
    ) -> Result<OhMyGamepadRuntimeSnapshotDto, String> {
        self.host
            .pause_sampling_device(device_id)
            .map_err(|error| format!("{:?}", error))?;
        self.emit_runtime_events()?;
        self.get_runtime_snapshot()
    }

    pub fn resume_sampling_device(
        &self,
        device_id: &str,
    ) -> Result<OhMyGamepadRuntimeSnapshotDto, String> {
        self.host
            .resume_sampling_device(device_id)
            .map_err(|error| format!("{:?}", error))?;
        self.emit_runtime_events()?;
        self.get_runtime_snapshot()
    }

    pub fn play_rumble(
        &self,
        request: OhMyGamepadRumbleRequestDto,
    ) -> Result<OhMyGamepadRumbleResultDto, String> {
        self.host
            .play_rumble(request)
            .map_err(|error| format!("{:?}", error))
    }

    pub fn stop_rumble(
        &self,
        target: OhMyGamepadRumbleTargetDto,
    ) -> Result<OhMyGamepadRumbleResultDto, String> {
        self.host
            .stop_rumble(target)
            .map_err(|error| format!("{:?}", error))
    }

    // 退出前 best-effort 恢复到 shell 路由，避免残留串流输入路由状态。
    pub fn shutdown(&self) {
        let _ = self
            .host
            .set_route_target(OhMyGamepadRouteTargetDto::ShellUi);
    }

    fn emit_runtime_events(&self) -> Result<(), String> {
        let snapshot = self.get_runtime_snapshot()?;
        let snapshot_value = serde_json::to_value(&snapshot).map_err(|error| error.to_string())?;

        event_bridge::emit_gamepad_runtime_snapshot(&self.app_handle, &snapshot_value)?;

        let devices_value =
            serde_json::to_value(&snapshot.devices).map_err(|error| error.to_string())?;
        event_bridge::emit_gamepad_devices_changed(&self.app_handle, &devices_value)?;

        let route_value =
            serde_json::to_value(&snapshot.route_target).map_err(|error| error.to_string())?;
        event_bridge::emit_gamepad_route_changed(&self.app_handle, &route_value)?;

        for pad in &snapshot.pads {
            let pad_value = serde_json::to_value(pad).map_err(|error| error.to_string())?;
            event_bridge::emit_gamepad_pad_snapshot(&self.app_handle, &pad_value)?;
        }

        Ok(())
    }
}
