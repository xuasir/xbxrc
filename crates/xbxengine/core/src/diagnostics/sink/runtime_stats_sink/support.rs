// 由 `runtime_stats_sink` 模块拆分；采集面只写事实，不驱动控制决策。

use crate::transport::rtc::recovery::keyframe_lifecycle::apply_keyframe_episode_lifecycle_field;
use crate::{
    XbxEngineAnchorCandidateLedger, XbxEngineH264InspectionObservation,
    XbxEngineKeyframeRequestEpisodeObservation, XbxEngineMediaRuntimeStats,
};

use super::RuntimeStatsSink;

pub(super) fn upsert_picture_recovery_episode(
    stats: &mut XbxEngineMediaRuntimeStats,
    episode_id: u64,
    update: impl FnOnce(&mut XbxEngineKeyframeRequestEpisodeObservation),
    create: impl FnOnce() -> XbxEngineKeyframeRequestEpisodeObservation,
) -> XbxEngineKeyframeRequestEpisodeObservation {
    let transport_recovery_epoch = stats.transport_recovery_epoch;
    let video_anchor_clean_epoch = stats.video_anchor_clean_epoch;
    let video_anchor_clean_observed_at_ms = stats.video_anchor_clean_observed_at_ms;
    let mut episode = if let Some(index) = stats
        .recent_keyframe_request_episodes
        .iter()
        .position(|episode| episode.episode_id == episode_id)
    {
        let episode = &mut stats.recent_keyframe_request_episodes[index];
        update(episode);
        let cloned = episode.clone();
        stats.recent_keyframe_request_episodes.remove(index);
        cloned
    } else {
        let mut episode = create();
        update(&mut episode);
        episode
    };
    enrich_picture_recovery_first_frame_latency_detail(stats, &mut episode);
    apply_keyframe_episode_lifecycle_field(
        transport_recovery_epoch,
        video_anchor_clean_epoch,
        video_anchor_clean_observed_at_ms,
        &mut episode,
    );
    stats.recent_keyframe_request_episodes.push(episode.clone());
    trim_recent_picture_recovery_episodes(stats);
    episode
}

#[cfg(test)]
pub(super) fn reuse_active_transport_recovery_episode_id(
    stats: &XbxEngineMediaRuntimeStats,
    request_reason: Option<&str>,
) -> Option<u64> {
    if request_reason != Some("receiverWaitingKeyframe") || !stats.transport_recovery_episode_active
    {
        return None;
    }
    let episode = stats.latest_keyframe_request_episode.as_ref()?;
    if !keyframe_episode_observability_active(episode) {
        return None;
    }
    if episode.request_reason.as_deref() != Some("receiverWaitingKeyframe") {
        return None;
    }
    if recovery_window_has_clean_anchor(stats) {
        return None;
    }
    if matches!(
        episode.response_verdict.as_deref(),
        Some("cleanAnchorCommitted" | "transportDeferred" | "transportFailed" | "unsentExpired")
    ) {
        return None;
    }
    Some(episode.episode_id)
}

pub(super) fn keyframe_episode_is_terminal(
    episode: &XbxEngineKeyframeRequestEpisodeObservation,
) -> bool {
    matches!(
        episode.response_verdict.as_deref(),
        Some("cleanAnchorCommitted" | "transportDeferred" | "transportFailed" | "unsentExpired")
    )
}

pub(super) fn should_advance_transport_await_owner_frame(
    episode: &XbxEngineKeyframeRequestEpisodeObservation,
    observed_at_ms: f64,
    rtp_timestamp: Option<u32>,
    latest_h264_observation: Option<&XbxEngineH264InspectionObservation>,
    latest_clean_anchor_submission_episode_id: Option<u64>,
) -> bool {
    if episode.request_reason.as_deref() != Some("receiverWaitingKeyframe")
        || episode.retired_at_ms.is_some()
        || keyframe_episode_is_terminal(episode)
    {
        return false;
    }
    let Some(current_rtp_timestamp) = episode.response_rtp_timestamp else {
        return false;
    };
    let Some(incoming_rtp_timestamp) = rtp_timestamp else {
        return false;
    };
    if episode.first_keyframe_decoded_at_ms.is_some()
        || latest_h264_observation.is_some_and(|inspection| {
            inspection.bound_episode_id == Some(episode.episode_id)
                && inspection.observed_at_ms <= observed_at_ms
                && inspection_matches_transport_recovery_continuation_family(episode, inspection)
        })
        || latest_clean_anchor_submission_episode_id
            .is_some_and(|episode_id| episode_id == episode.episode_id)
    {
        return false;
    }
    incoming_rtp_timestamp != current_rtp_timestamp
        && observed_at_ms
            >= episode
                .first_keyframe_packet_at_ms
                .unwrap_or(episode.requested_at_ms)
}

pub(super) fn advance_transport_await_owner_frame(
    episode: &mut XbxEngineKeyframeRequestEpisodeObservation,
    observed_at_ms: f64,
    rtp_timestamp: Option<u32>,
    detail: &str,
) {
    episode.first_keyframe_packet_at_ms = Some(observed_at_ms);
    episode.first_keyframe_decoded_at_ms = None;
    episode.response_rtp_timestamp = rtp_timestamp;
    episode.response_frame_seq = None;
    episode.response_verdict = Some("pending".to_string());
    episode.status_detail = Some(detail.to_string());
}

pub(super) fn retire_transport_await_episode_for_new_recovery_epoch(
    stats: &mut XbxEngineMediaRuntimeStats,
    observed_at_ms: f64,
) {
    let Some(episode) = stats.latest_keyframe_request_episode.as_mut() else {
        return;
    };
    if episode.request_reason.as_deref() != Some("receiverWaitingKeyframe")
        || !keyframe_episode_observability_active(episode)
    {
        return;
    }
    episode.retired_at_ms = Some(observed_at_ms);
    episode.status_detail = Some("supersededByNewRecoveryEpoch".to_string());
    let updated = episode.clone();
    sync_recent_picture_recovery_episode(stats, updated);
}

/// 对「已发出且本轮 episode 终端失败」计数一次，供 decoder reset 门槛与诊断使用。
pub(super) fn maybe_count_keyframe_sent_terminal_failure(
    stats: &mut XbxEngineMediaRuntimeStats,
    episode_id: u64,
) {
    if stats.keyframe_sent_failure_last_counted_episode_id == Some(episode_id) {
        return;
    }
    stats.keyframe_sent_failure_last_counted_episode_id = Some(episode_id);
    stats.keyframe_consecutive_sent_failures =
        stats.keyframe_consecutive_sent_failures.saturating_add(1);
}

pub(super) fn trim_recent_picture_recovery_episodes(stats: &mut XbxEngineMediaRuntimeStats) {
    if stats.recent_keyframe_request_episodes.len()
        <= RuntimeStatsSink::RECENT_PICTURE_RECOVERY_EPISODE_LIMIT
    {
        return;
    }
    let overflow = stats.recent_keyframe_request_episodes.len()
        - RuntimeStatsSink::RECENT_PICTURE_RECOVERY_EPISODE_LIMIT;
    stats.recent_keyframe_request_episodes.drain(0..overflow);
}

pub(super) fn sync_recent_picture_recovery_episode(
    stats: &mut XbxEngineMediaRuntimeStats,
    mut episode: XbxEngineKeyframeRequestEpisodeObservation,
) {
    let episode_id = episode.episode_id;
    enrich_picture_recovery_first_frame_latency_detail(stats, &mut episode);
    apply_keyframe_episode_lifecycle_field(
        stats.transport_recovery_epoch,
        stats.video_anchor_clean_epoch,
        stats.video_anchor_clean_observed_at_ms,
        &mut episode,
    );
    let synced_episode = episode.clone();
    if let Some(index) = stats
        .recent_keyframe_request_episodes
        .iter()
        .position(|candidate| candidate.episode_id == episode.episode_id)
    {
        stats.recent_keyframe_request_episodes[index] = episode;
    } else {
        stats.recent_keyframe_request_episodes.push(episode);
        trim_recent_picture_recovery_episodes(stats);
    }
    // `latest` 与 `recent` 必须保持同一份 episode 事实，避免 transport_detail / lifecycle 等字段分叉。
    if let Some(latest) = stats.latest_keyframe_request_episode.as_mut() {
        if latest.episode_id == episode_id {
            *latest = synced_episode;
        }
    }
}

pub(super) fn format_picture_recovery_response_observed_summary(
    episode: &XbxEngineKeyframeRequestEpisodeObservation,
    observed_at_ms: f64,
    rtp_timestamp: Option<u32>,
    is_keyframe: bool,
    detail: &str,
    first_video_packet_sequence: Option<u16>,
    first_keyframe_packet_sequence: Option<u16>,
    response_oos_depth_p75: Option<u16>,
    response_head_missing_active: bool,
    gap_expired_before_keyframe: bool,
) -> String {
    let sent_to_first_packet_ms = episode
        .sent_at_ms
        .map(|sent_at_ms| (observed_at_ms - sent_at_ms).max(0.0));
    format!(
        "episodeId={} rtpTimestamp={} isKeyframe={} detail={} sentToFirstPacketMs={} firstVideoPacketAtMs={} firstVideoPacketSeq={} firstVideoPacketIsKeyframe={} firstKeyframePacketSeq={} firstKeyframeArrivalLagMs={} oosDepthP75={} headMissingActive={} gapExpiredBeforeKeyframe={}",
        episode.episode_id,
        rtp_timestamp
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_string()),
        is_keyframe,
        detail,
        sent_to_first_packet_ms
            .map(|value| format!("{value:.1}"))
            .unwrap_or_else(|| "none".to_string()),
        episode
            .first_video_packet_at_ms
            .map(|value| format!("{value:.1}"))
            .unwrap_or_else(|| "none".to_string()),
        first_video_packet_sequence
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_string()),
        episode
            .first_video_packet_is_keyframe
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_string()),
        first_keyframe_packet_sequence
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_string()),
        if is_keyframe {
            episode
                .first_keyframe_packet_at_ms
                .or(Some(observed_at_ms))
                .zip(episode.sent_at_ms)
                .map(|(first_keyframe_at_ms, sent_at_ms)| {
                    (first_keyframe_at_ms - sent_at_ms).max(0.0)
                })
        } else {
            None
        }
        .map(|value| format!("{value:.1}"))
        .unwrap_or_else(|| "none".to_string()),
        response_oos_depth_p75
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_string()),
        response_head_missing_active,
        gap_expired_before_keyframe,
    )
}

pub(super) fn format_h264_inspection_summary(
    observation: &XbxEngineH264InspectionObservation,
) -> String {
    format!(
        "rtpTimestamp={} isIdr={} bootstrapReady={} bootstrapRejectReason={} continuationVerdict={} admissionAccepted={} boundEpisodeId={} boundAsRecoveryResponse={}",
        observation
            .frame_rtp_timestamp
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_string()),
        observation.is_idr,
        observation.bootstrap_ready,
        observation
            .bootstrap_reject_reason
            .as_deref()
            .unwrap_or("none"),
        observation
            .continuation_verdict
            .as_deref()
            .unwrap_or("none"),
        observation.admission_accepted,
        observation
            .bound_episode_id
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_string()),
        observation
            .bound_as_recovery_response
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_string()),
    )
}

pub(super) fn emit_picture_recovery_response_diagnosis_probe(
    stats: &XbxEngineMediaRuntimeStats,
    episode: Option<&XbxEngineKeyframeRequestEpisodeObservation>,
    observation: &XbxEngineH264InspectionObservation,
) {
    let Some(episode) = episode else {
        return;
    };
    if !request_reason_is_transport_recovery_keyframe_family(episode.request_reason.as_deref()) {
        return;
    }
    let bound_as_recovery_response = observation.bound_as_recovery_response.unwrap_or(false);
    if !bound_as_recovery_response {
        return;
    }
    let reject_reason = observation
        .bootstrap_reject_reason
        .as_deref()
        .unwrap_or("none");
    let unusable_response = !observation.admission_accepted
        || matches!(
            reject_reason,
            "bootstrapMissingSps"
                | "bootstrapMissingPps"
                | "bootstrapMissingIdr"
                | "mixedIdrWithTrailingDelta"
                | "inspectionRejectInvalidSliceHeader"
        );
    if !unusable_response {
        return;
    }
    crate::xbx_log_warn!(
        "[keyframe-diagnosis] unusable-recovery-response episodeId={} lifecycle={} status={} verdict={} rejectReason={} admissionAccepted={} bootstrapReady={} isIdr={} rtpTs={} nalCount={} inbandSps={} inbandPps={} sentFailureStreak={}",
        episode.episode_id,
        episode.lifecycle_phase.as_deref().unwrap_or("none"),
        episode.status.as_str(),
        episode.response_verdict.as_deref().unwrap_or("none"),
        reject_reason,
        observation.admission_accepted,
        observation.bootstrap_ready,
        observation.is_idr,
        observation
            .frame_rtp_timestamp
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_string()),
        observation.nal_count,
        observation.has_inband_sps,
        observation.has_inband_pps,
        stats.keyframe_consecutive_sent_failures,
    );
}

pub(super) fn apply_picture_recovery_episode_sent(
    episode: &mut XbxEngineKeyframeRequestEpisodeObservation,
    request_kind: &str,
    sent_at_ms: f64,
    deadline_at_ms: Option<f64>,
) {
    if episode.request_kind.as_deref() != Some("fir") {
        episode.request_kind = Some(request_kind.to_string());
    }
    episode.status = "sent".to_string();
    episode.status_detail = None;
    episode.sent_at_ms = Some(
        episode
            .sent_at_ms
            .map_or(sent_at_ms, |existing| existing.min(sent_at_ms)),
    );
    if let Some(deadline_at_ms) = deadline_at_ms {
        episode.deadline_at_ms = Some(
            episode
                .deadline_at_ms
                .map_or(deadline_at_ms, |existing| existing.min(deadline_at_ms)),
        );
    }
    if episode.response_verdict.is_none() {
        episode.response_verdict = Some("pending".to_string());
    }
}

pub(super) fn format_optional_latency_ms(value: Option<f64>) -> String {
    value
        .map(|latency_ms| format!("{latency_ms:.1}"))
        .unwrap_or_else(|| "none".to_string())
}

pub(super) fn enrich_picture_recovery_first_frame_latency_detail(
    stats: &XbxEngineMediaRuntimeStats,
    episode: &mut XbxEngineKeyframeRequestEpisodeObservation,
) {
    if matches!(
        episode.status.as_str(),
        "deferred" | "failed" | "missed" | "expired-unsent"
    ) {
        return;
    }
    let pli_sent_at_ms = episode.sent_at_ms;
    let first_idr_packet_at_ms = episode
        .first_keyframe_packet_at_ms
        .or(episode.first_video_packet_at_ms);
    let first_decode_at_ms = episode.first_keyframe_decoded_at_ms;
    let clean_anchor_committed_at_ms =
        if stats.video_anchor_clean_epoch == Some(stats.transport_recovery_epoch) {
            stats.video_anchor_clean_observed_at_ms
        } else {
            None
        };
    let display_stable_at_ms = stats.transport_recovery_episode_closed_at_ms.filter(|_| {
        stats.transport_recovery_episode_close_reason.as_deref() == Some("stableServingSettled")
    });
    let control_ready_to_pli_sent_ms = stats
        .control_ready_at_ms
        .zip(pli_sent_at_ms)
        .map(|(control_ready_at_ms, sent_at_ms)| (sent_at_ms - control_ready_at_ms).max(0.0));
    let pli_sent_to_first_idr_packet_ms = pli_sent_at_ms
        .zip(first_idr_packet_at_ms)
        .map(|(sent_at_ms, first_packet_at_ms)| (first_packet_at_ms - sent_at_ms).max(0.0));
    let first_idr_packet_to_first_decode_ms = first_idr_packet_at_ms
        .zip(first_decode_at_ms)
        .map(|(first_packet_at_ms, decoded_at_ms)| (decoded_at_ms - first_packet_at_ms).max(0.0));
    let first_decode_to_clean_anchor_committed_ms = first_decode_at_ms
        .zip(clean_anchor_committed_at_ms)
        .map(|(decoded_at_ms, committed_at_ms)| (committed_at_ms - decoded_at_ms).max(0.0));
    let clean_anchor_committed_to_display_stable_ms = clean_anchor_committed_at_ms
        .zip(display_stable_at_ms)
        .map(|(committed_at_ms, stable_at_ms)| (stable_at_ms - committed_at_ms).max(0.0));
    if control_ready_to_pli_sent_ms.is_none()
        && pli_sent_to_first_idr_packet_ms.is_none()
        && first_idr_packet_to_first_decode_ms.is_none()
        && first_decode_to_clean_anchor_committed_ms.is_none()
        && clean_anchor_committed_to_display_stable_ms.is_none()
    {
        return;
    }
    episode.transport_detail = Some(format!(
        "firstFrameLatencyTrace controlReadyToPliSentMs={} pliSentToFirstIdrPacketMs={} firstIdrPacketToFirstDecodeMs={} firstDecodeToCleanAnchorCommittedMs={} cleanAnchorCommittedToDisplayStableMs={}",
        format_optional_latency_ms(control_ready_to_pli_sent_ms),
        format_optional_latency_ms(pli_sent_to_first_idr_packet_ms),
        format_optional_latency_ms(first_idr_packet_to_first_decode_ms),
        format_optional_latency_ms(first_decode_to_clean_anchor_committed_ms),
        format_optional_latency_ms(clean_anchor_committed_to_display_stable_ms),
    ));
}

/// 当前 recovery epoch 下是否已观测到与本 epoch 对齐的 clean anchor（控制态，非单次 probe 语义）。
pub(super) fn recovery_window_has_clean_anchor(stats: &XbxEngineMediaRuntimeStats) -> bool {
    stats.video_anchor_clean_observed_at_ms.is_some()
        && stats.video_anchor_clean_epoch == Some(stats.transport_recovery_epoch)
}

pub(super) fn request_reason_is_transport_recovery_keyframe_family(
    request_reason: Option<&str>,
) -> bool {
    matches!(
        request_reason,
        Some("receiverWaitingKeyframe" | "transportExpiredDeadline")
    )
}

pub(super) fn keyframe_episode_observability_active(
    episode: &XbxEngineKeyframeRequestEpisodeObservation,
) -> bool {
    episode.retired_at_ms.is_none()
}

pub(super) fn episode_keeps_transport_recovery_family_context(
    episode: &XbxEngineKeyframeRequestEpisodeObservation,
) -> bool {
    matches!(
        episode.status.as_str(),
        "packet-seen" | "decoded" | "response-observed" | "missed"
    ) && request_reason_is_transport_recovery_keyframe_family(episode.request_reason.as_deref())
        && episode
            .first_keyframe_packet_at_ms
            .or(episode.first_video_packet_at_ms)
            .is_some()
}

pub(super) fn unresolved_transport_await_episode_keeps_serviceable_continuation_bridge(
    stats: &XbxEngineMediaRuntimeStats,
    episode: &XbxEngineKeyframeRequestEpisodeObservation,
    observation: &XbxEngineH264InspectionObservation,
) -> bool {
    if !request_reason_is_transport_recovery_keyframe_family(episode.request_reason.as_deref())
        || !keyframe_episode_observability_active(episode)
        || !matches!(episode.status.as_str(), "requested" | "sent")
        || current_transport_recovery_keyframe_episode_id(stats) != Some(episode.episode_id)
        || !stats.transport_recovery_episode_active
    {
        return false;
    }
    let bridge_started_at_ms = episode.sent_at_ms.unwrap_or(episode.requested_at_ms);
    if observation.observed_at_ms < bridge_started_at_ms
        || observation.is_idr
        || observation.bootstrap_ready
        || !transport_recovery_serviceable_continuation(observation)
    {
        return false;
    }
    if stats.recovery_playback_recovered_at_ms.is_some() {
        return true;
    }
    stats
        .latest_h264_inspection_observation
        .as_ref()
        .is_some_and(|latest| {
            latest.bound_episode_id == Some(episode.episode_id)
                && latest.bound_recovery_epoch == Some(stats.transport_recovery_epoch)
                && latest.observed_at_ms <= observation.observed_at_ms
                && transport_recovery_serviceable_continuation(latest)
        })
}

pub(super) fn transport_await_episode_matches_serviceable_continuation(
    stats: &XbxEngineMediaRuntimeStats,
    episode: &XbxEngineKeyframeRequestEpisodeObservation,
    observation: &XbxEngineH264InspectionObservation,
) -> bool {
    inspection_matches_transport_recovery_continuation_family(episode, observation)
        || unresolved_transport_await_episode_keeps_serviceable_continuation_bridge(
            stats,
            episode,
            observation,
        )
}

pub(super) fn collect_keyframe_episode_candidates(
    stats: &XbxEngineMediaRuntimeStats,
) -> Vec<XbxEngineKeyframeRequestEpisodeObservation> {
    let mut out: Vec<XbxEngineKeyframeRequestEpisodeObservation> = Vec::new();
    if let Some(episode) = stats.latest_keyframe_request_episode.as_ref() {
        out.push(episode.clone());
    }
    for episode in stats.recent_keyframe_request_episodes.iter() {
        if !out
            .iter()
            .any(|existing| existing.episode_id == episode.episode_id)
        {
            out.push(episode.clone());
        }
    }
    out
}

pub(super) fn host_display_rtp_qualifies_for_fresh_anchor(
    stats: &XbxEngineMediaRuntimeStats,
    displayed_rtp: u32,
    now_ms: f64,
) -> bool {
    if crate::transport::rtc::recovery::contract::recovery_supply_break_active_from_stats(
        stats, now_ms,
    ) {
        return false;
    }
    if stats
        .latest_h264_inspection_observation
        .as_ref()
        .is_some_and(|observation| {
            observation.frame_rtp_timestamp == Some(displayed_rtp)
                && observation.is_idr
                && observation.bootstrap_ready
                && observation.admission_accepted
        })
    {
        return true;
    }
    stats.recovery_pending_displayed_idr_rtp == Some(displayed_rtp)
        && crate::transport::rtc::recovery::contract::decoder_reference_synced_from_stats(
            stats, now_ms,
        )
}

pub(super) fn current_transport_recovery_keyframe_episode_snapshot(
    stats: &XbxEngineMediaRuntimeStats,
) -> Option<XbxEngineKeyframeRequestEpisodeObservation> {
    collect_keyframe_episode_candidates(stats)
        .into_iter()
        .filter(|episode| {
            keyframe_episode_observability_active(episode)
                && request_reason_is_transport_recovery_keyframe_family(
                    episode.request_reason.as_deref(),
                )
        })
        .max_by(|left, right| {
            left.first_keyframe_decoded_at_ms
                .is_some()
                .cmp(&right.first_keyframe_decoded_at_ms.is_some())
                .then_with(|| {
                    left.first_keyframe_packet_at_ms
                        .is_some()
                        .cmp(&right.first_keyframe_packet_at_ms.is_some())
                })
                .then_with(|| {
                    left.first_video_packet_at_ms
                        .is_some()
                        .cmp(&right.first_video_packet_at_ms.is_some())
                })
                .then_with(|| left.sent_at_ms.is_some().cmp(&right.sent_at_ms.is_some()))
                .then_with(|| left.requested_at_ms.total_cmp(&right.requested_at_ms))
                .then_with(|| left.episode_id.cmp(&right.episode_id))
        })
}

pub(super) fn current_transport_recovery_keyframe_episode_id(
    stats: &XbxEngineMediaRuntimeStats,
) -> Option<u64> {
    current_transport_recovery_keyframe_episode_snapshot(stats).map(|episode| episode.episode_id)
}

pub(super) fn select_picture_recovery_episode_snapshot_for_h264_inspection(
    stats: &XbxEngineMediaRuntimeStats,
    inspection: &XbxEngineH264InspectionObservation,
) -> Option<XbxEngineKeyframeRequestEpisodeObservation> {
    let candidates = collect_keyframe_episode_candidates(stats);
    if let Some(rtp) = inspection.frame_rtp_timestamp {
        // 精确匹配：如果帧RTP与episode的response_rtp_timestamp匹配，绑定
        if let Some(episode) = candidates
            .iter()
            .find(|episode| episode.response_rtp_timestamp == Some(rtp))
        {
            return Some(episode.clone());
        }
        // 精确匹配：如果帧RTP与episode的first_video_packet_rtp_timestamp匹配，绑定
        if let Some(episode) = candidates
            .iter()
            .find(|episode| episode.first_video_packet_rtp_timestamp == Some(rtp))
        {
            return Some(episode.clone());
        }
    }
    if let Some(current_episode) = current_transport_recovery_keyframe_episode_snapshot(stats)
        .filter(|episode| {
            transport_await_episode_matches_serviceable_continuation(stats, episode, inspection)
        })
    {
        return Some(current_episode);
    }
    const WINDOW_MS: f64 = 10_000.0;
    let mut best: Option<XbxEngineKeyframeRequestEpisodeObservation> = None;
    let mut best_delta = f64::INFINITY;
    for episode in candidates.iter().filter(|episode| {
        keyframe_episode_observability_active(episode)
            && request_reason_is_transport_recovery_keyframe_family(
                episode.request_reason.as_deref(),
            )
    }) {
        // 修复：只允许等待响应的episode进行fallback绑定
        // 已经收到响应的episode（response-observed及之后）不应该再通过fallback绑定
        match episode.status.as_str() {
            "requested" | "sent" => {} // 允许fallback绑定
            _ => continue,             // 其他状态跳过（已收到响应或已结束）
        }

        let anchor_ms = episode.sent_at_ms.unwrap_or(episode.requested_at_ms);
        let delta = (inspection.observed_at_ms - anchor_ms).abs();
        if delta < WINDOW_MS && delta < best_delta {
            best_delta = delta;
            best = Some(episode.clone());
        }
    }
    best
}

pub(super) fn inspection_matches_recovery_picture_recovery_response(
    stats: &XbxEngineMediaRuntimeStats,
    episode: &XbxEngineKeyframeRequestEpisodeObservation,
    observation: &XbxEngineH264InspectionObservation,
) -> bool {
    if !transport_await_episode_matches_serviceable_continuation(stats, episode, observation)
        && !episode_keeps_transport_recovery_family_context(episode)
    {
        return false;
    }
    if episode.response_rtp_timestamp.is_some()
        && episode.response_rtp_timestamp == observation.frame_rtp_timestamp
    {
        return true;
    }
    current_transport_recovery_keyframe_episode_id(stats) == Some(episode.episode_id)
        && transport_await_episode_matches_serviceable_continuation(stats, episode, observation)
}

pub(super) fn transport_recovery_serviceable_continuation(
    observation: &XbxEngineH264InspectionObservation,
) -> bool {
    observation.admission_accepted
        && observation.committed_sps_present
        && observation.committed_pps_present
        && observation.delta_continuation_ready
        && matches!(
            observation.continuation_verdict.as_deref(),
            Some("receiverLocalContinuation" | "continuationReady")
        )
        && matches!(
            observation.bootstrap_reject_reason.as_deref(),
            Some("bootstrapMissingIdr" | "NonIdrVcl")
        )
}

pub(super) fn inspection_matches_transport_recovery_continuation_family(
    episode: &XbxEngineKeyframeRequestEpisodeObservation,
    observation: &XbxEngineH264InspectionObservation,
) -> bool {
    if !episode_keeps_transport_recovery_family_context(episode) {
        return false;
    }
    let Some(response_seen_at_ms) = episode
        .first_keyframe_packet_at_ms
        .or(episode.first_video_packet_at_ms)
    else {
        return false;
    };
    if observation.observed_at_ms < response_seen_at_ms
        || observation.is_idr
        || observation.bootstrap_ready
    {
        return false;
    }
    transport_recovery_serviceable_continuation(observation)
}

pub(super) fn classify_h264_reject(
    observation: &XbxEngineH264InspectionObservation,
) -> Option<String> {
    if observation.continuation_verdict.as_deref() == Some("receiverLocalContinuation") {
        return Some("receiverLocalContinuation".to_string());
    }
    if !matches!(
        observation.bootstrap_reject_reason.as_deref(),
        Some("bootstrapMissingIdr" | "NonIdrVcl")
    ) {
        return observation
            .bootstrap_reject_reason
            .as_ref()
            .map(|reason| format!("h264Reject:{reason}"));
    }
    if observation.admission_accepted
        && observation.delta_continuation_ready
        && observation.bound_episode_id.is_some()
    {
        return Some("localWindowAcceptedButBootstrapRejected".to_string());
    }
    if observation.bound_episode_id.is_some() {
        return Some("remoteNoIdrYet".to_string());
    }
    if observation.admission_accepted
        && observation.delta_continuation_ready
        && observation.committed_sps_present
        && observation.committed_pps_present
    {
        return Some("receiverLocalContinuation".to_string());
    }
    if observation.delta_continuation_ready
        && observation.committed_sps_present
        && observation.committed_pps_present
        && matches!(
            observation.bootstrap_reject_reason.as_deref(),
            Some("bootstrapMissingIdr" | "NonIdrVcl")
        )
    {
        return Some("receiverLocalContinuation".to_string());
    }
    Some("outOfRecoveryContextContinuation".to_string())
}

/// 将 anchor ledger 与同帧/同 recovery 上下文的 episode 绑定，避免 probe 硬拼全局 latest。
pub(super) fn select_episode_snapshot_for_anchor_ledger(
    stats: &XbxEngineMediaRuntimeStats,
    ledger: &XbxEngineAnchorCandidateLedger,
) -> Option<XbxEngineKeyframeRequestEpisodeObservation> {
    let candidates = collect_keyframe_episode_candidates(stats);
    if let Some(rtp) = ledger.frame_rtp_timestamp {
        if let Some(episode) = candidates
            .iter()
            .find(|episode| episode.response_rtp_timestamp == Some(rtp))
        {
            return Some(episode.clone());
        }
        if let Some(episode) = candidates
            .iter()
            .find(|episode| episode.first_video_packet_rtp_timestamp == Some(rtp))
        {
            return Some(episode.clone());
        }
    }
    if ledger.recovery_epoch == stats.transport_recovery_epoch {
        let mut transport_await: Vec<&XbxEngineKeyframeRequestEpisodeObservation> = candidates
            .iter()
            .filter(|episode| {
                keyframe_episode_observability_active(episode)
                    && request_reason_is_transport_recovery_keyframe_family(
                        episode.request_reason.as_deref(),
                    )
                    && episode.sent_at_ms.is_some()
            })
            .collect();
        transport_await.sort_by_key(|episode| episode.episode_id);
        if let Some(episode) = transport_await.last() {
            return Some((*episode).clone());
        }
    }
    const WINDOW_MS: f64 = 10_000.0;
    if ledger.recovery_epoch != stats.transport_recovery_epoch {
        return None;
    }
    let mut best: Option<XbxEngineKeyframeRequestEpisodeObservation> = None;
    let mut best_delta = f64::INFINITY;
    for episode in candidates.iter().filter(|episode| {
        keyframe_episode_observability_active(episode)
            && request_reason_is_transport_recovery_keyframe_family(
                episode.request_reason.as_deref(),
            )
    }) {
        let anchor_ms = episode.sent_at_ms.unwrap_or(episode.requested_at_ms);
        let delta = (ledger.observed_at_ms - anchor_ms).abs();
        if delta < WINDOW_MS && delta < best_delta {
            best_delta = delta;
            best = Some(episode.clone());
        }
    }
    best
}

pub(super) fn emit_picture_recovery_closure_probe(
    stats: &XbxEngineMediaRuntimeStats,
    stage: &str,
    observed_at_ms: f64,
    episode: Option<&XbxEngineKeyframeRequestEpisodeObservation>,
    anchor: Option<&XbxEngineAnchorCandidateLedger>,
) {
    let (episode_id, sent_at_ms, response_seen, decoded_at_ms, response_verdict, lifecycle_phase) =
        episode
            .map(|episode| {
                (
                    Some(episode.episode_id),
                    episode.sent_at_ms,
                    episode
                        .first_keyframe_packet_at_ms
                        .or(episode.first_video_packet_at_ms),
                    episode.first_keyframe_decoded_at_ms,
                    episode.response_verdict.clone(),
                    episode.lifecycle_phase.clone(),
                )
            })
            .unwrap_or((None, None, None, None, None, None));
    let (anchor_state, anchor_source, anchor_epoch) = anchor
        .map(|candidate| {
            (
                Some(format!("{:?}", candidate.state)),
                Some(candidate.source_event.clone()),
                Some(candidate.recovery_epoch),
            )
        })
        .unwrap_or((None, None, None));
    let recovery_has_clean_anchor = recovery_window_has_clean_anchor(stats);
    let probe_clean_anchor = stage == "clean-anchor";
    let sent_failure_streak = stats.keyframe_consecutive_sent_failures;
    // 关键帧恢复闭环：request -> response -> decode -> clean-anchor。
    // cleanAnchorCommitted 与 recoveryHasCleanAnchor 对齐（仅当前 recovery epoch）；probeCleanAnchor 标记单次 clean-anchor probe。
    crate::xbx_log_debug!(
        "[keyframe-closure] stage={} atMs={:.1} episodeId={} lifecycle={} sentAtMs={} responseSeenAtMs={} decodedAtMs={} verdict={} sentFailureStreak={} anchorState={} anchorSource={} anchorEpoch={} cleanAnchorCommitted={} recoveryHasCleanAnchor={} probeCleanAnchor={}",
        stage,
        observed_at_ms,
        episode_id
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_string()),
        lifecycle_phase
            .as_deref()
            .unwrap_or("none"),
        sent_at_ms
            .map(|value| format!("{value:.1}"))
            .unwrap_or_else(|| "none".to_string()),
        response_seen
            .map(|value| format!("{value:.1}"))
            .unwrap_or_else(|| "none".to_string()),
        decoded_at_ms
            .map(|value| format!("{value:.1}"))
            .unwrap_or_else(|| "none".to_string()),
        response_verdict.unwrap_or_else(|| "none".to_string()),
        sent_failure_streak,
        anchor_state.unwrap_or_else(|| "none".to_string()),
        anchor_source.unwrap_or_else(|| "none".to_string()),
        anchor_epoch
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_string()),
        recovery_has_clean_anchor,
        recovery_has_clean_anchor,
        probe_clean_anchor
    );
}
