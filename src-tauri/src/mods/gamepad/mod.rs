pub mod events;
pub mod rpc;
pub mod service;

pub use service::GamepadService;

use ohmygamepad_protocol::{
    OhMyGamepadRumbleRequestDto, OhMyGamepadRumbleResultDto, OhMyGamepadRumbleTargetDto,
    OhMyGamepadRuntimeSnapshotDto,
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
    fn start_backend(&self) -> Result<OhMyGamepadRuntimeSnapshotDto, String>;
    fn stop_backend(&self) -> Result<(), String>;
    fn get_runtime_snapshot(&self) -> Result<OhMyGamepadRuntimeSnapshotDto, String>;
    fn play_rumble(
        &self,
        request: OhMyGamepadRumbleRequestDto,
    ) -> Result<OhMyGamepadRumbleResultDto, String>;
    fn stop_rumble(
        &self,
        target: OhMyGamepadRumbleTargetDto,
    ) -> Result<OhMyGamepadRumbleResultDto, String>;
    fn shutdown(&self) {
        let _ = self.stop_backend();
    }
}

pub type GamepadProviderRef = Arc<dyn GamepadProvider>;
