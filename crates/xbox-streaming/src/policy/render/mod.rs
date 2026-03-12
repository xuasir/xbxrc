use serde::{Deserialize, Serialize};

/// 渲染偏好：固定 renderer 初始化与首帧显示参数。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RenderConfig {
    /// 是否允许页面控制音量。
    pub enable_audio_control: bool,
    /// 视频显示格式，例如 `Contain` / `Stretch` / `Zoom` / `16:9`。
    pub video_format: Option<String>,
    /// 初始显示参数，运行期仍允许 UI 覆写。
    pub display_options: DisplayOptions,
}

impl Default for RenderConfig {
    fn default() -> Self {
        Self {
            enable_audio_control: false,
            video_format: None,
            display_options: DisplayOptions::default(),
        }
    }
}

/// 显示调节参数没有 Auto 语义，直接表达最终值。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DisplayOptions {
    pub sharpness: i16,
    pub saturation: i16,
    pub contrast: i16,
    pub brightness: i16,
}

impl Default for DisplayOptions {
    fn default() -> Self {
        Self {
            sharpness: 2,
            saturation: 100,
            contrast: 100,
            brightness: 100,
        }
    }
}

/// render plan 固定 renderer 初始化参数与首帧展示选项。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct RenderPlan {
    pub enable_audio_control: bool,
    pub video_format: Option<String>,
    pub display_options: RenderDisplayOptions,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RenderDisplayOptions {
    pub sharpness: i16,
    pub saturation: i16,
    pub contrast: i16,
    pub brightness: i16,
}

impl Default for RenderDisplayOptions {
    fn default() -> Self {
        Self {
            sharpness: 2,
            saturation: 100,
            contrast: 100,
            brightness: 100,
        }
    }
}
pub mod compiler;
