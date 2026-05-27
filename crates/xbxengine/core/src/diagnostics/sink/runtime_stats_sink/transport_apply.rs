// 由 `runtime_stats_sink` 模块拆分；采集面只写事实，不驱动控制决策。

use crate::transport::rtc::recovery::keyframe_lifecycle::apply_keyframe_episode_lifecycle_field;
use crate::XbxEngineMediaRuntimeStats;

use super::support::*;
use super::RuntimeStatsSink;

impl RuntimeStatsSink {
    pub(crate) fn apply_begin_transport_recovery_episode(
        stats: &mut XbxEngineMediaRuntimeStats,
        observed_at_ms: f64,
    ) -> u64 {
        if stats.transport_recovery_episode_active {
            return stats.transport_recovery_epoch;
        }
        retire_transport_await_episode_for_new_recovery_epoch(stats, observed_at_ms);
        stats.transport_recovery_epoch = stats.transport_recovery_epoch.saturating_add(1);
        stats.transport_recovery_episode_active = true;
        stats.transport_recovery_episode_opened_at_ms = Some(observed_at_ms);
        stats.transport_recovery_episode_closed_at_ms = None;
        stats.transport_recovery_episode_close_reason = None;
        stats.recovery_playback_recovered_at_ms = None;
        stats.recovery_playback_recovered_phase = None;
        stats.recovery_fresh_anchor_recovered_at_ms = None;
        stats.recovery_displayed_idr_rtp = None;
        stats.recovery_displayed_idr_at_ms = None;
        stats.recovery_pending_displayed_idr_rtp = None;
        Self::apply_clear_transport_clean_anchor(stats);
        stats.keyframe_consecutive_sent_failures = 0;
        stats.keyframe_sent_failure_last_counted_episode_id = None;
        stats.transport_recovery_epoch
    }

    pub(crate) fn apply_advance_transport_recovery_episode(
        stats: &mut XbxEngineMediaRuntimeStats,
        observed_at_ms: f64,
    ) -> u64 {
        retire_transport_await_episode_for_new_recovery_epoch(stats, observed_at_ms);
        stats.transport_recovery_epoch = stats.transport_recovery_epoch.saturating_add(1);
        stats.transport_recovery_episode_active = true;
        stats.transport_recovery_episode_opened_at_ms = Some(observed_at_ms);
        stats.transport_recovery_episode_closed_at_ms = None;
        stats.transport_recovery_episode_close_reason = None;
        stats.recovery_playback_recovered_at_ms = None;
        stats.recovery_playback_recovered_phase = None;
        stats.recovery_fresh_anchor_recovered_at_ms = None;
        stats.recovery_displayed_idr_rtp = None;
        stats.recovery_displayed_idr_at_ms = None;
        stats.recovery_pending_displayed_idr_rtp = None;
        Self::apply_clear_transport_clean_anchor(stats);
        stats.keyframe_consecutive_sent_failures = 0;
        stats.keyframe_sent_failure_last_counted_episode_id = None;
        stats.transport_recovery_epoch
    }

    pub(crate) fn apply_complete_transport_recovery_episode(
        stats: &mut XbxEngineMediaRuntimeStats,
        observed_at_ms: f64,
        reason: &str,
    ) {
        if !stats.transport_recovery_episode_active {
            return;
        }
        stats.transport_recovery_episode_active = false;
        stats.transport_recovery_episode_closed_at_ms = Some(observed_at_ms);
        stats.transport_recovery_episode_close_reason = Some(reason.to_string());
    }

    pub(crate) fn apply_transport_clean_anchor(
        stats: &mut XbxEngineMediaRuntimeStats,
        observed_at_ms: f64,
        source_event: &str,
    ) {
        stats.video_anchor_clean_epoch = Some(stats.transport_recovery_epoch);
        stats.video_anchor_clean_observed_at_ms = Some(observed_at_ms);
        stats.video_anchor_clean_source_event = Some(source_event.to_string());
        stats.recovery_fresh_anchor_recovered_at_ms = Some(observed_at_ms);
        stats.keyframe_consecutive_sent_failures = 0;
        stats.keyframe_sent_failure_last_counted_episode_id = None;
        let transport_recovery_epoch = stats.transport_recovery_epoch;
        let video_anchor_clean_epoch = stats.video_anchor_clean_epoch;
        let video_anchor_clean_observed_at_ms = stats.video_anchor_clean_observed_at_ms;
        if let Some(episode) = stats.latest_keyframe_request_episode.as_mut() {
            episode.status = "succeeded".to_string();
            episode.response_verdict = Some("cleanAnchorCommitted".to_string());
            episode.status_detail = None;
            episode.transport_detail = None;
            episode.retired_at_ms = Some(observed_at_ms);
            apply_keyframe_episode_lifecycle_field(
                transport_recovery_epoch,
                video_anchor_clean_epoch,
                video_anchor_clean_observed_at_ms,
                episode,
            );
            let updated = episode.clone();
            sync_recent_picture_recovery_episode(stats, updated);
        }
    }

    pub(crate) fn apply_clear_transport_clean_anchor(stats: &mut XbxEngineMediaRuntimeStats) {
        stats.video_anchor_clean_epoch = None;
        stats.video_anchor_clean_observed_at_ms = None;
        stats.video_anchor_clean_source_event = None;
        stats.video_anchor_bridge_epoch = None;
        stats.video_anchor_bridge_observed_at_ms = None;
        stats.video_anchor_bridge_source_event = None;
        stats.video_anchor_bridge_rtp_timestamp = None;
        stats.recovery_displayed_idr_rtp = None;
        stats.recovery_displayed_idr_at_ms = None;
        stats.recovery_pending_displayed_idr_rtp = None;
        stats.recovery_fresh_anchor_recovered_at_ms = None;
    }

    pub(super) fn next_picture_recovery_transition_observation_id(
        stats: &mut XbxEngineMediaRuntimeStats,
    ) -> u64 {
        stats.picture_recovery_transition_observation_count = stats
            .picture_recovery_transition_observation_count
            .saturating_add(1);
        stats.picture_recovery_transition_observation_count
    }

    pub(super) fn next_picture_recovery_blocker_observation_id(
        stats: &mut XbxEngineMediaRuntimeStats,
    ) -> u64 {
        stats.picture_recovery_blocker_observation_count = stats
            .picture_recovery_blocker_observation_count
            .saturating_add(1);
        stats.picture_recovery_blocker_observation_count
    }

    pub(super) fn next_video_ingress_termination_observation_id(
        stats: &mut XbxEngineMediaRuntimeStats,
    ) -> u64 {
        stats.video_ingress_termination_observation_count = stats
            .video_ingress_termination_observation_count
            .saturating_add(1);
        stats.video_ingress_termination_observation_count
    }

    pub(super) fn next_first_frame_latency_observation_id(
        stats: &mut XbxEngineMediaRuntimeStats,
    ) -> u64 {
        stats.first_frame_latency_observation_count = stats
            .first_frame_latency_observation_count
            .saturating_add(1);
        stats.first_frame_latency_observation_count
    }
}
