use crate::mods::gamepad::{
    GamepadAxisMappingDto, GamepadButtonMappingDto, GamepadDeviceProfileDto,
    GamepadDeviceProfileMatcherDto, GamepadFilterConfigDto, GamepadProvider,
};
use crate::settings_store::SettingsStoreResolver;
use ohmygamepad_core::{
    AxisMapping, ButtonMapping, DeviceProfile, DeviceProfileMatcher, FilterConfig,
};
use ohmygamepad_host::GamepadRuntimeHost;
use ohmygamepad_protocol::{
    MultiControllerSamplingStrategyDto, OhMyGamepadBackendKindDto, OhMyGamepadInputGateModeDto,
    OhMyGamepadKeyboardMappingDto, OhMyGamepadRumbleRejectionReasonDto,
    OhMyGamepadRumbleRequestDto, OhMyGamepadRumbleResultDto, OhMyGamepadRumbleTargetDto,
    OhMyGamepadRuntimeSnapshotDto, OhMyGamepadSamplingConfigDto, OhMyGamepadSamplingLifecycleDto,
};
use ohmygamepad_sdl3::ShellWindowGateHints;
use tauri::{AppHandle, Manager};

pub struct GamepadService {
    app_handle: AppHandle,
    host: GamepadRuntimeHost,
}

pub fn sanitize_runtime_snapshot_for_external_consumers(
    snapshot: OhMyGamepadRuntimeSnapshotDto,
) -> OhMyGamepadRuntimeSnapshotDto {
    snapshot
}

impl GamepadService {
    fn record_trace(&self, event: &str, payload: serde_json::Value) {
        let Some(app_state) = self.app_handle.try_state::<crate::AppState>() else {
            return;
        };
        app_state
            .runtime_trace
            .record_event("gamepad-provider", event, None, payload);
    }
}

impl GamepadProvider for GamepadService {
    fn get_runtime_snapshot(&self) -> Result<OhMyGamepadRuntimeSnapshotDto, String> {
        self.host
            .snapshot()
            .map(sanitize_runtime_snapshot_for_external_consumers)
            .map_err(|error| format!("{:?}", error))
    }

    fn stream_pad_forwarding(&self) -> bool {
        self.host.stream_pad_forwarding()
    }

    fn peek_derived_input_gate(
        &self,
        sampling_lifecycle: OhMyGamepadSamplingLifecycleDto,
    ) -> (OhMyGamepadInputGateModeDto, String) {
        self.host.peek_derived_input_gate(sampling_lifecycle)
    }

    fn set_shell_window_gate_hints(&self, hints: ShellWindowGateHints) {
        self.host.set_shell_window_gate_hints(hints);
    }

    fn set_stream_pad_forwarding(
        &self,
        enabled: bool,
    ) -> Result<OhMyGamepadRuntimeSnapshotDto, String> {
        self.host
            .set_stream_pad_forwarding(enabled)
            .map_err(|error| format!("{:?}", error))?;
        self.get_runtime_snapshot()
    }

    fn activate_sampling(&self) -> Result<OhMyGamepadRuntimeSnapshotDto, String> {
        log::debug!("tauri_gamepad_activate_sampling source=provider");
        let snapshot = self
            .host
            .activate_sampling()
            .map_err(|error| format!("{:?}", error))?;
        log_runtime_snapshot(
            "activate_sampling",
            &snapshot,
            self.host.stream_pad_forwarding(),
        );
        self.record_trace(
            "activateSamplingCompleted",
            serde_json::json!({
                "samplingLifecycle": snapshot.sampling_lifecycle,
                "samplingHealth": snapshot.sampling_health,
                "streamPadForwarding": self.host.stream_pad_forwarding(),
            }),
        );
        Ok(sanitize_runtime_snapshot_for_external_consumers(snapshot))
    }

    fn resume_shell_sampling(&self) -> Result<OhMyGamepadRuntimeSnapshotDto, String> {
        log::debug!("tauri_gamepad_resume_shell_sampling source=provider");
        let snapshot = self
            .host
            .resume_shell_sampling()
            .map_err(|error| format!("{:?}", error))?;
        log_runtime_snapshot(
            "resume_shell_sampling",
            &snapshot,
            self.host.stream_pad_forwarding(),
        );
        self.record_trace(
            "resumeShellSamplingCompleted",
            serde_json::json!({
                "samplingLifecycle": snapshot.sampling_lifecycle,
                "samplingHealth": snapshot.sampling_health,
                "streamPadForwarding": self.host.stream_pad_forwarding(),
            }),
        );
        Ok(sanitize_runtime_snapshot_for_external_consumers(snapshot))
    }

    fn update_sampling(
        &self,
        sampling: OhMyGamepadSamplingConfigDto,
    ) -> Result<OhMyGamepadRuntimeSnapshotDto, String> {
        self.host
            .set_sampling(sampling)
            .map_err(|error| format!("{:?}", error))?;
        self.get_runtime_snapshot()
    }

    fn set_sampling_strategy(
        &self,
        strategy: MultiControllerSamplingStrategyDto,
    ) -> Result<OhMyGamepadRuntimeSnapshotDto, String> {
        self.host
            .set_sampling_strategy(strategy)
            .map_err(|error| format!("{:?}", error))?;
        self.get_runtime_snapshot()
    }

    fn set_primary_sampling_device(
        &self,
        device_id: Option<String>,
    ) -> Result<OhMyGamepadRuntimeSnapshotDto, String> {
        self.host
            .set_primary_sampling_device(device_id)
            .map_err(|error| format!("{:?}", error))?;
        self.get_runtime_snapshot()
    }

    fn pause_sampling_device(
        &self,
        device_id: &str,
    ) -> Result<OhMyGamepadRuntimeSnapshotDto, String> {
        self.host
            .pause_sampling_device(device_id)
            .map_err(|error| format!("{:?}", error))?;
        self.get_runtime_snapshot()
    }

    fn resume_sampling_device(
        &self,
        device_id: &str,
    ) -> Result<OhMyGamepadRuntimeSnapshotDto, String> {
        self.host
            .resume_sampling_device(device_id)
            .map_err(|error| format!("{:?}", error))?;
        self.get_runtime_snapshot()
    }

    fn set_suspended(&self, suspended: bool) -> Result<(), String> {
        log::debug!(
            "tauri_gamepad_suspend_transition source=provider suspended={}",
            suspended
        );
        self.host
            .set_suspended(suspended)
            .map_err(|error| format!("{:?}", error))
    }

    fn set_sampling_lifecycle(
        &self,
        lifecycle: OhMyGamepadSamplingLifecycleDto,
    ) -> Result<(), String> {
        log::debug!(
            "tauri_gamepad_sampling_lifecycle source=provider lifecycle={:?}",
            lifecycle
        );
        self.record_trace(
            "setSamplingLifecycleRequested",
            serde_json::json!({
                "lifecycle": lifecycle,
            }),
        );
        self.host
            .set_sampling_lifecycle(lifecycle)
            .map_err(|error| format!("{:?}", error))
    }

    fn try_stalled_sampling_self_heal(&self) -> Result<bool, String> {
        let result = self
            .host
            .try_stalled_sampling_self_heal()
            .map_err(|error| format!("{:?}", error));
        self.record_trace(
            "tryStalledSamplingSelfHealCompleted",
            serde_json::json!({
                "result": match &result {
                    Ok(applied) => serde_json::json!({
                        "ok": true,
                        "applied": applied,
                    }),
                    Err(error) => serde_json::json!({
                        "ok": false,
                        "error": error,
                    }),
                },
            }),
        );
        result
    }

    fn try_startup_sampling_self_heal(&self) -> Result<bool, String> {
        let result = self
            .host
            .try_startup_sampling_self_heal()
            .map_err(|error| format!("{:?}", error));
        self.record_trace(
            "tryStartupSamplingSelfHealCompleted",
            serde_json::json!({
                "result": match &result {
                    Ok(applied) => serde_json::json!({
                        "ok": true,
                        "applied": applied,
                    }),
                    Err(error) => serde_json::json!({
                        "ok": false,
                        "error": error,
                    }),
                },
            }),
        );
        result
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
        self.get_runtime_snapshot()
    }

    fn replace_keyboard_mapping(
        &self,
        mapping: OhMyGamepadKeyboardMappingDto,
    ) -> Result<OhMyGamepadRuntimeSnapshotDto, String> {
        self.host
            .replace_keyboard_mapping(mapping)
            .map_err(|error| format!("{:?}", error))?;
        self.get_runtime_snapshot()
    }

    fn reset_device_profiles(&self) -> Result<OhMyGamepadRuntimeSnapshotDto, String> {
        self.host
            .replace_device_profiles(Vec::new())
            .map_err(|error| format!("{:?}", error))?;
        self.get_runtime_snapshot()
    }

    fn reset_keyboard_mapping(&self) -> Result<OhMyGamepadRuntimeSnapshotDto, String> {
        self.host
            .replace_keyboard_mapping(OhMyGamepadKeyboardMappingDto::default())
            .map_err(|error| format!("{:?}", error))?;
        self.get_runtime_snapshot()
    }

    fn shutdown(&self) {
        let _ = self.host.set_stream_pad_forwarding(false);
    }
}

#[cfg(test)]
mod tests {
    use super::sanitize_runtime_snapshot_for_external_consumers;
    use ohmygamepad_protocol::{
        GamepadSlotSnapshotDto, OhMyGamepadRuntimeSnapshotDto, OhMyGamepadSamplingHealthDto,
        OhMyGamepadSamplingLifecycleDto,
    };

    #[test]
    fn sanitize_runtime_snapshot_is_identity() {
        let snapshot = OhMyGamepadRuntimeSnapshotDto {
            sampling_lifecycle: OhMyGamepadSamplingLifecycleDto::BackgroundWarm,
            sampling_health: OhMyGamepadSamplingHealthDto::Healthy,
            slots: vec![GamepadSlotSnapshotDto::default()],
            last_sample_progress_at_ms: 1234,
            last_backend_sample_activity_at_ms: 5678,
            ..Default::default()
        };

        let sanitized = sanitize_runtime_snapshot_for_external_consumers(snapshot);
        assert_eq!(sanitized.slots.len(), 1);
        assert_eq!(sanitized.last_sample_progress_at_ms, 1234);
        assert_eq!(sanitized.last_backend_sample_activity_at_ms, 5678);
    }

    #[test]
    fn active_snapshot_round_trips_through_sanitize() {
        let snapshot = OhMyGamepadRuntimeSnapshotDto {
            sampling_lifecycle: OhMyGamepadSamplingLifecycleDto::Active,
            slots: vec![GamepadSlotSnapshotDto::default()],
            ..Default::default()
        };

        let sanitized = sanitize_runtime_snapshot_for_external_consumers(snapshot);
        assert_eq!(sanitized.slots.len(), 1);
    }
}

fn parse_backend_kind(raw: Option<String>) -> Result<Option<OhMyGamepadBackendKindDto>, String> {
    let Some(value) = raw else {
        return Ok(None);
    };
    match value.as_str() {
        // 兼容历史持久化配置里的 gilrs 值，读取时统一映射到 SDL3 主语义。
        "gilrs" | "sdl3" => Ok(Some(OhMyGamepadBackendKindDto::Sdl3)),
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

fn to_core_matcher(
    matcher: GamepadDeviceProfileMatcherDto,
) -> Result<DeviceProfileMatcher, String> {
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

    fn apply_persisted_mappings(&self) {
        let Ok(store) = SettingsStoreResolver::new(self.app_handle.clone()).open_read() else {
            return;
        };

        if let Some(value) = store.store().get("gamepad_device_profiles") {
            if let Ok(profiles) =
                serde_json::from_value::<Vec<GamepadDeviceProfileDto>>(value.clone())
            {
                // 保护性迁移：拒绝 matcher 全空的 profile，避免“全局误匹配”导致 Windows/Xbox 输入异常。
                let sanitized_profiles = profiles
                    .into_iter()
                    .filter(|profile| !matcher_is_empty(&profile.matcher))
                    .collect::<Vec<_>>();
                let _ = self.replace_device_profiles(sanitized_profiles);
            }
        }

        if let Some(value) = store.store().get("gamepad_keyboard_mapping") {
            if let Ok(mapping) =
                serde_json::from_value::<OhMyGamepadKeyboardMappingDto>(value.clone())
            {
                let _ = self.replace_keyboard_mapping(mapping);
            }
        }
    }
}

fn matcher_is_empty(matcher: &GamepadDeviceProfileMatcherDto) -> bool {
    matcher
        .device_id
        .as_deref()
        .map(str::trim)
        .map(str::is_empty)
        .unwrap_or(true)
        && matcher.vendor_id.is_none()
        && matcher.product_id.is_none()
        && matcher
            .backend
            .as_deref()
            .map(str::trim)
            .map(str::is_empty)
            .unwrap_or(true)
        && matcher
            .name_contains
            .as_deref()
            .map(str::trim)
            .map(str::is_empty)
            .unwrap_or(true)
}

fn log_runtime_snapshot(
    stage: &str,
    snapshot: &OhMyGamepadRuntimeSnapshotDto,
    stream_pad_forwarding: bool,
) {
    let devices = snapshot
        .devices
        .iter()
        .map(|device| {
            format!(
                "{}|{}|connected:{}|{:04x}:{:04x}|{}|{}|{}|{}",
                device.device_id,
                device.name,
                device.connected,
                device.vendor_id.unwrap_or_default(),
                device.product_id.unwrap_or_default(),
                device.path.as_deref().unwrap_or_default(),
                device
                    .mapping
                    .as_deref()
                    .map(mapping_guid_hint)
                    .unwrap_or_default(),
                device.serial_number.as_deref().unwrap_or_default(),
                device
                    .player_index
                    .map(|value| value.to_string())
                    .unwrap_or_default(),
            )
        })
        .collect::<Vec<_>>()
        .join(";");
    log::debug!(
        "tauri_gamepad_runtime_snapshot stage={} devices={} slots={} stream_pad_forwarding={} payload=[{}]",
        stage,
        snapshot.devices.len(),
        snapshot.slots.len(),
        stream_pad_forwarding,
        devices,
    );
}

fn mapping_guid_hint(mapping: &str) -> &str {
    mapping.split(',').next().unwrap_or_default()
}
