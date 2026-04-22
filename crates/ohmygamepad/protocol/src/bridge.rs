use crate::{
    GamepadSlotSnapshotDto, MultiControllerSamplingStrategyDto, OhMyGamepadDeviceDto,
    OhMyGamepadInputPolicyDto, OhMyGamepadKeyboardMappingDto, OhMyGamepadRuntimeHapticsDto,
    OhMyGamepadSamplingConfigDto, SimulatedGamepadDescriptorDto,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OhMyGamepadRuntimeSnapshotDto {
    pub devices: Vec<OhMyGamepadDeviceDto>,
    pub slot_bindings: Vec<crate::GamepadSlotBindingDto>,
    pub input_policy: OhMyGamepadInputPolicyDto,
    pub sampling: OhMyGamepadSamplingConfigDto,
    pub slots: Vec<GamepadSlotSnapshotDto>,
    pub haptics: OhMyGamepadRuntimeHapticsDto,
}

#[derive(Clone, Debug, PartialEq)]
pub enum OhMyGamepadBridgeCommandDto {
    RefreshRuntimeSnapshot,
    SetInputPolicy {
        policy: OhMyGamepadInputPolicyDto,
    },
    UpdateSampling {
        sampling: OhMyGamepadSamplingConfigDto,
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
    SlotSnapshot {
        snapshot: GamepadSlotSnapshotDto,
    },
    InputPolicyChanged {
        policy: OhMyGamepadInputPolicyDto,
    },
}
