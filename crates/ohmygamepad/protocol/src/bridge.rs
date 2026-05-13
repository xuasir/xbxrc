use crate::{
    GamepadSlotSnapshotDto, MultiControllerSamplingStrategyDto, OhMyGamepadDeviceDto,
    OhMyGamepadKeyboardMappingDto, OhMyGamepadRuntimeHapticsDto, OhMyGamepadSamplingConfigDto,
    SimulatedGamepadDescriptorDto,
};
use serde::{Deserialize, Serialize};

/// 对外仅 `active` / `backgroundWarm` 两态；停机语义由 `set_suspended` 独立表达（runner 内单独布尔，不在此枚举）。
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum OhMyGamepadSamplingLifecycleDto {
    #[default]
    Active,
    /// 历史 JSON 可能写为 `"suspended"`；并入 warm 一侧。
    #[serde(alias = "suspended")]
    BackgroundWarm,
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
    pub sampling: OhMyGamepadSamplingConfigDto,
    pub slots: Vec<GamepadSlotSnapshotDto>,
    pub haptics: OhMyGamepadRuntimeHapticsDto,
    /// Runtime sampling lifecycle（仅 Active / BackgroundWarm）。
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
    /// When true, logical pad samples may be forwarded to the active streaming/RTC session.
    #[serde(default)]
    pub stream_pad_forwarding: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum OhMyGamepadBridgeCommandDto {
    RefreshRuntimeSnapshot,
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
}
