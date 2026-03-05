use crate::{
    LogicalPadBindingDto, LogicalPadSnapshotDto, MultiControllerSamplingStrategyDto,
    OhMyGamepadDeviceDto, OhMyGamepadKeyboardMappingDto, OhMyGamepadRouteTargetDto,
    OhMyGamepadSamplingConfigDto, SimulatedGamepadDescriptorDto,
};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct OhMyGamepadRuntimeSnapshotDto {
    pub devices: Vec<OhMyGamepadDeviceDto>,
    pub bindings: Vec<LogicalPadBindingDto>,
    pub route_target: OhMyGamepadRouteTargetDto,
    pub sampling: OhMyGamepadSamplingConfigDto,
    pub pads: Vec<LogicalPadSnapshotDto>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum OhMyGamepadBridgeCommandDto {
    RefreshRuntimeSnapshot,
    SetRouteTarget {
        target: OhMyGamepadRouteTargetDto,
    },
    UpdateSampling {
        sampling: OhMyGamepadSamplingConfigDto,
    },
    RebindLogicalPad {
        binding: LogicalPadBindingDto,
    },
    SetSamplingStrategy {
        strategy: MultiControllerSamplingStrategyDto,
    },
    SetPrimarySamplingDevice {
        device_id: Option<String>,
    },
    PauseSamplingDevice {
        device_id: String,
    },
    ResumeSamplingDevice {
        device_id: String,
    },
    ConnectSimulatedGamepad {
        descriptor: SimulatedGamepadDescriptorDto,
    },
    DisconnectSimulatedGamepad {
        device_id: String,
    },
    SubmitKeyboardState {
        state: crate::LogicalPadStateDto,
    },
    ReplaceKeyboardMapping {
        mapping: OhMyGamepadKeyboardMappingDto,
    },
    SubmitSimulatedState {
        device_id: String,
        state: crate::LogicalPadStateDto,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum OhMyGamepadBridgeEventDto {
    RuntimeSnapshot {
        snapshot: OhMyGamepadRuntimeSnapshotDto,
    },
    DevicesChanged {
        devices: Vec<OhMyGamepadDeviceDto>,
    },
    PadSnapshot {
        snapshot: LogicalPadSnapshotDto,
    },
    RouteChanged {
        target: OhMyGamepadRouteTargetDto,
    },
}
