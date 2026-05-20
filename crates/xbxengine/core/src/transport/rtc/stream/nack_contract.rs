//! NACK 观测与批次类型（receiver-local 路径仍用于 RTCP 写出与统计）。

use crate::media::video::ingress::budget::FrameBudgetContext;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PacketRecoveryDisposition {
    Attempted,
    SkippedTooLate,
    SkippedLowValue,
    SkippedChainBroken,
}

impl PacketRecoveryDisposition {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Attempted => "attempted",
            Self::SkippedTooLate => "skippedTooLate",
            Self::SkippedLowValue => "skippedLowValue",
            Self::SkippedChainBroken => "skippedChainBroken",
        }
    }
}

#[derive(Clone, Debug)]
pub struct NackSchedulerConfig {
    pub max_age_ms: u64,
    pub frame_deadline_ms: u64,
    pub burst_count: u16,
    pub retry_interval_ms: u64,
    pub max_retry_count: u8,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NackBatch {
    pub sequences: Vec<u16>,
    pub retry_count: u8,
    pub source: &'static str,
    pub frame_rtp_timestamp: Option<u32>,
    pub frame_is_keyframe: Option<bool>,
    pub frame_importance: &'static str,
    pub deadline_at_ms: Option<f64>,
    pub estimated_recovery_arrival_ms: Option<f64>,
    pub frame_playout_deadline_at_ms: Option<f64>,
    pub nack_disposition: PacketRecoveryDisposition,
    pub frame_unrecoverable_reason: Option<&'static str>,
    pub budget_context: FrameBudgetContext,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedNack {
    pub sequence: u16,
    pub recovery_time_ms: f64,
    pub retry_count: u8,
    pub was_late: bool,
    pub source: &'static str,
    pub frame_rtp_timestamp: Option<u32>,
    pub frame_is_keyframe: Option<bool>,
    pub frame_importance: &'static str,
    pub deadline_at_ms: Option<f64>,
    pub estimated_recovery_arrival_ms: Option<f64>,
    pub frame_playout_deadline_at_ms: Option<f64>,
    pub nack_disposition: PacketRecoveryDisposition,
    pub frame_unrecoverable_reason: Option<&'static str>,
    pub budget_context: FrameBudgetContext,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SkippedNackBatch {
    pub sequences: Vec<u16>,
    pub source: &'static str,
    pub frame_rtp_timestamp: Option<u32>,
    pub frame_is_keyframe: Option<bool>,
    pub frame_importance: &'static str,
    pub deadline_at_ms: Option<f64>,
    pub estimated_recovery_arrival_ms: Option<f64>,
    pub frame_playout_deadline_at_ms: Option<f64>,
    pub nack_disposition: PacketRecoveryDisposition,
    pub frame_unrecoverable_reason: Option<&'static str>,
    pub budget_context: FrameBudgetContext,
}

#[derive(Clone, Copy, Debug)]
pub struct NackObservePolicy {
    pub source: &'static str,
    pub deadline_at_ms: Option<f64>,
    pub max_age_ms: Option<u64>,
    pub retry_interval_ms: Option<u64>,
    pub burst_count: Option<u16>,
    pub max_tracked_sequences: Option<u16>,
    pub frame_rtp_timestamp: Option<u32>,
    pub frame_is_keyframe: Option<bool>,
    pub frame_importance: &'static str,
    pub priority: u8,
    pub budget_context: FrameBudgetContext,
    pub estimated_recovery_arrival_ms: Option<f64>,
    pub frame_playout_deadline_at_ms: Option<f64>,
    pub nack_disposition: PacketRecoveryDisposition,
    pub frame_unrecoverable_reason: Option<&'static str>,
    pub first_attempt_survival_window_ms: Option<f64>,
    pub repairability_schedule: Option<f64>,
    pub admission_deadline_floor_at_ms: Option<f64>,
    pub max_retry_count_override: Option<u8>,
}
