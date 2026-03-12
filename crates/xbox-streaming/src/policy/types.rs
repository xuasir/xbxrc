use serde::{Deserialize, Serialize};
use thiserror::Error;

/// 编译失败只表达“无法产出稳定 plan”，不关心 UI 侧错误文案。
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum CompileError {
    #[error("streamingGsTokenMissing")]
    MissingGsToken,
    #[error("streamingRegionsMissing")]
    MissingRegions,
    #[error("streamingNoUsableRegion")]
    NoUsableRegion,
    #[error("streamingBaseUrlInvalid:{0}")]
    InvalidBaseUrl(String),
    #[error("streamingRuntimeUnavailable")]
    RuntimeUnavailable,
    #[error("streamingNativeMkbUnavailable")]
    NativeMkbUnavailable,
}

/// 目标类型是所有 plan 的一级分叉条件。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Target {
    Home,
    #[default]
    Cloud,
}

impl Target {
    pub fn is_home(self) -> bool {
        matches!(self, Self::Home)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Home => "home",
            Self::Cloud => "cloud",
        }
    }
}

/// owner 用于描述某段能力归浏览器还是 sidecar。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Owner {
    #[default]
    Browser,
    Sidecar,
}

/// region 描述来自 auth/login 响应，首期保留 better-xcloud 已使用的字段。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Region {
    pub name: String,
    pub base_uri: String,
    pub is_default: bool,
    pub short_name: Option<String>,
    pub display_name: Option<String>,
    pub continent: Option<String>,
}

/// 统一的 TURN server 结构，供 config/context/plan 复用。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct TurnServer {
    pub url: String,
    pub username: String,
    pub credential: String,
}

/// host candidate 注入使用的地址结构。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct HostAddr {
    pub ip: String,
    pub port: u16,
}

/// 通用开关：Auto 表示跟随标题能力和宿主能力。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum Switch {
    #[default]
    Auto,
    On,
    Off,
}

/// TURN 来源要显式落盘，避免 runtime 自己猜是哪个来源。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum TurnSource {
    #[default]
    None,
    Custom,
    Fallback,
}
