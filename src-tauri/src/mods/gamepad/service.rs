use crate::mods::gamepad::events;
use crate::mods::gamepad::{
    GamepadAxisMappingDto, GamepadButtonMappingDto, GamepadDeviceProfileDto,
    GamepadDeviceProfileMatcherDto, GamepadFilterConfigDto, GamepadProvider,
};
use crate::settings_store::SettingsStoreResolver;
use ohmygamepad_core::{AxisMapping, ButtonMapping, DeviceProfile, DeviceProfileMatcher, FilterConfig};
use ohmygamepad_host::GamepadRuntimeHost;
use ohmygamepad_protocol::{
    LogicalPadBindingDto, MultiControllerSamplingStrategyDto, OhMyGamepadRouteTargetDto,
    OhMyGamepadBackendKindDto, OhMyGamepadKeyboardMappingDto,
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

    fn replace_device_profiles(
        &self,
        profiles: Vec<GamepadDeviceProfileDto>,
    ) -> Result<OhMyGamepadRuntimeSnapshotDto, String> {
        let mapped = profiles
            .into_iter()
            .map(to_core_profile)
            .collect::<Result<Vec<_>, _>>()?;
        self.host
            .replace_device_profiles(mapped)
            .map_err(|error| format!("{:?}", error))?;
        let _ = self.emit_runtime_events();
        self.get_runtime_snapshot()
    }

    fn replace_keyboard_mapping(
        &self,
        mapping: OhMyGamepadKeyboardMappingDto,
    ) -> Result<OhMyGamepadRuntimeSnapshotDto, String> {
        self.host
            .replace_keyboard_mapping(mapping)
            .map_err(|error| format!("{:?}", error))?;
        let _ = self.emit_runtime_events();
        self.get_runtime_snapshot()
    }

    fn reset_device_profiles(&self) -> Result<OhMyGamepadRuntimeSnapshotDto, String> {
        self.host
            .replace_device_profiles(Vec::new())
            .map_err(|error| format!("{:?}", error))?;
        let _ = self.emit_runtime_events();
        self.get_runtime_snapshot()
    }

    fn reset_keyboard_mapping(&self) -> Result<OhMyGamepadRuntimeSnapshotDto, String> {
        self.host
            .replace_keyboard_mapping(OhMyGamepadKeyboardMappingDto::default())
            .map_err(|error| format!("{:?}", error))?;
        let _ = self.emit_runtime_events();
        self.get_runtime_snapshot()
    }

    fn shutdown(&self) {
        let _ = self
            .host
            .set_route_target(OhMyGamepadRouteTargetDto::ShellUi);
    }
}

fn parse_backend_kind(raw: Option<String>) -> Result<Option<OhMyGamepadBackendKindDto>, String> {
    let Some(value) = raw else {
        return Ok(None);
    };
    match value.as_str() {
        "gilrs" => Ok(Some(OhMyGamepadBackendKindDto::Gilrs)),
        "mock" => Ok(Some(OhMyGamepadBackendKindDto::Mock)),
        other => Err(format!("Unsupported backend kind: {other}")),
    }
}

fn to_core_profile(profile: GamepadDeviceProfileDto) -> Result<DeviceProfile, String> {
    Ok(DeviceProfile {
        matcher: to_core_matcher(profile.matcher)?,
        buttons: to_core_buttons(profile.buttons),
        axes: to_core_axes(profile.axes),
        filter: to_core_filter(profile.filter),
    })
}

fn to_core_matcher(matcher: GamepadDeviceProfileMatcherDto) -> Result<DeviceProfileMatcher, String> {
    Ok(DeviceProfileMatcher {
        device_id: matcher.device_id,
        vendor_id: matcher.vendor_id,
        product_id: matcher.product_id,
        backend: parse_backend_kind(matcher.backend)?,
        name_contains: matcher.name_contains,
    })
}

fn to_core_buttons(buttons: GamepadButtonMappingDto) -> ButtonMapping {
    ButtonMapping {
        south: buttons.south,
        east: buttons.east,
        west: buttons.west,
        north: buttons.north,
        l1: buttons.l1,
        r1: buttons.r1,
        l2: buttons.l2,
        r2: buttons.r2,
        view: buttons.view,
        menu: buttons.menu,
        l3: buttons.l3,
        r3: buttons.r3,
        dpad_up: buttons.dpad_up,
        dpad_down: buttons.dpad_down,
        dpad_left: buttons.dpad_left,
        dpad_right: buttons.dpad_right,
        home: buttons.home,
    }
}

fn to_core_axes(axes: GamepadAxisMappingDto) -> AxisMapping {
    AxisMapping {
        left_stick_x: axes.left_stick_x,
        left_stick_y: axes.left_stick_y,
        right_stick_x: axes.right_stick_x,
        right_stick_y: axes.right_stick_y,
        left_trigger_button: axes.left_trigger_button,
        right_trigger_button: axes.right_trigger_button,
        left_trigger_axis: axes.left_trigger_axis,
        right_trigger_axis: axes.right_trigger_axis,
    }
}

fn to_core_filter(filter: GamepadFilterConfigDto) -> FilterConfig {
    FilterConfig {
        stick_deadzone: filter.stick_deadzone,
        stick_epsilon: filter.stick_epsilon,
        trigger_deadzone: filter.trigger_deadzone,
        trigger_epsilon: filter.trigger_epsilon,
        button_epsilon: filter.button_epsilon,
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
        let service = Self { app_handle, host };
        service.apply_persisted_mappings();
        service
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

    fn apply_persisted_mappings(&self) {
        let Ok(store) = SettingsStoreResolver::new(self.app_handle.clone()).open_read() else {
            return;
        };

        if let Some(value) = store.store().get("gamepad_device_profiles") {
            if let Ok(profiles) = serde_json::from_value::<Vec<GamepadDeviceProfileDto>>(value.clone()) {
                // 保护性迁移：拒绝 matcher 全空的 profile，避免“全局误匹配”导致 Windows/Xbox 输入异常。
                let sanitized_profiles = profiles
                    .into_iter()
                    .filter(|profile| !matcher_is_empty(&profile.matcher))
                    .collect::<Vec<_>>();
                let _ = self.replace_device_profiles(sanitized_profiles);
            }
        }

        if let Some(value) = store.store().get("gamepad_keyboard_mapping") {
            if let Ok(mapping) = serde_json::from_value::<OhMyGamepadKeyboardMappingDto>(value.clone()) {
                let _ = self.replace_keyboard_mapping(mapping);
            }
        }
    }
}

fn matcher_is_empty(matcher: &GamepadDeviceProfileMatcherDto) -> bool {
    matcher.device_id.as_deref().map(str::trim).map(str::is_empty).unwrap_or(true)
        && matcher.vendor_id.is_none()
        && matcher.product_id.is_none()
        && matcher.backend.as_deref().map(str::trim).map(str::is_empty).unwrap_or(true)
        && matcher
            .name_contains
            .as_deref()
            .map(str::trim)
            .map(str::is_empty)
            .unwrap_or(true)
}
