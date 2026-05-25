// 由 `runtime_stats_sink` 模块拆分；采集面只写事实，不驱动控制决策。

use crate::transport::rtc::recovery::keyframe_lifecycle::apply_keyframe_episode_lifecycle_field;
use crate::{XbxEngineAnchorCandidateState, XbxEngineMediaRuntimeStats};

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

    pub(crate) fn apply_transport_clean_anchor_bridge(
        stats: &mut XbxEngineMediaRuntimeStats,
        observed_at_ms: f64,
        source_event: &str,
        rtp_timestamp: Option<u32>,
    ) {
        stats.video_anchor_bridge_epoch = Some(stats.transport_recovery_epoch);
        stats.video_anchor_bridge_observed_at_ms = Some(observed_at_ms);
        stats.video_anchor_bridge_source_event = Some(source_event.to_string());
        stats.video_anchor_bridge_rtp_timestamp = rtp_timestamp;
    }

    pub(crate) fn apply_clean_anchor_submission_fact(
        stats: &mut XbxEngineMediaRuntimeStats,
        submission_epoch: u64,
        submission_episode_id: Option<u64>,
        rtp_timestamp: Option<u32>,
        observed_at_ms: f64,
        source_event: &str,
    ) {
        stats.latest_clean_anchor_submission_epoch = Some(submission_epoch);
        stats.latest_clean_anchor_submission_episode_id = submission_episode_id;
        stats.latest_clean_anchor_submission_rtp_timestamp = rtp_timestamp;
        stats.latest_clean_anchor_submission_observed_at_ms = Some(observed_at_ms);
        stats.latest_clean_anchor_submission_source_event = Some(source_event.to_string());
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

    pub(super) fn apply_invalidate_current_transport_clean_anchor(
        stats: &mut XbxEngineMediaRuntimeStats,
        observed_at_ms: f64,
        reason: &str,
    ) -> bool {
        let has_current_clean_anchor = stats.video_anchor_clean_epoch
            == Some(stats.transport_recovery_epoch)
            && stats.video_anchor_clean_source_event.as_deref()
                == Some("chain-clean-anchor-submitted");
        let has_current_submitted_candidate = stats
            .latest_anchor_candidate_ledger
            .as_ref()
            .is_some_and(|candidate| {
                candidate.recovery_epoch == stats.transport_recovery_epoch
                    && candidate.state == XbxEngineAnchorCandidateState::SubmittedCleanAnchor
                    && candidate.source_event == "chain-clean-anchor-submitted"
            });
        let mut invalidated = false;
        if has_current_clean_anchor {
            Self::apply_clear_transport_clean_anchor(stats);
            invalidated = true;
        }
        if stats.latest_clean_anchor_submission_epoch == Some(stats.transport_recovery_epoch)
            && stats.latest_clean_anchor_submission_source_event.as_deref()
                == Some("chain-clean-anchor-submitted")
        {
            stats.latest_clean_anchor_submission_epoch = None;
            stats.latest_clean_anchor_submission_episode_id = None;
            stats.latest_clean_anchor_submission_rtp_timestamp = None;
            stats.latest_clean_anchor_submission_observed_at_ms = None;
            stats.latest_clean_anchor_submission_source_event = None;
            invalidated = true;
        }
        if has_current_submitted_candidate {
            stats.latest_anchor_candidate_ledger = None;
            invalidated = true;
        }
        if let Some(episode) = stats.latest_keyframe_request_episode.as_mut() {
            if episode.request_reason.as_deref() == Some("receiverWaitingKeyframe")
                && episode.response_verdict.as_deref() == Some("cleanAnchorCommitted")
            {
                episode.status = "decoded".to_string();
                episode.response_verdict = Some("on-time".to_string());
                episode.lifecycle_phase = Some("decoded".to_string());
                episode.retired_at_ms = None;
                episode.status_detail = Some(reason.to_string());
                let updated = episode.clone();
                sync_recent_picture_recovery_episode(stats, updated);
                invalidated = true;
            }
        }
        if invalidated {
            stats.latest_observation_label = Some("cleanAnchorInvalidated".to_string());
            stats.latest_observation_summary = Some(format!(
                "reason={reason} recoveryEpoch={} observedAtMs={observed_at_ms:.1}",
                stats.transport_recovery_epoch
            ));
        }
        invalidated
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
