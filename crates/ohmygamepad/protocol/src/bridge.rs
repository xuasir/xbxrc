use crate::{
    GamepadSlotSnapshotDto, MultiControllerSamplingStrategyDto, OhMyGamepadDeviceDto,
    OhMyGamepadInputPolicyDto, OhMyGamepadKeyboardMappingDto, OhMyGamepadRuntimeHapticsDto,
    OhMyGamepadSamplingConfigDto, SimulatedGamepadDescriptorDto,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum OhMyGamepadSamplingLifecycleDto {
    #[default]
    Active,
    BackgroundWarm,
    Suspended,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum OhMyGamepadSamplingHealthDto {
    #[default]
    Healthy,
    AwaitingBaseline,
    Stalled,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OhMyGamepadRuntimeSnapshotDto {
    pub devices: Vec<OhMyGamepadDeviceDto>,
    pub slot_bindings: Vec<crate::GamepadSlotBindingDto>,
    pub input_policy: OhMyGamepadInputPolicyDto,
    pub sampling: OhMyGamepadSamplingConfigDto,
    pub slots: Vec<GamepadSlotSnapshotDto>,
    pub haptics: OhMyGamepadRuntimeHapticsDto,
    /// Runtime sampling lifecycle (Active / BackgroundWarm / Suspended).
    #[serde(default)]
    pub sampling_lifecycle: OhMyGamepadSamplingLifecycleDto,
    /// Sampling chain health for diagnostics and stalled self-heal.
    #[serde(default)]
    pub sampling_health: OhMyGamepadSamplingHealthDto,
    /// Last `clock_ms` when logical sample progress was observed (`sample_seq` advanced).
    #[serde(default)]
    pub last_sample_progress_at_ms: u64,
    /// Last `clock_ms` when backend delivered raw device samples into the core.
    #[serde(default)]
    pub last_backend_sample_activity_at_ms: u64,
    /// Monotonic count of backend-driven sampling self-heal attempts (for diagnostics).
    #[serde(default)]
    pub sampling_self_heal_count: u32,
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
