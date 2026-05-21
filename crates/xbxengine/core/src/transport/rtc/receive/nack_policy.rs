//! 解码前 **sample-loss** 路径的 NACK admission 策略与 cloud RTT 窗辅助函数。
//!
//! RTP 序号 gap 不走本模块：由 [`super::nack_requester::NackRequester`] +
//! [`super::timing::ReceiveTimingProfile`] 统一调度，trace 见 [`super::trace_ledger`]。

use crate::media::video::ingress::budget::FrameBudgetContext;
use crate::media::video::types::FrameValue;
use crate::transport::rtc::recovery::timing::nack_retry_interval_u64_from_rtt_ms;
use crate::transport::rtc::stream::nack_contract::{NackObservePolicy, PacketRecoveryDisposition};

pub(super) const CLOUD_STARTUP_HEAD_HOLE_DEADLINE_FLOOR_MS: f64 = 320.0;
pub(super) const CLOUD_NACK_RTT_MARGIN_MS: f64 = 80.0;
pub(super) const CLOUD_STARTUP_NACK_RTT_MARGIN_MS: f64 = 140.0;
// 高 RTT 子画像：额外放宽 NACK 窗口，避免在长 RTT + 抖动下频繁 nackExpired。
// 从 100ms 开始分档：这能覆盖“中等 RTT 但抖动/重排明显”的场景。
pub(super) const CLOUD_HIGH_RTT_MS: f64 = 100.0;
pub(super) const CLOUD_SEVERE_RTT_MS: f64 = 200.0;
pub(super) const CLOUD_EXTREME_RTT_MS: f64 = 300.0;
pub(super) const CLOUD_HIGH_RTT_EXTRA_MARGIN_MS: f64 = 40.0;
pub(super) const CLOUD_SEVERE_RTT_EXTRA_MARGIN_MS: f64 = 80.0;
pub(super) const CLOUD_EXTREME_RTT_EXTRA_MARGIN_MS: f64 = 140.0;
pub(super) const NACK_MAINTENANCE_TICK_INTERVAL_MS: u64 = 10;
pub(super) const RECOVERY_KEYFRAME_RETRY_TIMEOUT_MS: f64 = 700.0;
pub(super) const RECOVERY_KEYFRAME_RETRY_INTERVAL_MS: f64 = 450.0;
pub(super) const RECOVERY_KEYFRAME_RETRY_MAX_COUNT: u16 = 8;
pub(super) const OOS_REPAIRABILITY_PENALTY: f64 = 0.08;

pub(super) fn cloud_nack_rtt_margin_ms(startup_mode: bool, cloud_rtt_ms: Option<f64>) -> f64 {
    let rtt_ms = cloud_rtt_ms.unwrap_or(0.0);
    let base = if startup_mode {
        CLOUD_STARTUP_NACK_RTT_MARGIN_MS
    } else {
        CLOUD_NACK_RTT_MARGIN_MS
    };
    let extra = if rtt_ms >= CLOUD_EXTREME_RTT_MS {
        CLOUD_EXTREME_RTT_EXTRA_MARGIN_MS
    } else if rtt_ms >= CLOUD_SEVERE_RTT_MS {
        CLOUD_SEVERE_RTT_EXTRA_MARGIN_MS
    } else if rtt_ms >= CLOUD_HIGH_RTT_MS {
        CLOUD_HIGH_RTT_EXTRA_MARGIN_MS
    } else {
        0.0
    };
    base + extra
}

pub(super) fn cloud_startup_head_hole_deadline_at_ms(
    now_ms: f64,
    deadline_at_ms: f64,
    cloud_mode: bool,
    startup_mode: bool,
    cloud_rtt_ms: Option<f64>,
    // RFC：与 `recovery_dynamic_nack_timeout_ms` 对齐；与 cloud RTT floor 取 max，避免短窗判死。
    dynamic_nack_timeout_ms: Option<f64>,
) -> f64 {
    if !cloud_mode {
        return deadline_at_ms;
    }
    let rtt_ms = cloud_rtt_ms.unwrap_or(0.0);
    let rtt_margin_ms = cloud_nack_rtt_margin_ms(startup_mode, cloud_rtt_ms);
    let mut deadline_floor_ms = now_ms
        + if startup_mode {
            (rtt_ms + rtt_margin_ms).max(CLOUD_STARTUP_HEAD_HOLE_DEADLINE_FLOOR_MS)
        } else {
            rtt_ms + rtt_margin_ms
        };
    if let Some(nack_to) = dynamic_nack_timeout_ms {
        deadline_floor_ms = deadline_floor_ms.max(now_ms + nack_to.max(0.0));
    }
    deadline_at_ms.max(deadline_floor_ms)
}

pub(super) fn cloud_nack_max_age_ms(
    base_max_age_ms: u64,
    cloud_mode: bool,
    startup_mode: bool,
    cloud_rtt_ms: Option<f64>,
) -> u64 {
    if !cloud_mode {
        return base_max_age_ms;
    }

    let rtt_ms = cloud_rtt_ms.unwrap_or(0.0);
    let rtt_margin_ms = cloud_nack_rtt_margin_ms(startup_mode, cloud_rtt_ms);
    base_max_age_ms.max((rtt_ms + rtt_margin_ms).round() as u64)
}

pub(super) fn sample_loss_nack_policy(
    sample_rtp_timestamp: u32,
    frame_is_keyframe: bool,
    budget_context: FrameBudgetContext,
    deadline_at_ms: f64,
    repairability: f64,
    cloud_mode: bool,
    startup_mode: bool,
    cloud_rtt_floor_ms: Option<f64>,
) -> NackObservePolicy {
    let frame_importance = if frame_is_keyframe {
        "anchor"
    } else {
        budget_context.recovery_value_tier()
    };
    let (base_max_age_ms, base_retry_interval_ms, base_burst_count, base_priority) =
        match (cloud_mode, startup_mode, frame_importance) {
            (true, true, "anchor") => (360.0, 40.0, 8.0, 3u8),
            (true, true, "supply") => (300.0, 34.0, 7.0, 2u8),
            (true, true, _) => (240.0, 28.0, 6.0, 1u8),
            (true, false, "anchor") => (240.0, 32.0, 6.0, 3u8),
            (true, false, "supply") => (180.0, 26.0, 5.0, 2u8),
            (true, false, _) => (120.0, 22.0, 4.0, 1u8),
            (false, _, "anchor") => (30.0, 10.0, 4.0, 3u8),
            (false, _, "supply") => (20.0, 8.0, 3.0, 2u8),
            (false, _, _) => (14.0, 6.0, 2.0, 1u8),
        };
    let max_age_ms = cloud_nack_max_age_ms(
        (base_max_age_ms * (0.85 + repairability * 0.45)).round() as u64,
        cloud_mode,
        startup_mode,
        cloud_rtt_floor_ms,
    );
    let mut retry_interval_ms = (base_retry_interval_ms * (1.25 - repairability * 0.45))
        .round()
        .max(4.0) as u64;
    if let Some(rtt) = cloud_rtt_floor_ms {
        retry_interval_ms = retry_interval_ms.max(nack_retry_interval_u64_from_rtt_ms(rtt));
    }
    let burst_count = (base_burst_count + (repairability * 1.8)).round().max(1.0) as u16;
    let priority = budget_context
        .repair_priority(frame_value_for_importance(frame_importance))
        .max(if repairability >= 0.86 {
            base_priority.saturating_add(1).min(4)
        } else {
            base_priority
        });
    NackObservePolicy {
        source: "sampleLoss",
        deadline_at_ms: Some(deadline_at_ms),
        max_age_ms: Some(max_age_ms),
        retry_interval_ms: Some(retry_interval_ms),
        burst_count: Some(burst_count),
        max_tracked_sequences: Some(match (cloud_mode, startup_mode, frame_importance) {
            (true, true, "anchor") => 24,
            (true, true, "supply") => 18,
            (true, true, _) => 14,
            (true, false, "anchor") => 18,
            (true, false, "supply") => 12,
            (true, false, _) => 8,
            (false, _, "anchor") => 12,
            (false, _, "supply") => 8,
            (false, _, _) => 4,
        }),
        frame_rtp_timestamp: Some(sample_rtp_timestamp),
        frame_is_keyframe: Some(frame_is_keyframe),
        frame_importance,
        priority,
        budget_context,
        estimated_recovery_arrival_ms: None,
        frame_playout_deadline_at_ms: Some(deadline_at_ms),
        nack_disposition: PacketRecoveryDisposition::Attempted,
        frame_unrecoverable_reason: None,
        max_retry_count_override: None,
        first_attempt_survival_window_ms: None,
        repairability_schedule: None,
        admission_deadline_floor_at_ms: None,
    }
}

pub(super) fn frame_value_for_importance(frame_importance: &'static str) -> FrameValue {
    match frame_importance {
        "anchor" => FrameValue::new(true, false, 128 * 1024),
        "supply" => FrameValue::new(false, true, 48 * 1024),
        _ => FrameValue::new(false, false, 12 * 1024),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloud_startup_head_hole_deadline_respects_dynamic_nack_floor() {
        let now_ms = 1_000.0;
        let base_deadline_at_ms = 1_120.0;
        // RTT 档与 margin 下 floor 为 1170；动态 NACK 超时 240ms 要求至少到 1240。
        let adjusted = cloud_startup_head_hole_deadline_at_ms(
            now_ms,
            base_deadline_at_ms,
            true,
            false,
            Some(90.0),
            Some(240.0),
        );
        assert_eq!(adjusted, 1_240.0);
    }

    #[test]
    fn cloud_nack_windows_follow_rtt_without_floor() {
        let now_ms = 1_000.0;
        let base_deadline_at_ms = 1_120.0;

        let adjusted_deadline = cloud_startup_head_hole_deadline_at_ms(
            now_ms,
            base_deadline_at_ms,
            true,
            false,
            Some(90.0),
            None,
        );
        let adjusted_max_age = cloud_nack_max_age_ms(100, true, false, Some(90.0));

        assert_eq!(adjusted_deadline, 1_170.0);
        assert_eq!(adjusted_max_age, 170);
    }

    #[test]
    fn non_cloud_nack_windows_remain_unchanged() {
        let now_ms = 1_000.0;
        let base_deadline_at_ms = 1_120.0;

        let adjusted_deadline = cloud_startup_head_hole_deadline_at_ms(
            now_ms,
            base_deadline_at_ms,
            false,
            false,
            Some(90.0),
            None,
        );
        let adjusted_max_age = cloud_nack_max_age_ms(180, false, false, Some(90.0));

        assert_eq!(adjusted_deadline, base_deadline_at_ms);
        assert_eq!(adjusted_max_age, 180);
    }
}
