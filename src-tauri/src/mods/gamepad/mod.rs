pub mod events;
pub mod rpc;
pub mod service;

pub use service::GamepadService;

use ohmygamepad_protocol::{
    LogicalPadBindingDto, MultiControllerSamplingStrategyDto, OhMyGamepadRouteTargetDto,
    OhMyGamepadRumbleRequestDto, OhMyGamepadRumbleResultDto, OhMyGamepadRumbleTargetDto,
    OhMyGamepadRuntimeSnapshotDto, OhMyGamepadSamplingConfigDto,
};
use std::sync::Arc;

pub trait GamepadProvider: Send + Sync {
    fn get_runtime_snapshot(&self) -> Result<OhMyGamepadRuntimeSnapshotDto, String>;
    fn set_route_target(
        &self,
        target: OhMyGamepadRouteTargetDto,
    ) -> Result<OhMyGamepadRuntimeSnapshotDto, String>;
    fn update_sampling(
        &self,
        sampling: OhMyGamepadSamplingConfigDto,
    ) -> Result<OhMyGamepadRuntimeSnapshotDto, String>;
    fn rebind_logical_pad(
        &self,
        binding: LogicalPadBindingDto,
    ) -> Result<OhMyGamepadRuntimeSnapshotDto, String>;
    fn set_sampling_strategy(
        &self,
        strategy: MultiControllerSamplingStrategyDto,
    ) -> Result<OhMyGamepadRuntimeSnapshotDto, String>;
    fn set_primary_sampling_device(
        &self,
        device_id: Option<String>,
    ) -> Result<OhMyGamepadRuntimeSnapshotDto, String>;
    fn pause_sampling_device(
        &self,
        device_id: &str,
    ) -> Result<OhMyGamepadRuntimeSnapshotDto, String>;
    fn resume_sampling_device(
        &self,
        device_id: &str,
    ) -> Result<OhMyGamepadRuntimeSnapshotDto, String>;
    fn set_suspended(&self, suspended: bool) -> Result<(), String>;
    fn play_rumble(
        &self,
        request: OhMyGamepadRumbleRequestDto,
    ) -> Result<OhMyGamepadRumbleResultDto, String>;
    fn stop_rumble(
        &self,
        target: OhMyGamepadRumbleTargetDto,
    ) -> Result<OhMyGamepadRumbleResultDto, String>;
    fn shutdown(&self);
}

pub type GamepadProviderRef = Arc<dyn GamepadProvider>;
