//! 统一 host 刷新与视频流帧率为呈现节拍尺，供 pacer / decode 邮箱 / stats 共用。

use crate::XbxEngineMediaRuntimeStats;

pub const PRESENT_CADENCE_INTERVAL_FALLBACK_MS: f64 = 33.0;
/// Steady 供给健康但 present 明显落后 decode（与 display_supply / runtime_state 口径对齐）。
pub const PRESENT_PIPELINE_STRESSED_MIN_DECODE_FPS: f64 = 28.0;
pub const PRESENT_PIPELINE_STRESSED_MAX_PRESENT_FPS: f64 = 25.0;
pub const PRESENT_PIPELINE_STRESSED_MIN_FPS_GAP: f64 = 6.0;
pub const PRESENT_PIPELINE_STRESSED_EXIT_FPS_GAP: f64 = 3.0;
pub const PRESENT_PIPELINE_STRESSED_EXIT_AGE_MS: f64 = 80.0;
pub const PRESENT_PIPELINE_SEVERE_LOW_PRESENT_MIN_DECODE_FPS: f64 = 24.0;
pub const PRESENT_PIPELINE_SEVERE_LOW_PRESENT_MAX_PRESENT_FPS: f64 = 12.0;
pub const PRESENT_PIPELINE_SEVERE_LOW_PRESENT_MIN_FPS_GAP: f64 = 12.0;
pub const PRESENT_PIPELINE_LATENCY_STRESSED_MIN_DECODE_FPS: f64 = 18.0;
pub const PRESENT_PIPELINE_LATENCY_STRESSED_MAX_PRESENT_FPS: f64 = 24.0;
pub const PRESENT_PIPELINE_LATENCY_STRESSED_MIN_FPS_GAP: f64 = 3.0;
pub const PRESENT_PIPELINE_LATENCY_STRESSED_MIN_SUBMIT_AGE_MS: f64 = 120.0;
pub const PRESENT_PIPELINE_LATENCY_STRESSED_MIN_PRESENT_AGE_MS: f64 = 150.0;
pub const PRESENT_PIPELINE_HIGH_FPS_LATENCY_STRESSED_MIN_FPS: f64 = 45.0;
pub const PRESENT_PIPELINE_HIGH_FPS_LATENCY_STRESSED_MIN_SUBMIT_AGE_MS: f64 = 120.0;
pub const PRESENT_PIPELINE_HIGH_FPS_LATENCY_STRESSED_MIN_DISPLAY_AGE_MS: f64 = 64.0;
const HOST_MAILBOX_SERVICEABLE_MAX_DISPLAY_AGE_MS: f64 = 300.0;
const FRAME_INTERVAL_MIN_MS: f64 = 8.0;
const FRAME_INTERVAL_MAX_MS: f64 = 100.0;
const STRESSED_RELEASE_INTERVAL_MIN_MS: u64 = 8;

/// 由 inbound / decode 帧率推导的流帧间隔。
pub(crate) fn resolve_stream_frame_interval_ms(stats: &XbxEngineMediaRuntimeStats) -> Option<f64> {
    let video_fps = if stats.inbound_video_frame_rate_fps > 0.0 {
        stats.inbound_video_frame_rate_fps
    } else if stats.video_decode_fps > 0.0 {
        stats.video_decode_fps
    } else {
        return None;
    };
    let frame_interval_ms = 1_000.0 / video_fps;
    if (FRAME_INTERVAL_MIN_MS..=FRAME_INTERVAL_MAX_MS).contains(&frame_interval_ms) {
        Some(frame_interval_ms)
    } else {
        None
    }
}

pub(crate) fn resolve_host_display_interval_ms(stats: &XbxEngineMediaRuntimeStats) -> Option<f64> {
    stats
        .host_display_interval_ms
        .filter(|interval_ms| *interval_ms > 0.0)
}

/// 邮箱龄期 / anchor 保护窗：不慢于 host 与流任一侧，避免高刷屏下过密丢帧。
pub(crate) fn resolve_present_cadence_interval_ms(
    stats: &XbxEngineMediaRuntimeStats,
    fallback_ms: f64,
) -> f64 {
    match (
        resolve_host_display_interval_ms(stats),
        resolve_stream_frame_interval_ms(stats),
    ) {
        (Some(host_ms), Some(stream_ms)) => host_ms.max(stream_ms),
        (Some(host_ms), None) => host_ms,
        (None, Some(stream_ms)) => stream_ms,
        (None, None) => fallback_ms,
    }
    .clamp(FRAME_INTERVAL_MIN_MS, FRAME_INTERVAL_MAX_MS)
}

/// decode≈30 且 present 薄，或 host 稳态下 submit/present age 出现长尾：
/// 显示提交链已经落后，允许 pacer 按更快 host/流间隔释放。
pub(crate) fn present_pipeline_stressed_from_stats(stats: &XbxEngineMediaRuntimeStats) -> bool {
    let decode_fps = stats.video_decode_fps;
    let present_fps = stats.video_present_fps.max(0.0);
    let host_display_age_serviceable = host_display_age_serviceable(stats);
    let high_decode_present_gap = decode_fps >= PRESENT_PIPELINE_STRESSED_MIN_DECODE_FPS
        && present_fps > 0.0
        && present_fps <= PRESENT_PIPELINE_STRESSED_MAX_PRESENT_FPS
        && (decode_fps - present_fps) >= PRESENT_PIPELINE_STRESSED_MIN_FPS_GAP
        && host_display_age_serviceable;
    high_decode_present_gap
        || present_pipeline_severe_low_present_from_stats(stats)
        || present_pipeline_latency_stressed_from_stats(stats)
        || present_pipeline_high_fps_latency_stressed_from_stats(stats)
}

pub(crate) fn present_pipeline_stressed_with_hysteresis(
    stats: &XbxEngineMediaRuntimeStats,
    previously_stressed: bool,
) -> bool {
    if present_pipeline_stressed_from_stats(stats) {
        return true;
    }
    if !previously_stressed {
        return false;
    }
    present_pipeline_still_lagging_from_stats(stats)
}

fn host_mailbox_serviceable(stats: &XbxEngineMediaRuntimeStats) -> bool {
    stats.host_frame_present_epoch > 0
        && stats.host_cadence_phase.as_deref() == Some("steady")
        && stats.host_no_pending_streak == 0
        && host_display_age_serviceable(stats)
}

fn host_display_age_serviceable(stats: &XbxEngineMediaRuntimeStats) -> bool {
    stats
        .display_age_ms
        .map(|age_ms| age_ms < HOST_MAILBOX_SERVICEABLE_MAX_DISPLAY_AGE_MS)
        .unwrap_or(true)
}

fn present_pipeline_still_lagging_from_stats(stats: &XbxEngineMediaRuntimeStats) -> bool {
    if !host_mailbox_serviceable(stats) {
        return false;
    }
    let decode_fps = stats.video_decode_fps;
    let present_fps = stats.video_present_fps.max(0.0);
    let fps_gap = (decode_fps - present_fps) >= PRESENT_PIPELINE_STRESSED_EXIT_FPS_GAP;
    let age_late = stats
        .submit_age_ms
        .is_some_and(|age_ms| age_ms >= PRESENT_PIPELINE_STRESSED_EXIT_AGE_MS)
        || stats
            .display_age_ms
            .is_some_and(|age_ms| age_ms >= PRESENT_PIPELINE_STRESSED_EXIT_AGE_MS);
    fps_gap || age_late
}

fn present_pipeline_severe_low_present_from_stats(stats: &XbxEngineMediaRuntimeStats) -> bool {
    let decode_fps = stats.video_decode_fps;
    let present_fps = stats.video_present_fps.max(0.0);

    host_mailbox_serviceable(stats)
        && decode_fps >= PRESENT_PIPELINE_SEVERE_LOW_PRESENT_MIN_DECODE_FPS
        && present_fps <= PRESENT_PIPELINE_SEVERE_LOW_PRESENT_MAX_PRESENT_FPS
        && (decode_fps - present_fps) >= PRESENT_PIPELINE_SEVERE_LOW_PRESENT_MIN_FPS_GAP
}

fn present_pipeline_latency_stressed_from_stats(stats: &XbxEngineMediaRuntimeStats) -> bool {
    let decode_fps = stats.video_decode_fps;
    let present_fps = stats.video_present_fps.max(0.0);
    let submit_or_present_late = stats
        .submit_age_ms
        .is_some_and(|age_ms| age_ms >= PRESENT_PIPELINE_LATENCY_STRESSED_MIN_SUBMIT_AGE_MS)
        || stats
            .display_age_ms
            .is_some_and(|age_ms| age_ms >= PRESENT_PIPELINE_LATENCY_STRESSED_MIN_PRESENT_AGE_MS);

    host_mailbox_serviceable(stats)
        && submit_or_present_late
        && decode_fps >= PRESENT_PIPELINE_LATENCY_STRESSED_MIN_DECODE_FPS
        && present_fps <= PRESENT_PIPELINE_LATENCY_STRESSED_MAX_PRESENT_FPS
        && (decode_fps - present_fps) >= PRESENT_PIPELINE_LATENCY_STRESSED_MIN_FPS_GAP
}

fn present_pipeline_high_fps_latency_stressed_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
) -> bool {
    let decode_fps = stats.video_decode_fps;
    let present_fps = stats.video_present_fps;
    let submit_late = stats.submit_age_ms.is_some_and(|age_ms| {
        age_ms >= PRESENT_PIPELINE_HIGH_FPS_LATENCY_STRESSED_MIN_SUBMIT_AGE_MS
    });
    let display_late = stats.display_age_ms.is_some_and(|age_ms| {
        age_ms >= PRESENT_PIPELINE_HIGH_FPS_LATENCY_STRESSED_MIN_DISPLAY_AGE_MS
    });

    host_mailbox_serviceable(stats)
        && submit_late
        && display_late
        && decode_fps >= PRESENT_PIPELINE_HIGH_FPS_LATENCY_STRESSED_MIN_FPS
        && present_fps >= PRESENT_PIPELINE_HIGH_FPS_LATENCY_STRESSED_MIN_FPS
}

/// 供给压力下缩短 release 间隔，仍保持 latest-only（不深队列）。
pub(crate) fn resolve_stressed_release_interval_ms(
    stream_release_interval_ms: u64,
    host_refresh_interval_ms: u64,
) -> u64 {
    stream_release_interval_ms
        .min(host_refresh_interval_ms.max(STRESSED_RELEASE_INTERVAL_MIN_MS))
        .max(STRESSED_RELEASE_INTERVAL_MIN_MS)
}

/// Pacer 向 render 释放：流帧率优先，避免 144Hz 屏导致过度消费 decode 产出。
pub(crate) fn resolve_pacer_release_interval_ms(
    stats: &XbxEngineMediaRuntimeStats,
    fallback_ms: u64,
) -> u64 {
    resolve_stream_frame_interval_ms(stats)
        .map(|interval_ms| interval_ms.round() as u64)
        .filter(|interval_ms| {
            (FRAME_INTERVAL_MIN_MS as u64..=FRAME_INTERVAL_MAX_MS as u64).contains(&interval_ms)
        })
        .or_else(|| {
            resolve_host_display_interval_ms(stats)
                .map(|interval_ms| interval_ms.round() as u64)
                .filter(|interval_ms| *interval_ms > 0)
        })
        .unwrap_or(fallback_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn present_cadence_uses_slower_of_host_and_stream() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.host_display_interval_ms = Some(7.0);
        stats.inbound_video_frame_rate_fps = 30.0;
        let cadence =
            resolve_present_cadence_interval_ms(&stats, PRESENT_CADENCE_INTERVAL_FALLBACK_MS);
        assert!((cadence - 33.333).abs() < 0.5);
    }

    #[test]
    fn pacer_release_prefers_stream_over_fast_host() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.host_display_interval_ms = Some(7.0);
        stats.video_decode_fps = 30.0;
        assert_eq!(resolve_pacer_release_interval_ms(&stats, 16), 33);
    }

    #[test]
    fn pacer_release_falls_back_to_host_when_stream_unknown() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.host_display_interval_ms = Some(16.5);
        assert_eq!(resolve_pacer_release_interval_ms(&stats, 16), 17);
    }

    #[test]
    fn stressed_release_uses_faster_of_stream_and_host() {
        assert_eq!(resolve_stressed_release_interval_ms(33, 16), 16);
        assert_eq!(resolve_stressed_release_interval_ms(12, 16), 12);
    }

    #[test]
    fn present_pipeline_stressed_when_decode_high_and_present_lags() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.video_decode_fps = 30.0;
        stats.video_present_fps = 18.0;
        assert!(present_pipeline_stressed_from_stats(&stats));
        stats.video_present_fps = 24.0;
        assert!(present_pipeline_stressed_from_stats(&stats));
        stats.video_present_fps = 26.0;
        assert!(!present_pipeline_stressed_from_stats(&stats));
    }

    #[test]
    fn stale_host_display_age_blocks_decode_present_gap_stress() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.video_decode_fps = 31.0;
        stats.video_present_fps = 21.0;
        stats.display_age_ms = Some(360.0);
        stats.submit_age_ms = Some(360.0);
        stats.host_frame_present_epoch = 288;
        stats.host_cadence_phase = Some("starved".to_string());
        stats.host_no_pending_streak = 15;

        assert!(!present_pipeline_stressed_from_stats(&stats));
        assert!(!present_pipeline_stressed_with_hysteresis(&stats, true));
    }

    #[test]
    fn present_pipeline_stressed_when_submit_age_lags_in_steady_host_cadence() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.video_decode_fps = 22.8;
        stats.video_present_fps = 19.0;
        stats.submit_age_ms = Some(357.0);
        stats.display_age_ms = Some(181.0);
        stats.host_frame_present_epoch = 609;
        stats.host_mailbox_enqueue_count_total = 609;
        stats.host_cadence_phase = Some("steady".to_string());
        stats.host_no_pending_streak = 0;

        assert!(present_pipeline_stressed_from_stats(&stats));
    }

    #[test]
    fn present_pipeline_latency_stress_requires_decode_to_lead_present() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.video_decode_fps = 14.0;
        stats.video_present_fps = 20.3;
        stats.submit_age_ms = Some(353.0);
        stats.host_frame_present_epoch = 846;
        stats.host_mailbox_enqueue_count_total = 846;
        stats.host_cadence_phase = Some("steady".to_string());

        assert!(!present_pipeline_stressed_from_stats(&stats));
    }

    #[test]
    fn present_pipeline_stressed_when_present_fps_hits_zero_after_host_started() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.video_decode_fps = 29.0;
        stats.video_present_fps = 0.0;
        stats.submit_age_ms = Some(1_200.0);
        stats.host_frame_present_epoch = 1_700;
        stats.host_mailbox_enqueue_count_total = 1_700;
        stats.host_cadence_phase = Some("steady".to_string());
        stats.host_no_pending_streak = 0;

        assert!(present_pipeline_stressed_from_stats(&stats));
    }

    #[test]
    fn present_pipeline_zero_present_does_not_stress_before_host_started() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.video_decode_fps = 29.0;
        stats.video_present_fps = 0.0;
        stats.submit_age_ms = Some(1_200.0);
        stats.host_cadence_phase = Some("steady".to_string());

        assert!(!present_pipeline_stressed_from_stats(&stats));
    }

    #[test]
    fn present_pipeline_stressed_when_tail_present_fps_collapses_but_decode_continues() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.video_decode_fps = 26.6;
        stats.video_present_fps = 4.6;
        stats.host_frame_present_epoch = 3_802;
        stats.host_mailbox_enqueue_count_total = 3_804;
        stats.host_cadence_phase = Some("steady".to_string());
        stats.host_no_pending_streak = 0;

        assert!(present_pipeline_stressed_from_stats(&stats));
    }

    #[test]
    fn host_serviceable_allows_view_replay_present_epoch_ahead_of_enqueue_count() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.video_decode_fps = 26.6;
        stats.video_present_fps = 4.6;
        stats.host_frame_present_epoch = 101;
        stats.host_mailbox_enqueue_count_total = 100;
        stats.host_cadence_phase = Some("steady".to_string());
        stats.host_no_pending_streak = 0;

        assert!(present_pipeline_stressed_from_stats(&stats));
    }

    #[test]
    fn present_pipeline_severe_low_present_requires_host_serviceable() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.video_decode_fps = 26.6;
        stats.video_present_fps = 4.6;

        assert!(!present_pipeline_stressed_from_stats(&stats));
    }

    #[test]
    fn present_pipeline_stressed_when_high_fps_submit_latency_spikes() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.video_decode_fps = 52.2;
        stats.video_present_fps = 55.9;
        stats.submit_age_ms = Some(294.0);
        stats.display_age_ms = Some(82.0);
        stats.host_frame_present_epoch = 4_173;
        stats.host_mailbox_enqueue_count_total = 4_178;
        stats.host_cadence_phase = Some("steady".to_string());
        stats.host_no_pending_streak = 0;

        assert!(present_pipeline_stressed_from_stats(&stats));
    }

    #[test]
    fn present_pipeline_hysteresis_holds_until_gap_and_latency_recover() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.video_decode_fps = 29.0;
        stats.video_present_fps = 26.0;
        stats.host_frame_present_epoch = 100;
        stats.host_mailbox_enqueue_count_total = 100;
        stats.host_cadence_phase = Some("steady".to_string());
        stats.host_no_pending_streak = 0;

        assert!(!present_pipeline_stressed_from_stats(&stats));
        assert!(present_pipeline_stressed_with_hysteresis(&stats, true));

        stats.video_present_fps = 28.0;
        stats.submit_age_ms = Some(40.0);
        stats.display_age_ms = Some(50.0);
        assert!(!present_pipeline_stressed_with_hysteresis(&stats, true));
    }
}
