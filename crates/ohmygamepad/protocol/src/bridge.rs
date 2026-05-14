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

/// Tauri 推导并写入窗口 hints 后，由 host enrich 映射到快照；仅决定物理采样是否进入业务输入流。
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum OhMyGamepadInputGateModeDto {
    /// 业务输入不进入后续业务处理（物理采样仍继续）。
    #[default]
    Closed,
    /// 业务输入允许继续进入后续业务处理；UI / stream 归属由消费层决定。
    Open,
}

impl OhMyGamepadInputGateModeDto {
    pub fn allows_business_input(self) -> bool {
        matches!(self, Self::Open)
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OhMyGamepadRuntimeSnapshotDto {
    pub devices: Vec<OhMyGamepadDeviceDto>,
    pub slot_bindings: Vec<crate::GamepadSlotBindingDto>,
    pub sampling: OhMyGamepadSamplingConfigDto,
    pub slots: Vec<GamepadSlotSnapshotDto>,
    pub haptics: OhMyGamepadRuntimeHapticsDto,
    /// Runtime sampling lifecycle（仅 Active / BackgroundWarm）；诊断/兼容，业务放行判定已收口到 `input_gate`。
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
    /// When true, logical pad samples may be forwarded to the active streaming/RTC session (仍受 `input_gate` 放行约束)。
    #[serde(default)]
    pub stream_pad_forwarding: bool,
    /// 业务输入门控：由壳层窗口 hints 与 runtime lifecycle 推导并写入快照。
    #[serde(default)]
    pub input_gate: OhMyGamepadInputGateModeDto,
    /// 最近一次门控推导原因（便于诊断）。
    #[serde(default)]
    pub input_gate_reason: String,
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
