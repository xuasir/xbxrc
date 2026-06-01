//! 当前 `transport_recovery_epoch` 内 picture recovery 闭合谓词（coordinator / owner 共用）。

use crate::XbxEngineMediaRuntimeStats;

use super::decode_sync::decoder_reference_synced_from_stats;

/// Owner / stats 共用的 receive picture recovery 闭合字段。
pub(crate) struct ReceivePictureRecoveryCompleteFields<'a> {
    pub recovery_epoch: u64,
    pub receive_keyframe_required: Option<bool>,
    pub receive_keyframe_response_state: Option<&'a str>,
    pub receive_display_state: Option<&'a str>,
    pub recovery_displayed_idr_at_ms: Option<f64>,
    pub clean_anchor_epoch: Option<u64>,
    pub decoder_reference_synced: bool,
}

pub(crate) fn receive_picture_recovery_complete_from_fields(
    fields: &ReceivePictureRecoveryCompleteFields<'_>,
) -> bool {
    if fields.receive_keyframe_required == Some(true) {
        return false;
    }
    if fields.receive_keyframe_response_state != Some("usable-idr") {
        return false;
    }
    if fields.receive_display_state == Some("display-stable")
        && fields.recovery_displayed_idr_at_ms.is_some()
        && fields.clean_anchor_epoch == Some(fields.recovery_epoch)
    {
        return true;
    }
    fields.clean_anchor_epoch == Some(fields.recovery_epoch) && fields.decoder_reference_synced
}

const DECODER_REFERENCE_SYNCED_FRESH_MS: f64 = 2_000.0;

fn picture_recovery_evaluation_now_ms(stats: &XbxEngineMediaRuntimeStats) -> f64 {
    stats
        .recovery_displayed_idr_at_ms
        .or(stats.recovery_fresh_anchor_recovered_at_ms)
        .or(stats.video_anchor_clean_observed_at_ms)
        .or(stats
            .latest_h264_inspection_observation
            .as_ref()
            .filter(|obs| obs.bound_recovery_epoch == Some(stats.transport_recovery_epoch))
            .map(|obs| obs.observed_at_ms))
        .unwrap_or(f64::MAX)
}

fn decoder_reference_synced_for_recovery_epoch(
    recovery_epoch: u64,
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> bool {
    if stats
        .recovery_decoder_reference_synced_at_ms
        .is_some_and(|synced_at| {
            stats
                .transport_recovery_episode_opened_at_ms
                .is_some_and(|opened_at| synced_at >= opened_at)
                && (now_ms - synced_at).max(0.0) <= DECODER_REFERENCE_SYNCED_FRESH_MS
        })
    {
        return true;
    }
    if !decoder_reference_synced_from_stats(stats, now_ms) {
        return false;
    }
    stats
        .latest_h264_inspection_observation
        .as_ref()
        .is_some_and(|obs| {
            obs.bound_recovery_epoch == Some(recovery_epoch)
                && obs.frame_rtp_timestamp == stats.latest_video_decode_ok_rtp_timestamp
        })
}

/// 仅认当前轮的 usable-idr + decoder sync + clean anchor / display-stable。
pub(crate) fn receive_picture_recovery_complete(
    recovery_epoch: u64,
    stats: &XbxEngineMediaRuntimeStats,
) -> bool {
    let now_ms = picture_recovery_evaluation_now_ms(stats);
    receive_picture_recovery_complete_at(recovery_epoch, stats, now_ms)
}

pub(crate) fn receive_picture_recovery_complete_at(
    recovery_epoch: u64,
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> bool {
    if stats.receive_keyframe_required == Some(true) {
        return false;
    }
    if stats.receive_keyframe_response_state.as_deref() != Some("usable-idr") {
        return false;
    }
    if stats.receive_display_state.as_deref() == Some("display-stable")
        && stats.recovery_displayed_idr_at_ms.is_some()
        && stats.video_anchor_clean_epoch == Some(recovery_epoch)
    {
        return true;
    }
    stats.video_anchor_clean_epoch == Some(recovery_epoch)
        && decoder_reference_synced_for_recovery_epoch(recovery_epoch, stats, now_ms)
}

/// stats 当前 `transport_recovery_epoch` 的便捷入口。
pub(crate) fn receive_picture_recovery_complete_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
) -> bool {
    receive_picture_recovery_complete(stats.transport_recovery_epoch, stats)
}
