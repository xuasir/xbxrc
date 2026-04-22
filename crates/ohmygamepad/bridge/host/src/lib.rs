use std::sync::{
    mpsc::{self, Receiver},
    Arc, OnceLock,
};

use ohmygamepad_core::{
    DesktopDriverSelector, DesktopHapticsProviderKind, DeviceProfile, InputRuntimeError,
};
use ohmygamepad_sdl3::{OhMyGamepadService, OhMyGamepadServiceConfig};
use ohmygamepad_protocol::{
    LogicalPadBindingDto, LogicalPadStateDto, MultiControllerSamplingStrategyDto,
    OhMyGamepadBackendKindDto, OhMyGamepadCapabilityFlagsDto, OhMyGamepadHapticsProviderKindDto,
    OhMyGamepadKeyboardMappingDto, OhMyGamepadRouteTargetDto, OhMyGamepadRumbleRequestDto,
    OhMyGamepadRumbleResultDto, OhMyGamepadRumbleTargetDto, OhMyGamepadRuntimeHapticsDto,
    OhMyGamepadRuntimeSnapshotDto, OhMyGamepadSamplingConfigDto,
};

static SHARED_GAMEPAD_RUNTIME: OnceLock<Result<SharedGamepadRuntime, String>> = OnceLock::new();

struct SharedGamepadRuntime {
    runtime: Arc<OhMyGamepadService>,
    haptics_provider: DesktopHapticsProviderKind,
}

#[derive(Clone)]
pub struct GamepadRuntimeHost {
    runtime: Arc<OhMyGamepadService>,
    haptics_provider: DesktopHapticsProviderKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GamepadRuntimeHostError {
    message: String,
}

impl GamepadRuntimeHostError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for GamepadRuntimeHostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for GamepadRuntimeHostError {}

impl GamepadRuntimeHost {
    pub fn shared() -> Result<Self, GamepadRuntimeHostError> {
        let runtime = SHARED_GAMEPAD_RUNTIME
            .get_or_init(|| {
                bootstrap_gamepad_runtime()
                    .map_err(|error| format!("bootstrapOhMyGamepadHost:{error}"))
            })
            .as_ref()
            .map_err(|message| GamepadRuntimeHostError::new(message.clone()))?;

        Ok(Self {
            runtime: Arc::clone(&runtime.runtime),
            haptics_provider: runtime.haptics_provider,
        })
    }

    pub fn snapshot(&self) -> Result<OhMyGamepadRuntimeSnapshotDto, InputRuntimeError> {
        self.runtime
            .snapshot()
            .map(|snapshot| enrich_runtime_snapshot(snapshot, self.haptics_provider))
    }

    pub fn subscribe_runtime_snapshot(&self) -> Receiver<OhMyGamepadRuntimeSnapshotDto> {
        let source_rx = self.runtime.subscribe_runtime_snapshot();
        let (tx, rx) = mpsc::channel();
        let haptics_provider = self.haptics_provider;

        std::thread::spawn(move || {
            while let Ok(snapshot) = source_rx.recv() {
                if tx
                    .send(enrich_runtime_snapshot(snapshot, haptics_provider))
                    .is_err()
                {
                    break;
                }
            }
        });

        rx
    }

    pub fn set_route_target(
        &self,
        target: OhMyGamepadRouteTargetDto,
    ) -> Result<(), InputRuntimeError> {
        self.runtime.set_route_target(target)
    }

    pub fn set_sampling(
        &self,
        sampling: OhMyGamepadSamplingConfigDto,
    ) -> Result<(), InputRuntimeError> {
        self.runtime.set_sampling(sampling)
    }

    pub fn rebind_logical_pad(
        &self,
        binding: LogicalPadBindingDto,
    ) -> Result<(), InputRuntimeError> {
        self.runtime.rebind_logical_pad(binding)
    }

    pub fn set_sampling_strategy(
        &self,
        strategy: MultiControllerSamplingStrategyDto,
    ) -> Result<(), InputRuntimeError> {
        self.runtime.set_sampling_strategy(strategy)
    }

    pub fn set_suspended(&self, suspended: bool) -> Result<(), InputRuntimeError> {
        self.runtime.set_suspended(suspended)
    }

    pub fn set_primary_sampling_device(
        &self,
        device_id: Option<String>,
    ) -> Result<(), InputRuntimeError> {
        self.runtime.set_primary_sampling_device(device_id)
    }

    pub fn pause_sampling_device(&self, device_id: &str) -> Result<(), InputRuntimeError> {
        self.runtime.pause_sampling_device(device_id)
    }

    pub fn resume_sampling_device(&self, device_id: &str) -> Result<(), InputRuntimeError> {
        self.runtime.resume_sampling_device(device_id)
    }

    pub fn play_rumble(
        &self,
        request: OhMyGamepadRumbleRequestDto,
    ) -> Result<OhMyGamepadRumbleResultDto, InputRuntimeError> {
        self.runtime.play_rumble(request)
    }

    pub fn stop_rumble(
        &self,
        target: OhMyGamepadRumbleTargetDto,
    ) -> Result<OhMyGamepadRumbleResultDto, InputRuntimeError> {
        self.runtime.stop_rumble(target)
    }

    pub fn submit_simulated_state(
        &self,
        device_id: &str,
        state: LogicalPadStateDto,
    ) -> Result<(), GamepadRuntimeHostError> {
        self.runtime
            .submit_simulated_state(device_id, state)
            .map_err(|error| {
                GamepadRuntimeHostError::new(format!("submitSimulatedState:{error:?}"))
            })
    }

    pub fn replace_device_profiles(
        &self,
        profiles: Vec<DeviceProfile>,
    ) -> Result<(), InputRuntimeError> {
        self.runtime.replace_device_profiles(profiles)
    }

    pub fn replace_keyboard_mapping(
        &self,
        mapping: OhMyGamepadKeyboardMappingDto,
    ) -> Result<(), InputRuntimeError> {
        self.runtime
            .replace_keyboard_mapping(mapping)
            .map_err(|_| InputRuntimeError::CommandChannelClosed)
    }
}

fn bootstrap_gamepad_runtime() -> Result<SharedGamepadRuntime, String> {
    let config = OhMyGamepadServiceConfig::default();
    let selected_providers = DesktopDriverSelector::select(&config.core);

    let runtime = OhMyGamepadService::spawn(config).map_err(|error| error.to_string())?;

    Ok(SharedGamepadRuntime {
        runtime: Arc::new(runtime),
        haptics_provider: selected_providers.haptics_provider,
    })
}

fn enrich_runtime_snapshot(
    mut snapshot: OhMyGamepadRuntimeSnapshotDto,
    haptics_provider: DesktopHapticsProviderKind,
) -> OhMyGamepadRuntimeSnapshotDto {
    let default_device_id = resolve_default_device_id(&snapshot);

    for device in &mut snapshot.devices {
        device.name = normalize_device_name(device, haptics_provider);
        device.effective_capabilities = infer_effective_capabilities(device, haptics_provider);
        device.is_default_target = default_device_id.as_deref() == Some(device.device_id.as_str());
    }

    snapshot.haptics = OhMyGamepadRuntimeHapticsDto {
        provider: map_haptics_provider_kind(haptics_provider),
        supports_auto_target: true,
        supports_basic_rumble: snapshot
            .devices
            .iter()
            .any(|device| device.effective_capabilities.basic_rumble),
        supports_advanced_haptics: snapshot
            .devices
            .iter()
            .any(|device| device.effective_capabilities.advanced_haptics),
        default_device_id,
    };
    snapshot
}

fn resolve_default_device_id(snapshot: &OhMyGamepadRuntimeSnapshotDto) -> Option<String> {
    snapshot
        .pads
        .iter()
        .find_map(|pad| pad.device_ids.first().cloned())
        .or_else(|| {
            snapshot
                .devices
                .iter()
                .find(|device| device.connected)
                .map(|device| device.device_id.clone())
        })
}

fn infer_effective_capabilities(
    device: &ohmygamepad_protocol::OhMyGamepadDeviceDto,
    haptics_provider: DesktopHapticsProviderKind,
) -> OhMyGamepadCapabilityFlagsDto {
    let mut effective = device.capabilities;
    if !device.connected || !is_physical_gamepad_candidate(device) {
        return effective;
    }

    match haptics_provider {
        DesktopHapticsProviderKind::Sdl3Gamepad => {
            effective.basic_rumble = true;
            effective.advanced_haptics = false;
        }
    }

    effective
}

fn is_physical_gamepad_candidate(device: &ohmygamepad_protocol::OhMyGamepadDeviceDto) -> bool {
    device.backend == Some(OhMyGamepadBackendKindDto::Sdl3) && device.device_id != "virtual:keyboard"
}

fn normalize_device_name(
    device: &ohmygamepad_protocol::OhMyGamepadDeviceDto,
    haptics_provider: DesktopHapticsProviderKind,
) -> String {
    let trimmed = device.name.trim();
    if trimmed.is_empty() {
        return "Controller".to_owned();
    }

    if should_force_xbox_label(device, haptics_provider) {
        return "Xbox Controller".to_owned();
    }

    trimmed.to_owned()
}

fn should_force_xbox_label(
    device: &ohmygamepad_protocol::OhMyGamepadDeviceDto,
    haptics_provider: DesktopHapticsProviderKind,
) -> bool {
    let _ = (device, haptics_provider);
    false
}

fn map_haptics_provider_kind(
    provider: DesktopHapticsProviderKind,
) -> OhMyGamepadHapticsProviderKindDto {
    match provider {
        DesktopHapticsProviderKind::Sdl3Gamepad => OhMyGamepadHapticsProviderKindDto::Sdl3Gamepad,
    }
}
