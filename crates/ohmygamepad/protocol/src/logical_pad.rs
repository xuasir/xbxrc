use crate::OhMyGamepadRouteTargetDto;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LogicalPadId {
    #[default]
    Pad0,
    Pad1,
    Pad2,
    Pad3,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LogicalStickDto {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
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

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LogicalPadStateDto {
    pub buttons: LogicalButtonsStateDto,
    pub left_stick: LogicalStickDto,
    pub right_stick: LogicalStickDto,
    pub left_trigger: f32,
    pub right_trigger: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LogicalPadSnapshotDto {
    pub pad_id: LogicalPadId,
    pub device_ids: Vec<String>,
    pub sampled_at_ms: u64,
    pub sample_seq: u64,
    pub route_target: OhMyGamepadRouteTargetDto,
    pub state: LogicalPadStateDto,
}
