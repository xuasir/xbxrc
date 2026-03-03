use crate::{
    GamepadDeviceDto, GamepadRouteTargetDto, GamepadSamplingConfigDto, LogicalPadBindingDto,
    LogicalPadSnapshotDto,
};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GamepadRuntimeSnapshotDto {
    pub devices: Vec<GamepadDeviceDto>,
    pub bindings: Vec<LogicalPadBindingDto>,
    pub route_target: GamepadRouteTargetDto,
    pub sampling: GamepadSamplingConfigDto,
    pub pads: Vec<LogicalPadSnapshotDto>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum GamepadBridgeCommandDto {
    RefreshRuntimeSnapshot,
    SetRouteTarget { target: GamepadRouteTargetDto },
    UpdateSampling { sampling: GamepadSamplingConfigDto },
    RebindLogicalPad { binding: LogicalPadBindingDto },
}

#[derive(Clone, Debug, PartialEq)]
pub enum GamepadBridgeEventDto {
    RuntimeSnapshot { snapshot: GamepadRuntimeSnapshotDto },
    DevicesChanged { devices: Vec<GamepadDeviceDto> },
    PadSnapshot { snapshot: LogicalPadSnapshotDto },
    RouteChanged { target: GamepadRouteTargetDto },
}
