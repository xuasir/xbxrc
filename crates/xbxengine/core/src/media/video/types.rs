use bytes::Bytes;
use std::time::Instant;

use super::h264::inspection::H264AccessUnitInspection;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VideoCodec {
    H264,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameDependency {
    Independent,
    Predicted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameValue {
    pub dependency: FrameDependency,
    pub refresh_boost: bool,
    pub payload_size_bytes: usize,
}

impl FrameValue {
    pub fn new(is_keyframe: bool, refresh_boost: bool, payload_size_bytes: usize) -> Self {
        Self {
            dependency: if is_keyframe {
                FrameDependency::Independent
            } else {
                FrameDependency::Predicted
            },
            refresh_boost,
            payload_size_bytes,
        }
    }

    pub fn is_sync_point(&self) -> bool {
        matches!(self.dependency, FrameDependency::Independent)
    }

    /// 越大表示越值得在 backlog 下保留。
    pub fn backlog_priority_score(&self) -> u32 {
        let dependency_score = match self.dependency {
            FrameDependency::Independent => 1_000,
            FrameDependency::Predicted => 300,
        };
        let refresh_score = if self.refresh_boost { 250 } else { 0 };
        let size_bonus = (256usize.saturating_sub(self.payload_size_bytes / 512)) as u32;
        dependency_score + refresh_score + size_bonus
    }

    /// 返回相对基础晚到窗口的保留比例，单位千分比。
    pub fn late_budget_ratio_per_mille(&self) -> u16 {
        match (self.dependency, self.refresh_boost) {
            (FrameDependency::Independent, _) => 1_000,
            (FrameDependency::Predicted, true) => 800,
            (FrameDependency::Predicted, false) => 500,
        }
    }

    /// 返回相对基础 deadline 的比例，单位千分比。
    pub fn deadline_budget_ratio_per_mille(&self) -> u16 {
        match (self.dependency, self.refresh_boost) {
            (FrameDependency::Independent, _) => 1_000,
            (FrameDependency::Predicted, true) => 700,
            (FrameDependency::Predicted, false) => 450,
        }
    }
}

#[derive(Clone, Debug)]
pub struct EncodedFrame {
    pub codec: VideoCodec,

    pub is_keyframe: bool,
    pub config_changed: bool,
    pub value: FrameValue,

    pub width: u32,
    pub height: u32,

    pub rtp_timestamp: u32,

    pub assembled_at: Instant,
    pub target_playout_time: Instant,

    pub h264: H264AccessUnitInspection,
    pub payload: Bytes,
}

pub trait FrameSurface: Send + Sync {}

pub struct DecodedFrame {
    pub width: u32,
    pub height: u32,

    pub pts: Instant,

    pub surface: crate::media::video::render::renderer::XbxRenderFrame,
}
