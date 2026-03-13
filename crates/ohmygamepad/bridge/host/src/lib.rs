use std::sync::{mpsc::Receiver, Arc, OnceLock};

use ohmygamepad_core::{
    DesktopDriverSelector, DesktopHapticsProviderKind, DeviceProfile, InputRuntimeError,
};
use ohmygamepad_gilrs::{OhMyGamepadService, OhMyGamepadServiceConfig};
use ohmygamepad_macos_gccontroller_haptics::MacosGcControllerHapticsProvider;
use ohmygamepad_protocol::{
    LogicalPadBindingDto, LogicalPadStateDto, MultiControllerSamplingStrategyDto,
    OhMyGamepadRouteTargetDto, OhMyGamepadRumbleRequestDto, OhMyGamepadRumbleResultDto,
    OhMyGamepadRumbleTargetDto, OhMyGamepadRuntimeSnapshotDto, OhMyGamepadSamplingConfigDto,
};

static SHARED_GAMEPAD_RUNTIME: OnceLock<Result<Arc<OhMyGamepadService>, String>> = OnceLock::new();

#[derive(Clone)]
pub struct GamepadRuntimeHost {
    runtime: Arc<OhMyGamepadService>,
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
                    .map(Arc::new)
                    .map_err(|error| format!("bootstrapOhMyGamepadHost:{error}"))
            })
            .as_ref()
            .map_err(|message| GamepadRuntimeHostError::new(message.clone()))?;

        Ok(Self {
            runtime: Arc::clone(runtime),
        })
    }

    pub fn snapshot(&self) -> Result<OhMyGamepadRuntimeSnapshotDto, InputRuntimeError> {
        self.runtime.snapshot()
    }

    pub fn subscribe_runtime_snapshot(&self) -> Receiver<OhMyGamepadRuntimeSnapshotDto> {
        self.runtime.subscribe_runtime_snapshot()
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
}

fn bootstrap_gamepad_runtime() -> Result<OhMyGamepadService, String> {
    let config = OhMyGamepadServiceConfig::default();
    let selected_providers = DesktopDriverSelector::select(&config.core);

    match selected_providers.haptics_provider {
        DesktopHapticsProviderKind::MacosGcController => {
            OhMyGamepadService::spawn_with_haptics_provider(
                config,
                Box::new(MacosGcControllerHapticsProvider::default()),
            )
            .map_err(|error| error.to_string())
        }
        DesktopHapticsProviderKind::GilrsBasic | DesktopHapticsProviderKind::None => {
            OhMyGamepadService::spawn(config).map_err(|error| error.to_string())
        }
    }
}
