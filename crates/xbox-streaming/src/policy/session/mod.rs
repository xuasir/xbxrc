use std::collections::BTreeMap;
use serde::{Deserialize, Serialize};

use crate::policy::types::{Region, Target};

/// 会话侧偏好：区服、语言、分辨率画像都在这里。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionConfig {
    /// 游戏会话的 locale 偏好，对应 play payload 里的 `settings.locale`。
    pub preferred_game_locale: String,
    /// 期望的逻辑区服名，例如 `WESTEUROPE`。
    pub preferred_region_name: Option<String>,
    /// 通过请求头伪装客户端来源 IP，用于区域绕过场景。
    pub client_ip_override: Option<String>,
    /// 通过服务端 region/baseUri 中的 IP 片段强制命中目标区域。
    pub force_region_ip: Option<String>,
    /// Cloud Gaming 的目标分辨率偏好。
    pub cloud_resolution: ResolutionPreference,
    /// Remote Play 的目标分辨率偏好。
    pub home_resolution: ResolutionPreference,
    /// 直接指定设备画像，优先级高于分辨率派生。
    pub device_profile: Option<DeviceProfileKind>,
    /// xHome 开流前是否允许发送唤醒请求。
    pub power_on: bool,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            preferred_game_locale: "en-US".to_string(),
            preferred_region_name: None,
            client_ip_override: None,
            force_region_ip: None,
            cloud_resolution: ResolutionPreference::Auto,
            home_resolution: ResolutionPreference::Auto,
            device_profile: None,
            power_on: false,
        }
    }
}

/// 分辨率偏好不会直接传给 runtime，而是先编译成设备画像。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ResolutionPreference {
    #[default]
    Auto,
    P720,
    P1080,
    P1080Hq,
}

/// 设备画像是从分辨率或 override 编译出来的具体结果。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DeviceProfile {
    pub kind: DeviceProfileKind,
    pub os_name: String,
    pub max_width: u32,
    pub max_height: u32,
}

impl Default for DeviceProfile {
    fn default() -> Self {
        Self {
            kind: DeviceProfileKind::Android,
            os_name: "android".to_string(),
            max_width: 1280,
            max_height: 720,
        }
    }
}

/// 设备画像类型与 better-xcloud 的 `android/windows/tizen` 经验对齐。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum DeviceProfileKind {
    #[default]
    Android,
    Windows,
    Tizen,
    Custom {
        name: String,
    },
}

/// 会话入口上下文：编译区服/base_url 时需要读取这里。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct SessionAccessContext {
    pub gs_token: Option<String>,
    pub regions: Vec<Region>,
}

/// 这是“会话接入结果”，不是 plan 本身，但属于 streaming 领域稳定输出。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedSessionAccess {
    pub gs_token: String,
    pub base_url: String,
    pub region: Option<Region>,
}

/// 会话 plan 负责固定入口、区域、locale 和设备画像。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionPlan {
    pub target: Target,
    pub target_id: String,
    pub base_url: String,
    pub region: Option<Region>,
    pub locale: String,
    pub device: DeviceProfile,
    /// 会话调度策略（轮询/保活/超时/重试）由 policy 统一给出。
    pub schedule: SessionSchedulePlan,
    /// 串流启动时的 Payload 策略设置。
    pub settings: PlaySettings,
    /// 供主进程组装 `x-ms-device-info` 请求头使用。
    pub ms_device_info: MsDeviceInfo,
    /// 已编译好的请求头补丁，adapter 不应再自行拼装业务头。
    pub headers: BTreeMap<String, String>,
}

impl Default for SessionPlan {
    fn default() -> Self {
        Self {
            target: Target::default(),
            target_id: String::new(),
            base_url: String::new(),
            region: None,
            locale: "en-US".to_string(),
            device: DeviceProfile::default(),
            schedule: SessionSchedulePlan::default(),
            settings: PlaySettings::default(),
            ms_device_info: MsDeviceInfo::default(),
            headers: BTreeMap::new(),
        }
    }
}

/// session 调度 plan：所有节奏常量必须来自策略层，避免 UI/adapter 分散维护。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionSchedulePlan {
    pub monitor_interval_ms: u64,
    pub keepalive_interval_ms: u64,
    pub offer_poll_interval_ms: u64,
    pub ice_poll_interval_ms: u64,
    pub startup_timeout_ms: u64,
    pub ready_timeout_ms: u64,
    pub retry_backoff_ms: Vec<u64>,
    pub wake_console: bool,
    pub require_console_ready: bool,
}

impl Default for SessionSchedulePlan {
    fn default() -> Self {
        Self {
            monitor_interval_ms: 1_000,
            keepalive_interval_ms: 30_000,
            offer_poll_interval_ms: 1_000,
            ice_poll_interval_ms: 1_000,
            startup_timeout_ms: 90_000,
            ready_timeout_ms: 90_000,
            retry_backoff_ms: vec![1_000, 3_000, 5_000],
            wake_console: false,
            require_console_ready: false,
        }
    }
}

/// 对应 play 接口 settings 字段的决策结果。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PlaySettings {
    pub nano_version: String,
    pub enable_text_to_speech: bool,
    pub high_contrast: u8,
    pub locale: String,
    pub use_ice_connection: bool,
    pub timezone_offset_minutes: i32,
    pub sdk_type: String,
    pub os_name: String,
}

impl Default for PlaySettings {
    fn default() -> Self {
        Self {
            nano_version: "V3;WebrtcTransport.dll".to_string(),
            enable_text_to_speech: false,
            high_contrast: 0,
            locale: "en-US".to_string(),
            use_ice_connection: false,
            timezone_offset_minutes: 120,
            sdk_type: "web".to_string(),
            os_name: "windows".to_string(),
        }
    }
}

/// 统一后的 `x-ms-device-info` 结构，避免继续把业务字段塞进裸 JSON。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct MsDeviceInfo {
    pub client_app_type: String,
    pub device_make: String,
    pub device_model: String,
    pub sdk_type: String,
    pub os_name: String,
    pub os_version: Option<String>,
    pub browser_name: String,
    pub browser_version: Option<String>,
    pub display_width_pixels: u32,
    pub display_height_pixels: u32,
    pub dpi_x: u32,
    pub dpi_y: u32,
}
pub mod compiler;
