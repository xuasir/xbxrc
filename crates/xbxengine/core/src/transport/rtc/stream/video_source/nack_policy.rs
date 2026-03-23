use crate::media::video::types::FrameValue;
use crate::transport::rtc::stream::nack_scheduler::NackObservePolicy;

pub(super) const CLOUD_STARTUP_HEAD_HOLE_DEADLINE_FLOOR_MS: f64 = 320.0;
pub(super) const CLOUD_NACK_RTT_MARGIN_MS: f64 = 80.0;
pub(super) const CLOUD_STARTUP_NACK_RTT_MARGIN_MS: f64 = 140.0;

pub(super) fn cloud_startup_head_hole_deadline_at_ms(
    now_ms: f64,
    deadline_at_ms: f64,
    cloud_mode: bool,
    startup_mode: bool,
    cloud_rtt_ms: Option<f64>,
) -> f64 {
    if !cloud_mode {
        return deadline_at_ms;
    }
    let rtt_ms = cloud_rtt_ms.unwrap_or(0.0);
    let deadline_floor_ms = now_ms
        + if startup_mode {
            (rtt_ms + CLOUD_STARTUP_NACK_RTT_MARGIN_MS)
                .max(CLOUD_STARTUP_HEAD_HOLE_DEADLINE_FLOOR_MS)
        } else {
            rtt_ms + CLOUD_NACK_RTT_MARGIN_MS
        };
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
    let rtt_margin_ms = if startup_mode {
        CLOUD_STARTUP_NACK_RTT_MARGIN_MS
    } else {
        CLOUD_NACK_RTT_MARGIN_MS
    };
    base_max_age_ms.max((rtt_ms + rtt_margin_ms).round() as u64)
}

pub(super) fn sample_loss_nack_policy(
    sample_rtp_timestamp: u32,
    frame_is_keyframe: bool,
    frame_importance: &'static str,
    deadline_at_ms: f64,
    repairability: f64,
    cloud_mode: bool,
    startup_mode: bool,
    cloud_rtt_floor_ms: Option<f64>,
) -> NackObservePolicy {
    let (base_max_age_ms, base_retry_interval_ms, base_burst_count, base_priority) =
        match (cloud_mode, startup_mode, frame_importance) {
            (true, true, "keyframe") => (360.0, 40.0, 8.0, 3u8),
            (true, true, "reference") => (300.0, 34.0, 7.0, 2u8),
            (true, true, _) => (240.0, 28.0, 6.0, 1u8),
            (true, false, "keyframe") => (240.0, 32.0, 6.0, 3u8),
            (true, false, "reference") => (180.0, 26.0, 5.0, 2u8),
            (true, false, _) => (120.0, 22.0, 4.0, 1u8),
            (false, _, "keyframe") => (30.0, 10.0, 4.0, 3u8),
            (false, _, "reference") => (20.0, 8.0, 3.0, 2u8),
            (false, _, _) => (14.0, 6.0, 2.0, 1u8),
        };
    let max_age_ms = cloud_nack_max_age_ms(
        (base_max_age_ms * (0.85 + repairability * 0.45)).round() as u64,
        cloud_mode,
        startup_mode,
        cloud_rtt_floor_ms,
    );
    let retry_interval_ms = (base_retry_interval_ms * (1.25 - repairability * 0.45))
        .round()
        .max(4.0) as u64;
    let burst_count = (base_burst_count + (repairability * 1.8)).round().max(1.0) as u16;
    let priority = if repairability >= 0.86 {
        base_priority.saturating_add(1).min(4)
    } else {
        base_priority
    };
    NackObservePolicy {
        source: "sampleLoss",
        deadline_at_ms: Some(deadline_at_ms),
        max_age_ms: Some(max_age_ms),
        retry_interval_ms: Some(retry_interval_ms),
        burst_count: Some(burst_count),
        max_tracked_sequences: Some(match (cloud_mode, startup_mode, frame_importance) {
            (true, true, "keyframe") => 24,
            (true, true, "reference") => 18,
            (true, true, _) => 14,
            (true, false, "keyframe") => 18,
            (true, false, "reference") => 12,
            (true, false, _) => 8,
            (false, _, "keyframe") => 12,
            (false, _, "reference") => 8,
            (false, _, _) => 4,
        }),
        frame_rtp_timestamp: Some(sample_rtp_timestamp),
        frame_is_keyframe: Some(frame_is_keyframe),
        frame_importance,
        priority,
    }
}

pub(super) fn rtp_window_nack_policy(
    frame_value: FrameValue,
    deadline_at_ms: f64,
    cloud_mode: bool,
    startup_mode: bool,
    cloud_rtt_floor_ms: Option<f64>,
) -> NackObservePolicy {
    let (frame_importance, frame_is_keyframe, retry_interval_ms, burst_count, priority) =
        transport_policy_tuple(frame_value, cloud_mode, startup_mode);
    NackObservePolicy {
        source: "rtpWindow",
        deadline_at_ms: Some(deadline_at_ms),
        max_age_ms: Some(cloud_nack_max_age_ms(
            match (cloud_mode, startup_mode) {
                (true, true) => 300,
                (true, false) => 180,
                (false, _) => 26,
            },
            cloud_mode,
            startup_mode,
            cloud_rtt_floor_ms,
        )),
        retry_interval_ms: Some(retry_interval_ms),
        burst_count: Some(burst_count),
        max_tracked_sequences: Some(match (cloud_mode, startup_mode, frame_importance) {
            (true, true, "keyframe") => 20,
            (true, true, "reference") => 14,
            (true, true, _) => 10,
            (true, false, "keyframe") => 14,
            (true, false, "reference") => 10,
            (true, false, _) => 6,
            (false, _, "keyframe") => 10,
            (false, _, "reference") => 6,
            (false, _, _) => 4,
        }),
        frame_rtp_timestamp: None,
        frame_is_keyframe: Some(frame_is_keyframe),
        frame_importance,
        priority,
    }
}

pub(super) fn rtp_gap_nack_policy(
    frame_value: FrameValue,
    deadline_at_ms: f64,
    cloud_mode: bool,
    startup_mode: bool,
    cloud_rtt_floor_ms: Option<f64>,
) -> NackObservePolicy {
    let (frame_importance, frame_is_keyframe, retry_interval_ms, burst_count, priority) =
        transport_policy_tuple(frame_value, cloud_mode, startup_mode);
    NackObservePolicy {
        source: "rtpGap",
        deadline_at_ms: Some(deadline_at_ms),
        max_age_ms: Some(cloud_nack_max_age_ms(
            match (cloud_mode, startup_mode) {
                (true, true) => 260,
                (true, false) => 160,
                (false, _) => 22,
            },
            cloud_mode,
            startup_mode,
            cloud_rtt_floor_ms,
        )),
        retry_interval_ms: Some(if cloud_mode {
            retry_interval_ms
        } else {
            retry_interval_ms.saturating_sub(1).max(4)
        }),
        burst_count: Some(burst_count.saturating_add(1)),
        max_tracked_sequences: Some(match (cloud_mode, startup_mode, frame_importance) {
            (true, true, "keyframe") => 22,
            (true, true, "reference") => 16,
            (true, true, _) => 12,
            (true, false, "keyframe") => 16,
            (true, false, "reference") => 12,
            (true, false, _) => 8,
            (false, _, "keyframe") => 12,
            (false, _, "reference") => 8,
            (false, _, _) => 4,
        }),
        frame_rtp_timestamp: None,
        frame_is_keyframe: Some(frame_is_keyframe),
        frame_importance,
        priority,
    }
}

fn transport_policy_tuple(
    frame_value: FrameValue,
    cloud_mode: bool,
    startup_mode: bool,
) -> (&'static str, bool, u64, u16, u8) {
    if frame_value.is_sync_point() {
        if cloud_mode && startup_mode {
            ("keyframe", true, 30, 8, 3)
        } else if cloud_mode {
            ("keyframe", true, 24, 6, 3)
        } else {
            ("keyframe", true, 8, 4, 3)
        }
    } else if frame_value.refresh_boost {
        if cloud_mode && startup_mode {
            ("reference", false, 26, 7, 2)
        } else if cloud_mode {
            ("reference", false, 20, 5, 2)
        } else {
            ("reference", false, 7, 3, 2)
        }
    } else if cloud_mode && startup_mode {
        ("delta", false, 22, 6, 1)
    } else if cloud_mode {
        ("delta", false, 16, 4, 1)
    } else {
        ("delta", false, 6, 2, 1)
    }
}

pub(super) fn frame_value_for_importance(frame_importance: &'static str) -> FrameValue {
    match frame_importance {
        "keyframe" => FrameValue::new(true, false, 128 * 1024),
        "reference" => FrameValue::new(false, true, 48 * 1024),
        _ => FrameValue::new(false, false, 12 * 1024),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        );
        let adjusted_max_age = cloud_nack_max_age_ms(180, false, false, Some(90.0));

        assert_eq!(adjusted_deadline, base_deadline_at_ms);
        assert_eq!(adjusted_max_age, 180);
    }
}
