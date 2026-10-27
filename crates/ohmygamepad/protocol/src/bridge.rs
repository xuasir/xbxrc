use crate::{
    GamepadSlotSnapshotDto, MultiControllerSamplingStrategyDto, OhMyGamepadDeviceDto,
    OhMyGamepadKeyboardMappingDto, OhMyGamepadRuntimeHapticsDto, OhMyGamepadSamplingConfigDto,
    SimulatedGamepadDescriptorDto,
};
use serde::{Deserialize, Serialize};

/// 采样生命周期枚举保留用于协议兼容；runtime 常驻采样后固定为 `active`。
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

/// 统一输入路由（Phase 4 引入，取代旧 gate 的真相源）。
///
/// ```text
/// foreground  = !minimized（macOS 额外要求 visible）
/// route =
///   !foreground                         → Blocked  // 等同旧 gate Closed
///   stream_pad_forwarding && !ui_capture → Stream
///   otherwise                           → Ui
/// ```
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum OhMyGamepadInputRouteDto {
    /// 输入全部阻挡（非前台）。
    #[default]
    Blocked,
    /// 输入交给 UI（菜单、诊断页、串流前 UI、overlay capturing 等）。
    Ui,
    /// 输入交给串流传输（stream_pad_forwarding && !ui_capture）。
    Stream,
}

impl OhMyGamepadInputRouteDto {
    pub fn discriminant(self) -> u8 {
        match self {
            Self::Blocked => 0,
            Self::Ui => 1,
            Self::Stream => 2,
        }
    }

    pub fn from_discriminant(value: u8) -> Self {
        match value {
            1 => Self::Ui,
            2 => Self::Stream,
            _ => Self::Blocked,
        }
    }

    pub fn allows_business_input(self) -> bool {
        matches!(self, Self::Ui | Self::Stream)
    }
}

/// 每台物理设备的标准状态，保留原始模拟幅度；业务映射与死区由公共输入层处理。
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GamepadDeviceSampleDto {
    pub device_id: String,
    pub state: crate::LogicalPadStateDto,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OhMyGamepadRuntimeSnapshotDto {
    pub devices: Vec<OhMyGamepadDeviceDto>,
    #[serde(default)]
    pub device_samples: Vec<GamepadDeviceSampleDto>,
    pub slot_bindings: Vec<crate::GamepadSlotBindingDto>,
    pub sampling: OhMyGamepadSamplingConfigDto,
    pub slots: Vec<GamepadSlotSnapshotDto>,
    pub haptics: OhMyGamepadRuntimeHapticsDto,
    /// Runtime sampling lifecycle（诊断/兼容字段）；常驻采样后固定为 `active`。
    #[serde(default)]
    pub sampling_lifecycle: OhMyGamepadSamplingLifecycleDto,
    /// Sampling chain health（诊断/兼容字段）；常驻采样后固定为 `healthy`。
    #[serde(default)]
    pub sampling_health: OhMyGamepadSamplingHealthDto,
    /// Last `clock_ms` when logical sample progress was observed (`sample_seq` advanced).
    #[serde(default)]
    pub last_sample_progress_at_ms: u64,
    /// Last `clock_ms` when backend delivered raw device samples into the core.
    #[serde(default)]
    pub last_backend_sample_activity_at_ms: u64,
    /// 采样自愈计数（诊断/兼容字段）；自愈链已删除，固定为 0。
    #[serde(default)]
    pub sampling_self_heal_count: u32,
    /// When true, logical pad samples may be forwarded to the active streaming/RTC session.
    #[serde(default)]
    pub stream_pad_forwarding: bool,
    /// 统一输入路由（Phase 4 引入，真相源）：取代旧 input_gate。
    #[serde(default)]
    pub input_route: OhMyGamepadInputRouteDto,
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
