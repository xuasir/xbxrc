use serde::{Deserialize, Serialize};

use crate::policy::types::{Owner, Switch};

/// 输入偏好：真正的输入 owner/mode 还要结合 title/runtime capability 决定。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InputConfig {
    /// 用户对输入模式的显式偏好。
    pub mode: InputPreference,
    /// 原生键鼠开关，Auto 表示跟随标题能力与宿主能力。
    pub native_mkb: Switch,
    /// 是否允许键鼠退化为虚拟手柄。
    pub virtual_mkb: bool,
    /// 触控输入策略。
    pub touch: TouchPreference,
    /// 输入轮询频率，单位 Hz。
    pub polling_rate_hz: u16,
    /// 是否允许震动输出。
    pub vibration: bool,
    /// 麦克风启用策略。
    pub microphone: MicrophonePreference,
}

impl Default for InputConfig {
    fn default() -> Self {
        Self {
            mode: InputPreference::Auto,
            native_mkb: Switch::Auto,
            virtual_mkb: false,
            touch: TouchPreference::FollowTitle,
            polling_rate_hz: 250,
            vibration: true,
            microphone: MicrophonePreference::Off,
        }
    }
}

/// 输入模式偏好只表达“想要什么”，最终模式由 compiler 定案。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum InputPreference {
    #[default]
    Auto,
    PhysicalGamepad,
    VirtualGamepad,
    NativeMkb,
}

/// 触控策略需要显式区分“跟随标题”和“强制给所有标题打开”。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum TouchPreference {
    #[default]
    FollowTitle,
    On,
    Off,
}

/// 麦克风策略与输入 plan 一起编译，避免 UI/runtime 各自解释。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum MicrophonePreference {
    #[default]
    Off,
    OnDemand,
    StartWithSession,
}

/// 统一标题输入能力枚举，便于 compiler 决定输入 mode。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SupportedInput {
    Gamepad,
    Mouse,
    Keyboard,
    NativeTouch,
    GenericTouch,
    CustomTouchOverlay,
    Mkb,
    Unknown(String),
}

impl Default for SupportedInput {
    fn default() -> Self {
        Self::Gamepad
    }
}

/// 标题能力来自 input config / title metadata，不由配置直接决定。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct TitleCapabilities {
    pub supported_inputs: Vec<SupportedInput>,
    pub has_mkb: bool,
    pub has_touch: bool,
    pub has_native_touch: bool,
}

/// capability 解释结果：统一收口 inputconfigs、配置阶段事实和本地兜底后的有效能力。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct EffectiveInputCapabilities {
    pub source: InputCapabilitySource,
    pub title_supports_mkb: bool,
    pub title_supports_touch: bool,
    pub title_supports_native_touch: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum InputCapabilitySource {
    #[default]
    Fallback,
    InputConfig,
}

/// 输入 plan 明确谁来接管输入，以及浏览器/sidecar 侧该开哪些能力。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct InputPlan {
    pub owner: Owner,
    pub mode: InputMode,
    pub polling_rate_hz: u16,
    pub vibration: bool,
    pub mouse: bool,
    pub keyboard: bool,
    pub touch: bool,
    pub max_touch_points: Option<u8>,
    pub microphone_on_play: bool,
}

/// 编译后的输入模式不再允许 Auto。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum InputMode {
    #[default]
    PhysicalGamepad,
    VirtualGamepad,
    NativeMkb,
}
pub mod compiler;
