use serde::{Deserialize, Serialize};

use crate::policy::types::{Owner, TurnServer, TurnSource};

/// runtime 偏好：只描述“想走哪条路”，最终 owner 由 plan 固定。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeConfig {
    pub mode: RuntimePreference,
    /// 自定义 TURN 配置，优先级高于 fallback TURN。
    pub custom_turn: Option<TurnServer>,
    /// xHome 在没有自定义 TURN 时，是否允许走 fallback TURN。
    pub home_fallback_turn: bool,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            mode: RuntimePreference::Auto,
            custom_turn: None,
            home_fallback_turn: false,
        }
    }
}

/// runtime 偏好，Auto 表示由 compiler 根据上下文判定。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum RuntimePreference {
    #[default]
    Auto,
    WebRtcDirect,
    RustOwned,
}

/// runtime 宿主能力决定 plan 最终能不能落到某条执行路径。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeCapabilities {
    pub browser_webrtc: bool,
    pub rust_owned: bool,
    pub native_mkb: bool,
    pub touch_surface: bool,
    /// 当前宿主是否倾向浏览器侧接管 transport/decode/render。
    pub prefer_browser: bool,
}

impl Default for RuntimeCapabilities {
    fn default() -> Self {
        Self {
            browser_webrtc: true,
            rust_owned: false,
            native_mkb: false,
            touch_surface: false,
            prefer_browser: true,
        }
    }
}

/// TURN 上下文由主进程在建 runtime 前解析出来。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct TurnContext {
    pub fallback: Option<TurnServer>,
}

/// compiler 完成后，可把 runtime 侧最关心的执行能力投影出来。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeProjection {
    pub mode: RuntimeMode,
    pub transport: Owner,
    pub decode: Owner,
    pub render: Owner,
    pub input: Owner,
}

/// runtime plan 固定整条执行链的 owner 与 TURN 选择结果。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct RuntimePlan {
    pub mode: RuntimeMode,
    pub transport: Owner,
    pub decode: Owner,
    pub render: Owner,
    pub input: Owner,
    pub microphone: Owner,
    pub turn: TurnPlan,
}

/// runtime 最终模式不再允许 Auto。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum RuntimeMode {
    #[default]
    WebRtcDirect,
    RustOwned,
}

/// TURN plan 记录 custom/fallback/resolved 三层结果，便于调试和回放。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct TurnPlan {
    pub custom: Option<TurnServer>,
    pub fallback: Option<TurnServer>,
    pub resolved: Option<TurnServer>,
    pub source: TurnSource,
}
pub mod compiler;
