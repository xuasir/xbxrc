pub mod events;
pub mod rpc;
pub mod service;

pub use service::GamepadService;

use ohmygamepad_protocol::{
    MultiControllerSamplingStrategyDto, OhMyGamepadInputPolicyDto, OhMyGamepadKeyboardMappingDto,
    OhMyGamepadRumbleRequestDto, OhMyGamepadRumbleResultDto, OhMyGamepadRumbleTargetDto,
    OhMyGamepadRuntimeSnapshotDto, OhMyGamepadSamplingConfigDto, OhMyGamepadSamplingLifecycleDto,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GamepadDeviceProfileDto {
    pub matcher: GamepadDeviceProfileMatcherDto,
    pub buttons: GamepadButtonMappingDto,
    pub axes: GamepadAxisMappingDto,
    pub filter: GamepadFilterConfigDto,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GamepadDeviceProfileMatcherDto {
    pub device_id: Option<String>,
    pub vendor_id: Option<u16>,
    pub product_id: Option<u16>,
    pub backend: Option<String>,
    pub name_contains: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GamepadButtonMappingDto {
    pub south: usize,
    pub east: usize,
    pub west: usize,
    pub north: usize,
    pub l1: usize,
    pub r1: usize,
    pub l2: usize,
    pub r2: usize,
    pub view: usize,
    pub menu: usize,
    pub l3: usize,
    pub r3: usize,
    pub dpad_up: usize,
    pub dpad_down: usize,
    pub dpad_left: usize,
    pub dpad_right: usize,
    pub home: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GamepadAxisMappingDto {
    pub left_stick_x: usize,
    pub left_stick_y: usize,
    pub right_stick_x: usize,
    pub right_stick_y: usize,
    pub left_trigger_button: usize,
    pub right_trigger_button: usize,
    pub left_trigger_axis: Option<usize>,
    pub right_trigger_axis: Option<usize>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GamepadFilterConfigDto {
    pub stick_deadzone: f32,
    pub stick_epsilon: f32,
    pub trigger_deadzone: f32,
    pub trigger_epsilon: f32,
    pub button_epsilon: f32,
}

pub trait GamepadProvider: Send + Sync {
    fn get_runtime_snapshot(&self) -> Result<OhMyGamepadRuntimeSnapshotDto, String>;
    fn set_input_policy(
        &self,
        policy: OhMyGamepadInputPolicyDto,
    ) -> Result<OhMyGamepadRuntimeSnapshotDto, String>;
    fn activate_sampling(
        &self,
        policy: Option<OhMyGamepadInputPolicyDto>,
    ) -> Result<OhMyGamepadRuntimeSnapshotDto, String>;
    fn resume_shell_sampling(
        &self,
        policy: OhMyGamepadInputPolicyDto,
    ) -> Result<OhMyGamepadRuntimeSnapshotDto, String>;
    fn update_sampling(
        &self,
        sampling: OhMyGamepadSamplingConfigDto,
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
    fn set_sampling_lifecycle(
        &self,
        lifecycle: OhMyGamepadSamplingLifecycleDto,
    ) -> Result<(), String>;
    fn try_stalled_sampling_self_heal(&self) -> Result<bool, String>;
    fn try_startup_sampling_self_heal(&self) -> Result<bool, String>;
    fn play_rumble(
        &self,
        request: OhMyGamepadRumbleRequestDto,
    ) -> Result<OhMyGamepadRumbleResultDto, String>;
    fn stop_rumble(
        &self,
        target: OhMyGamepadRumbleTargetDto,
    ) -> Result<OhMyGamepadRumbleResultDto, String>;
    fn replace_device_profiles(
        &self,
        profiles: Vec<GamepadDeviceProfileDto>,
    ) -> Result<OhMyGamepadRuntimeSnapshotDto, String>;
    fn replace_keyboard_mapping(
        &self,
        mapping: OhMyGamepadKeyboardMappingDto,
    ) -> Result<OhMyGamepadRuntimeSnapshotDto, String>;
    fn reset_device_profiles(&self) -> Result<OhMyGamepadRuntimeSnapshotDto, String>;
    fn reset_keyboard_mapping(&self) -> Result<OhMyGamepadRuntimeSnapshotDto, String>;
    fn shutdown(&self);
}

pub type GamepadProviderRef = Arc<dyn GamepadProvider>;
