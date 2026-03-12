use serde::{Deserialize, Serialize};

use crate::policy::types::HostAddr;

/// 协商侧偏好：编译后会落成 SDP/ICE/runtime negotiation plan。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NegotiationConfig {
    /// Cloud Gaming 是否优先 IPv6 candidate。
    pub cloud_prefer_ipv6: bool,
    /// Remote Play 是否优先 IPv6 candidate。
    pub home_prefer_ipv6: bool,
    /// 视频 codec 偏好。
    pub video_codec: CodecPreference,
    /// Cloud Gaming 视频码率偏好。
    pub cloud_video_bitrate: BitratePreference,
    /// Remote Play 视频码率偏好。
    pub home_video_bitrate: BitratePreference,
    /// 音频码率偏好。
    pub audio_bitrate: BitratePreference,
    /// 音频声道偏好，用于是否启用 stereo。
    pub audio_channels: AudioChannels,
    /// 手工覆盖 offer profile，未设置时由 compiler 按 runtime 推导。
    pub offer_profile: Option<String>,
}

impl Default for NegotiationConfig {
    fn default() -> Self {
        Self {
            cloud_prefer_ipv6: false,
            home_prefer_ipv6: false,
            video_codec: CodecPreference::Auto,
            cloud_video_bitrate: BitratePreference::Auto,
            home_video_bitrate: BitratePreference::Auto,
            audio_bitrate: BitratePreference::Auto,
            audio_channels: AudioChannels::Auto,
            offer_profile: None,
        }
    }
}

/// 视频 codec 偏好，首期覆盖 H264 档位与显式 mimeType。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CodecPreference {
    #[default]
    Auto,
    H264Low,
    H264Normal,
    H264High,
    MimeType {
        mime_type: String,
    },
}

/// 码率策略沿用 Auto/Custom 两态，便于兼容现有设置面板。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum BitratePreference {
    #[default]
    Auto,
    CustomKbps {
        kbps: u32,
    },
}

/// 音频声道偏好最终会编译成是否启用 stereo。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum AudioChannels {
    #[default]
    Auto,
    Mono,
    Stereo,
}

/// 编译后的 codec 选择结果，适合直接映射到 SDP policy。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Codec {
    pub mime_type: String,
    pub profiles: Vec<String>,
}

/// 协商 plan 固定 SDP/ICE/runtime negotiation 需要的输入。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct NegotiationPlan {
    pub prefer_ipv6: bool,
    pub codec: Option<Codec>,
    pub video_bitrate_kbps: Option<u32>,
    pub audio_bitrate_kbps: Option<u32>,
    pub stereo_audio: bool,
    pub offer_profile: String,
    pub normalize_end_of_candidates: bool,
    pub inject_console_addrs: bool,
    pub console_addrs: Vec<HostAddr>,
}
pub mod compiler;
