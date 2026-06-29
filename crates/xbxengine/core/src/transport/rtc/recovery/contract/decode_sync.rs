use crate::transport::rtc::session::facts::recovery_episode::{
    recovery_progress_allows_decoder_reset, RecoveryProgressLevel,
};
use crate::XbxEngineMediaRuntimeStats;

use super::display::{
    current_displayed_idr_at_ms_from_stats, current_fresh_anchor_recovered_at_ms_from_stats,
};

const FRESH_H264_IDR_ADMISSION_MS: f64 = 3_000.0;
/// 与 owner TimedFallback 对齐：尽早结束 transport-await 焊死并触发续播窄路径。
pub(crate) fn fresh_h264_idr_admission_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> bool {
    stats
        .latest_h264_inspection_observation
        .as_ref()
        .is_some_and(|inspection| {
            inspection.is_idr
                && inspection.admission_accepted
                && (now_ms - inspection.observed_at_ms).max(0.0) <= FRESH_H264_IDR_ADMISSION_MS
        })
}

/// waiting-keyframe 且无 IDR 进展时禁止本地 decoder reset（Reconfigure 等显式路径除外）。
pub(crate) fn decoder_reset_permitted_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
    progress: Option<RecoveryProgressLevel>,
    now_ms: f64,
    allow_waiting_keyframe_bypass: bool,
) -> bool {
    if allow_waiting_keyframe_bypass {
        return true;
    }
    if stats.video_decoder_recovery_state.as_deref() != Some("waiting-keyframe") {
        return true;
    }
    if fresh_h264_idr_admission_from_stats(stats, now_ms) {
        return true;
    }
    recovery_progress_allows_decoder_reset(progress)
}

const DECODER_REFERENCE_SYNCED_FRESH_MS: f64 = 2_000.0;
pub(crate) const CONTINUATION_NO_OUTPUT_REQUEST_IDR_STREAK: u32 = 8;

pub(crate) fn decoder_reference_synced_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> bool {
    if stats
        .recovery_decoder_reference_synced_at_ms
        .is_some_and(|at| (now_ms - at).max(0.0) <= DECODER_REFERENCE_SYNCED_FRESH_MS)
    {
        return true;
    }
    let Some(decode_ok_ms) = stats.latest_video_decode_ok_time_ms else {
        return false;
    };
    if (now_ms - decode_ok_ms).max(0.0) > DECODER_REFERENCE_SYNCED_FRESH_MS {
        return false;
    }
    let Some(decode_rtp) = stats.latest_video_decode_ok_rtp_timestamp else {
        return false;
    };
    if stats.recovery_displayed_idr_rtp == Some(decode_rtp) {
        return true;
    }
    stats
        .latest_h264_inspection_observation
        .as_ref()
        .is_some_and(|obs| {
            obs.frame_rtp_timestamp == Some(decode_rtp) && obs.is_idr && obs.bootstrap_ready
        })
}

fn decoder_recovery_debt_cleared_by_anchor_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> bool {
    if stats.video_anchor_clean_epoch == Some(stats.transport_recovery_epoch)
        && stats.video_anchor_clean_observed_at_ms.is_some()
    {
        return true;
    }
    if current_fresh_anchor_recovered_at_ms_from_stats(stats).is_some() {
        return true;
    }
    decoder_reference_synced_from_stats(stats, now_ms)
}

/// 控制面只把尚未被 clean anchor / FrameDecoded 事实覆盖的 decoder waiting 视为硬门槛。
pub(crate) fn decoder_waiting_keyframe_control_active_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> bool {
    if stats.video_decoder_recovery_state.as_deref() != Some("waiting-keyframe") {
        return false;
    }
    !decoder_recovery_debt_cleared_by_anchor_from_stats(stats, now_ms)
}

/// 控制面只把尚未被 clean anchor / FrameDecoded 事实覆盖的 no-output streak 视为 IDR 压力。
pub(crate) fn decoder_no_output_request_idr_control_active_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> bool {
    if decoder_recovery_debt_cleared_by_anchor_from_stats(stats, now_ms) {
        return false;
    }
    stats
        .latest_decode_output_path_observation
        .as_ref()
        .and_then(|obs| obs.backend_no_output_streak)
        .unwrap_or(0)
        >= CONTINUATION_NO_OUTPUT_REQUEST_IDR_STREAK
}

pub(crate) fn displayed_idr_decoder_synced_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> bool {
    current_displayed_idr_at_ms_from_stats(stats).is_some()
        && decoder_reference_synced_from_stats(stats, now_ms)
}

pub(crate) fn receiver_nack_exhausted_from_stats(stats: &XbxEngineMediaRuntimeStats) -> bool {
    let receiver_state = stats
        .latest_video_receiver_observation
        .as_ref()
        .map(|obs| obs.receiver_state.as_str());
    let has_active_gap = stats
        .latest_video_timeline_observation
        .as_ref()
        .and_then(|timeline| timeline.gap.as_ref())
        .is_some();
    let nack_in_flight = stats
        .latest_video_receiver_observation
        .as_ref()
        .is_some_and(|obs| obs.nack_in_flight);
    nack_in_flight && has_active_gap && receiver_state != Some("repairing")
}
