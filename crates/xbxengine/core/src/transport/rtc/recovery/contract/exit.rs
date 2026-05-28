use super::decode_sync::fresh_h264_idr_admission_from_stats;
use super::display::RecoveryDisplayFacts;
use crate::XbxEngineMediaRuntimeStats;

const RECOVERY_EXIT_TIMED_FALLBACK_SUBMIT_AGE_MS: f64 = 1_500.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum RecoveryExitPath {
    HostIdr,
    DecodeOutput,
    TimedFallback,
    #[default]
    AwaitingAnchor,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct RecoveryExitThresholds {
    pub(crate) degraded_decode_age_ms: f64,
    pub(crate) timed_fallback_submit_age_ms: f64,
}

impl Default for RecoveryExitThresholds {
    fn default() -> Self {
        Self {
            degraded_decode_age_ms: 1_200.0,
            timed_fallback_submit_age_ms: RECOVERY_EXIT_TIMED_FALLBACK_SUBMIT_AGE_MS,
        }
    }
}

/// 恢复退出用的 host IDR 证据：不接受「历史上屏过」的 stale displayed-idr 单独挡 TimedFallback。
fn recovery_exit_host_idr_path_active(stats: &XbxEngineMediaRuntimeStats, now_ms: f64) -> bool {
    if fresh_h264_idr_admission_from_stats(stats, now_ms) {
        return true;
    }
    let display = RecoveryDisplayFacts::from_stats(stats);
    if display.fresh_anchor_recovered_at_ms.is_some() {
        return true;
    }
    stats.video_anchor_clean_epoch == Some(stats.transport_recovery_epoch)
        && stats.video_anchor_clean_observed_at_ms.is_some()
}

/// 恢复会话退出 `receiverWaitingKeyframe` 焊死：新鲜 IDR/锚点 → decode 输出 → 超时降级。
pub(crate) fn recovery_exit_path_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
    thresholds: RecoveryExitThresholds,
) -> RecoveryExitPath {
    let waiting_keyframe =
        stats.video_decoder_recovery_state.as_deref() == Some("waiting-keyframe");
    let submit_stalled = stats
        .submit_age_ms
        .is_some_and(|age| age >= thresholds.timed_fallback_submit_age_ms);
    if waiting_keyframe && submit_stalled && twcc_healthy_for_recovery_fallback(stats) {
        return RecoveryExitPath::TimedFallback;
    }
    if recovery_exit_host_idr_path_active(stats, now_ms) {
        return RecoveryExitPath::HostIdr;
    }
    let decode_fresh = stats
        .latest_video_decode_ok_time_ms
        .is_some_and(|at_ms| (now_ms - at_ms).max(0.0) <= thresholds.degraded_decode_age_ms);
    let host_output_advancing =
        stats.host_frame_present_epoch > 0 && stats.recovery_playback_recovered_at_ms.is_some();
    let submit_pipeline_active = stats
        .submit_age_ms
        .map(|age| age < thresholds.timed_fallback_submit_age_ms)
        .unwrap_or(true);
    if decode_fresh && host_output_advancing && submit_pipeline_active {
        return RecoveryExitPath::DecodeOutput;
    }
    RecoveryExitPath::AwaitingAnchor
}

pub(crate) fn recovery_exit_trace_await_suffix(path: RecoveryExitPath) -> &'static str {
    match path {
        RecoveryExitPath::HostIdr => "hostIdrOrCleanAnchor",
        RecoveryExitPath::DecodeOutput => "decodeOutput",
        RecoveryExitPath::TimedFallback => "timedFallback",
        RecoveryExitPath::AwaitingAnchor => "hostIdrOrCleanAnchor",
    }
}

fn twcc_healthy_for_recovery_fallback(stats: &XbxEngineMediaRuntimeStats) -> bool {
    stats.latest_video_twcc_observation.as_ref().map_or(
        stats.transport_state == xbxengine_protocol::XbxEngineTransportStateDto::Connected,
        |twcc| {
            twcc.twcc_sample_valid && twcc.packet_loss_ratio <= 0.08 && twcc.delivery_ratio >= 0.92
        },
    )
}
