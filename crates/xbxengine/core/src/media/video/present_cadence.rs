//! 统一 host 刷新与视频流帧率为呈现节拍尺，供 pacer / decode 邮箱 / stats 共用。

use crate::XbxEngineMediaRuntimeStats;

pub const PRESENT_CADENCE_INTERVAL_FALLBACK_MS: f64 = 33.0;
/// Steady 供给健康但 present 明显落后 decode（与 display_supply / runtime_state 口径对齐）。
pub const PRESENT_PIPELINE_STRESSED_MIN_DECODE_FPS: f64 = 28.0;
pub const PRESENT_PIPELINE_STRESSED_MAX_PRESENT_FPS: f64 = 22.0;
pub const PRESENT_PIPELINE_STRESSED_MIN_FPS_GAP: f64 = 6.0;
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

/// decode≈30 且 present 薄：显示链为瓶颈，允许 pacer 按更快 host/流间隔释放。
pub(crate) fn present_pipeline_stressed_from_stats(stats: &XbxEngineMediaRuntimeStats) -> bool {
    let decode_fps = stats.video_decode_fps;
    let present_fps = stats.video_present_fps;
    decode_fps >= PRESENT_PIPELINE_STRESSED_MIN_DECODE_FPS
        && present_fps > 0.0
        && present_fps <= PRESENT_PIPELINE_STRESSED_MAX_PRESENT_FPS
        && (decode_fps - present_fps) >= PRESENT_PIPELINE_STRESSED_MIN_FPS_GAP
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
        stats.video_present_fps = 26.0;
        assert!(!present_pipeline_stressed_from_stats(&stats));
    }
}
