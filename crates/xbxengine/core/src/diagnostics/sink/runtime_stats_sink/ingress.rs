// 由 `runtime_stats_sink` 模块拆分；采集面只写事实，不驱动控制决策。

use crate::{
    XbxEngineFirstFrameLatencyObservation, XbxEngineMediaRuntimeStats,
    XbxEngineVideoIngressTerminationObservation,
};

use super::support::*;
use super::RuntimeStatsSink;

impl RuntimeStatsSink {
    pub(super) fn record_video_ingress_termination_internal(
        stats: &mut XbxEngineMediaRuntimeStats,
        observed_at_ms: f64,
        kind: &str,
        cause: &str,
        upstream_cause: Option<&str>,
        source_subsystem: Option<&str>,
        derived_from_termination_id: Option<u64>,
    ) {
        let termination_id = if kind == "closeIntent" {
            stats.video_ingress_termination_id_seq =
                stats.video_ingress_termination_id_seq.saturating_add(1);
            let next = stats.video_ingress_termination_id_seq;
            stats.latest_video_ingress_termination_id = Some(next);
            next
        } else {
            stats
                .latest_video_ingress_termination_id
                .unwrap_or_else(|| {
                    stats.video_ingress_termination_id_seq =
                        stats.video_ingress_termination_id_seq.saturating_add(1);
                    let next = stats.video_ingress_termination_id_seq;
                    stats.latest_video_ingress_termination_id = Some(next);
                    next
                })
        };
        let observation = XbxEngineVideoIngressTerminationObservation {
            observation_id: Self::next_video_ingress_termination_observation_id(stats),
            termination_id,
            derived_from_termination_id,
            kind: kind.to_string(),
            cause: cause.to_string(),
            upstream_cause: upstream_cause.map(ToString::to_string),
            source_subsystem: source_subsystem.map(ToString::to_string),
            linked_recovery_epoch: Some(stats.transport_recovery_epoch),
            linked_episode_id: stats
                .latest_keyframe_request_episode
                .as_ref()
                .map(|episode| episode.episode_id),
            transport_state: Some(format!("{:?}", stats.transport_state)),
            owner_state: stats.video_owner_state.clone(),
            video_track_state: stats
                .latest_video_track_status
                .as_ref()
                .map(|status| status.state.clone()),
            recent_command: stats.latest_observation_label.clone(),
            observed_at_ms,
        };
        stats.latest_video_ingress_termination_observation = Some(observation);
    }

    pub(super) fn refresh_first_frame_latency_observation(
        stats: &mut XbxEngineMediaRuntimeStats,
        observed_at_ms: f64,
    ) {
        let Some(episode) = stats.latest_keyframe_request_episode.clone() else {
            return;
        };
        let control_ready_to_pli_sent_ms = stats
            .control_ready_at_ms
            .zip(episode.sent_at_ms)
            .map(|(control_ready_at_ms, sent_at_ms)| (sent_at_ms - control_ready_at_ms).max(0.0));
        let pli_sent_to_first_idr_packet_ms = episode
            .sent_at_ms
            .zip(episode.first_keyframe_packet_at_ms)
            .map(|(sent_at_ms, first_packet_at_ms)| (first_packet_at_ms - sent_at_ms).max(0.0));
        let first_idr_packet_to_first_decode_ms = episode
            .first_keyframe_packet_at_ms
            .zip(episode.first_keyframe_decoded_at_ms)
            .map(|(first_packet_at_ms, decoded_at_ms)| {
                (decoded_at_ms - first_packet_at_ms).max(0.0)
            });
        let first_decode_to_clean_anchor_committed_ms = episode
            .first_keyframe_decoded_at_ms
            .zip(stats.video_anchor_clean_observed_at_ms)
            .map(|(decoded_at_ms, committed_at_ms)| (committed_at_ms - decoded_at_ms).max(0.0));
        let clean_anchor_committed_to_display_stable_ms = stats
            .video_anchor_clean_observed_at_ms
            .zip(stats.transport_recovery_episode_closed_at_ms)
            .filter(|_| {
                stats.transport_recovery_episode_close_reason.as_deref()
                    == Some("stableServingSettled")
            })
            .map(|(committed_at_ms, stable_at_ms)| (stable_at_ms - committed_at_ms).max(0.0));
        let continuation_only_seen = episode.first_video_packet_at_ms.is_some()
            && episode.first_video_packet_is_keyframe == Some(false)
            && episode.first_keyframe_packet_at_ms.is_none();
        let terminal_phase = if clean_anchor_committed_to_display_stable_ms.is_some() {
            Some("DisplayStable".to_string())
        } else if first_decode_to_clean_anchor_committed_ms.is_some() {
            Some("CleanAnchorCommitted".to_string())
        } else if episode.first_keyframe_decoded_at_ms.is_some() {
            Some("Decoded".to_string())
        } else if episode.first_keyframe_packet_at_ms.is_some() {
            Some("AnchorSeen".to_string())
        } else if continuation_only_seen {
            Some("ContinuationSeen".to_string())
        } else if episode.sent_at_ms.is_some() {
            Some("WaitingResponse".to_string())
        } else {
            None
        };
        let incomplete_reason = if episode.first_keyframe_decoded_at_ms.is_some()
            && stats.video_anchor_clean_observed_at_ms.is_none()
        {
            if stats.recovery_playback_recovered_at_ms.is_some() {
                Some("playbackRecoveredAnchorPending".to_string())
            } else {
                Some("noCleanAnchorCommit".to_string())
            }
        } else if episode.first_keyframe_decoded_at_ms.is_some()
            && stats.transport_recovery_episode_close_reason.as_deref()
                != Some("stableServingSettled")
            && stats.video_anchor_clean_observed_at_ms.is_some()
        {
            Some("noDisplayStable".to_string())
        } else if episode.sent_at_ms.is_none()
            && episode.first_keyframe_packet_at_ms.is_none()
            && episode.first_keyframe_decoded_at_ms.is_none()
        {
            Some("missingPliSent".to_string())
        } else if continuation_only_seen {
            Some("continuationOnlyAwaitingIdr".to_string())
        } else if episode.first_keyframe_packet_at_ms.is_none() {
            Some("noIdrPacket".to_string())
        } else if episode.first_keyframe_decoded_at_ms.is_none() {
            Some("noDecode".to_string())
        } else if stats.transport_recovery_episode_close_reason.as_deref()
            != Some("stableServingSettled")
        {
            Some("noDisplayStable".to_string())
        } else {
            None
        };
        if control_ready_to_pli_sent_ms.is_none()
            && pli_sent_to_first_idr_packet_ms.is_none()
            && first_idr_packet_to_first_decode_ms.is_none()
            && first_decode_to_clean_anchor_committed_ms.is_none()
            && clean_anchor_committed_to_display_stable_ms.is_none()
        {
            return;
        }
        let transport_detail = format!(
            "firstFrameLatencyTrace controlReadyToPliSentMs={} pliSentToFirstIdrPacketMs={} firstIdrPacketToFirstDecodeMs={} firstDecodeToCleanAnchorCommittedMs={} cleanAnchorCommittedToDisplayStableMs={}",
            format_optional_latency_ms(control_ready_to_pli_sent_ms),
            format_optional_latency_ms(pli_sent_to_first_idr_packet_ms),
            format_optional_latency_ms(first_idr_packet_to_first_decode_ms),
            format_optional_latency_ms(first_decode_to_clean_anchor_committed_ms),
            format_optional_latency_ms(clean_anchor_committed_to_display_stable_ms),
        );
        if let Some(current_episode) = stats.latest_keyframe_request_episode.as_mut() {
            current_episode.transport_detail = Some(transport_detail);
        }
        stats.latest_first_frame_latency_observation =
            Some(XbxEngineFirstFrameLatencyObservation {
                observation_id: Self::next_first_frame_latency_observation_id(stats),
                episode_id: Some(episode.episode_id),
                recovery_epoch: Some(stats.transport_recovery_epoch),
                control_ready_to_pli_sent_ms,
                pli_sent_to_first_idr_packet_ms,
                first_idr_packet_to_first_decode_ms,
                first_decode_to_clean_anchor_committed_ms,
                clean_anchor_committed_to_display_stable_ms,
                terminal_phase,
                incomplete_reason,
                observed_at_ms,
            });
    }

    pub(crate) fn record_video_ingress_rx_closed(&self, observed_at_ms: f64, cause: Option<&str>) {
        self.update(|stats| {
            let resolved_cause = cause.unwrap_or("upstreamSenderDropped");
            let upstream_cause = stats.latest_video_ingress_close_intent_cause.clone();
            stats.latest_observation_label = Some("rtcVideoIngressRxClosed".to_string());
            stats.latest_observation_summary = Some(format!(
                "cause={resolved_cause} observedAtMs={observed_at_ms:.1}"
            ));
            Self::record_video_ingress_termination_internal(
                stats,
                observed_at_ms,
                "rxClosed",
                resolved_cause,
                upstream_cause.as_deref(),
                Some("video-ingress"),
                stats.latest_video_ingress_termination_id,
            );
        });
    }
}
