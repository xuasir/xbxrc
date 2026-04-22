use bytes::Bytes;
use std::time::Instant;

use super::h264::inspection::H264AccessUnitInspection;
use crate::media::video::ingress::budget::FrameBudgetContext;

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
pub struct AssembledVideoFrame {
    pub codec: VideoCodec,

    pub is_keyframe: bool,
    pub config_changed: bool,
    pub value: FrameValue,
    pub(crate) budget: FrameBudgetContext,

    pub width: u32,
    pub height: u32,

    pub rtp_timestamp: u32,
    pub first_packet_sequence: Option<u16>,
    pub frame_playout_deadline_at_ms: Option<f64>,
    pub frame_recovery_disposition: FrameRecoveryDisposition,
    pub frame_unrecoverable_reason: Option<String>,

    pub assembled_at: Instant,
    /// 物化阶段计算 target playout 的时间基准，优先取首包到达时刻。
    pub first_packet_arrived_at: Option<Instant>,

    pub h264: H264AccessUnitInspection,
    pub payload: Bytes,
}

impl AssembledVideoFrame {
    pub fn into_encoded_frame(self, target_playout_instant: Instant) -> EncodedFrame {
        EncodedFrame {
            codec: self.codec,
            is_keyframe: self.is_keyframe,
            config_changed: self.config_changed,
            value: self.value,
            budget: self.budget,
            width: self.width,
            height: self.height,
            rtp_timestamp: self.rtp_timestamp,
            first_packet_sequence: self.first_packet_sequence,
            frame_playout_deadline_at_ms: self.frame_playout_deadline_at_ms,
            frame_recovery_disposition: self.frame_recovery_disposition,
            frame_unrecoverable_reason: self.frame_unrecoverable_reason,
            target_playout_instant,
            h264: self.h264,
            payload: self.payload,
        }
    }
}

#[derive(Clone, Debug)]
pub struct EncodedFrame {
    pub codec: VideoCodec,

    pub is_keyframe: bool,
    pub config_changed: bool,
    pub value: FrameValue,
    pub(crate) budget: FrameBudgetContext,

    pub width: u32,
    pub height: u32,

    pub rtp_timestamp: u32,
    pub first_packet_sequence: Option<u16>,
    pub frame_playout_deadline_at_ms: Option<f64>,
    pub frame_recovery_disposition: FrameRecoveryDisposition,
    pub frame_unrecoverable_reason: Option<String>,

    pub target_playout_instant: Instant,

    pub h264: H264AccessUnitInspection,
    pub payload: Bytes,
}

#[derive(Clone, Debug)]
pub struct DecodedFrame {
    pub pts: Instant,
    pub rtp_timestamp: u32,
    pub is_keyframe: bool,
    pub(crate) budget: FrameBudgetContext,
    pub frame_recovery_disposition: FrameRecoveryDisposition,
    pub frame_unrecoverable_reason: Option<String>,

    pub surface: crate::media::video::render::renderer::XbxRenderFrame,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameRecoveryDisposition {
    Repairing,
    UnrecoverableLate,
    UnrecoverableReferenceChain,
}

impl FrameRecoveryDisposition {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Repairing => "repairing",
            Self::UnrecoverableLate => "abandonedLate",
            Self::UnrecoverableReferenceChain => "abandonedReferenceChain",
        }
    }

    pub fn ingress_reason(self) -> Option<&'static str> {
        match self {
            Self::Repairing => None,
            Self::UnrecoverableLate => Some("late"),
            Self::UnrecoverableReferenceChain => Some("referenceChain"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{FrameDependency, FrameValue};

    #[test]
    fn frame_value_keeps_intrinsic_priority_semantics() {
        let keyframe = FrameValue::new(true, false, 8_192);
        let refresh_predicted = FrameValue::new(false, true, 1_024);
        let plain_predicted = FrameValue::new(false, false, 1_024);

        assert_eq!(keyframe.dependency, FrameDependency::Independent);
        assert_eq!(refresh_predicted.dependency, FrameDependency::Predicted);
        assert!(keyframe.is_sync_point());
        assert!(!refresh_predicted.is_sync_point());

        assert!(keyframe.backlog_priority_score() > refresh_predicted.backlog_priority_score());
        assert!(
            refresh_predicted.backlog_priority_score() > plain_predicted.backlog_priority_score()
        );
        assert_eq!(keyframe.late_budget_ratio_per_mille(), 1_000);
        assert_eq!(refresh_predicted.late_budget_ratio_per_mille(), 800);
        assert_eq!(plain_predicted.late_budget_ratio_per_mille(), 500);
        assert_eq!(keyframe.deadline_budget_ratio_per_mille(), 1_000);
        assert_eq!(refresh_predicted.deadline_budget_ratio_per_mille(), 700);
        assert_eq!(plain_predicted.deadline_budget_ratio_per_mille(), 450);
    }
}
