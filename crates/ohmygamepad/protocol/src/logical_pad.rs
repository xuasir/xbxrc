use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum GamepadSlotDto {
    #[default]
    #[serde(rename = "pad-0")]
    Pad0,
    #[serde(rename = "pad-1")]
    Pad1,
    #[serde(rename = "pad-2")]
    Pad2,
    #[serde(rename = "pad-3")]
    Pad3,
}

pub type LogicalPadId = GamepadSlotDto;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LogicalButtonDto {
    South,
    East,
    West,
    North,
    L1,
    R1,
    L2,
    R2,
    L3,
    R3,
    View,
    Menu,
    Home,
    DpadUp,
    DpadDown,
    DpadLeft,
    DpadRight,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogicalStickDto {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogicalButtonsStateDto {
    pub south: f32,
    pub east: f32,
    pub west: f32,
    pub north: f32,
    pub l1: f32,
    pub r1: f32,
    pub l2: f32,
    pub r2: f32,
    pub l3: f32,
    pub r3: f32,
    pub view: f32,
    pub menu: f32,
    pub home: f32,
    pub dpad_up: f32,
    pub dpad_down: f32,
    pub dpad_left: f32,
    pub dpad_right: f32,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogicalPadStateDto {
    pub buttons: LogicalButtonsStateDto,
    pub left_stick: LogicalStickDto,
    pub right_stick: LogicalStickDto,
    pub left_trigger: f32,
    pub right_trigger: f32,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GamepadSlotSnapshotDto {
    pub slot: GamepadSlotDto,
    pub device_ids: Vec<String>,
    pub sampled_at_ms: u64,
    pub sample_seq: u64,
    pub state: LogicalPadStateDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_buttons: Option<Vec<GamepadRawButtonStateDto>>,
}

pub type LogicalPadSnapshotDto = GamepadSlotSnapshotDto;

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GamepadRawButtonStateDto {
    pub index: usize,
    pub value: f32,
}
