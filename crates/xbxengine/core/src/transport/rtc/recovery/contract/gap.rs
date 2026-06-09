use super::decode_sync::{
    decoder_reference_synced_from_stats, decoder_waiting_keyframe_control_active_from_stats,
    fresh_h264_idr_admission_from_stats, receiver_nack_exhausted_from_stats,
};
use super::display::is_soft_missing_idr_bootstrap_reject_reason;
use super::insert::{derive_packet_recovery_action_stage_from_stats, PacketRecoveryActionStage};
use crate::XbxEngineMediaRuntimeStats;

const MEDIA_SUPPLY_PRIMING_ACQUISITION_WINDOW_MS: f64 = 5_000.0;
const MEDIA_SUPPLY_PRIMING_MAX_DECODE_AGE_MS: f64 = 200.0;
const MEDIA_SUPPLY_PRIMING_MAX_SUBMIT_AGE_MS: f64 = 500.0;

fn media_supply_priming_for_ps_strict(stats: &XbxEngineMediaRuntimeStats, now_ms: f64) -> bool {
    if stats.host_frame_present_epoch == 0 {
        return true;
    }
    if stats
        .media_supply_host_first_present_at_ms
        .is_some_and(|at_ms| (now_ms - at_ms).max(0.0) < MEDIA_SUPPLY_PRIMING_ACQUISITION_WINDOW_MS)
    {
        return true;
    }
    let decode_ok = stats
        .latest_video_decode_ok_time_ms
        .map(|ts| (now_ms - ts).max(0.0) <= MEDIA_SUPPLY_PRIMING_MAX_DECODE_AGE_MS)
        .unwrap_or(false);
    let submit_ok = stats
        .submit_age_ms
        .is_some_and(|age| age <= MEDIA_SUPPLY_PRIMING_MAX_SUBMIT_AGE_MS);
    !(decode_ok && submit_ok)
}

pub(crate) const GAP_KEYFRAME_ONLY_MAX_AGE_MS: f64 = 1_500.0;
const GAP_ABANDON_KEYFRAME_ONLY_MS: f64 = 5_000.0;
/// PS/config 变更后短窗：非 IDR 不进解码器，优先要 IDR。
const PARAMETER_SETS_CHANGE_STRICT_MIN_MS: f64 = 400.0;
const PARAMETER_SETS_CHANGE_STRICT_MAX_MS: f64 = 2_000.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GapVsKeyframeMode {
    RepairFirst,
    KeyframeOnly,
    AbandonGap,
}

pub(crate) fn parameter_sets_change_strict_window_ms(effective_rtt_ms: f64) -> f64 {
    let rtt_ms = effective_rtt_ms.clamp(20.0, 400.0);
    (4.0 * rtt_ms)
        .max(PARAMETER_SETS_CHANGE_STRICT_MIN_MS)
        .min(PARAMETER_SETS_CHANGE_STRICT_MAX_MS)
}

/// PS/config 变更后的短窗：修洞期应优先 IDR，不读 display projection。
pub(crate) fn parameter_sets_change_strict_active_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
    effective_rtt_ms: f64,
) -> bool {
    if fresh_h264_idr_admission_from_stats(stats, now_ms) {
        return false;
    }
    if media_supply_priming_for_ps_strict(stats, now_ms) {
        return false;
    }
    let window_ms = parameter_sets_change_strict_window_ms(effective_rtt_ms);
    stats
        .video_parameter_sets_changed_at_ms
        .is_some_and(|at_ms| (now_ms - at_ms).max(0.0) < window_ms)
}

fn repairing_missing_idr_keyframe_pressure_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> bool {
    if fresh_h264_idr_admission_from_stats(stats, now_ms) {
        return false;
    }
    if decoder_reference_synced_from_stats(stats, now_ms) {
        return false;
    }
    let decoder_waiting = decoder_waiting_keyframe_control_active_from_stats(stats, now_ms);
    if !decoder_waiting && !receiver_nack_exhausted_from_stats(stats) {
        return false;
    }
    let receiver_repairing = stats
        .latest_video_receiver_observation
        .as_ref()
        .is_some_and(|obs| obs.receiver_state == "repairing");
    if !receiver_repairing {
        return false;
    }
    let missing_idr = stats
        .latest_h264_inspection_observation
        .as_ref()
        .is_some_and(|inspection| {
            !inspection.is_idr
                && is_soft_missing_idr_bootstrap_reject_reason(
                    inspection.bootstrap_reject_reason.as_deref(),
                )
        });
    let gap_open = stats
        .latest_video_timeline_observation
        .as_ref()
        .and_then(|timeline| timeline.gap.as_ref())
        .is_some();
    missing_idr
        && (gap_open
            || stats
                .latest_video_receiver_observation
                .as_ref()
                .is_some_and(|obs| obs.keyframe_request_pending || obs.nack_in_flight))
}

pub(crate) fn resolve_gap_vs_keyframe_mode(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
    effective_rtt_ms: f64,
) -> GapVsKeyframeMode {
    let decoder_waiting = decoder_waiting_keyframe_control_active_from_stats(stats, now_ms);
    let action_stage =
        derive_packet_recovery_action_stage_from_stats(stats, now_ms, effective_rtt_ms);
    let gap_age_ms = stats
        .latest_video_timeline_observation
        .as_ref()
        .and_then(|timeline| timeline.gap.as_ref())
        .map(|gap| (now_ms - gap.observed_at_ms).max(0.0));
    let gap_stale = gap_age_ms
        .is_some_and(|age| age >= GAP_KEYFRAME_ONLY_MAX_AGE_MS.max(effective_rtt_ms * 2.0));
    let idr_pressure =
        parameter_sets_change_strict_active_from_stats(stats, now_ms, effective_rtt_ms)
            || repairing_missing_idr_keyframe_pressure_from_stats(stats, now_ms);
    if decoder_waiting
        || gap_stale
        || idr_pressure
        || action_stage >= PacketRecoveryActionStage::WaitKeyframe
    {
        if gap_age_ms.is_some_and(|age| age >= GAP_ABANDON_KEYFRAME_ONLY_MS) {
            return GapVsKeyframeMode::AbandonGap;
        }
        return GapVsKeyframeMode::KeyframeOnly;
    }
    GapVsKeyframeMode::RepairFirst
}

pub(crate) fn gap_keyframe_only_mode_active(mode: GapVsKeyframeMode) -> bool {
    matches!(
        mode,
        GapVsKeyframeMode::KeyframeOnly | GapVsKeyframeMode::AbandonGap
    )
}
