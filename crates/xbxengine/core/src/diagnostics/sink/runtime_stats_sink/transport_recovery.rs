// 由 `runtime_stats_sink` 模块拆分；采集面只写事实，不驱动控制决策。

use crate::transport::rtc::recovery::runtime_state::project_recovery_escalation_context;
use crate::{XbxEnginePictureRecoveryTransitionObservation, XbxEngineVideoEscalationObservation};

use super::support::*;
use super::RuntimeStatsSink;

impl RuntimeStatsSink {
    pub(crate) fn begin_transport_recovery_episode(&self, observed_at_ms: f64) -> u64 {
        let mut next_epoch = 0u64;
        self.update(|stats| {
            next_epoch = Self::apply_begin_transport_recovery_episode(stats, observed_at_ms);
        });
        next_epoch
    }

    pub(crate) fn advance_transport_recovery_episode(&self, observed_at_ms: f64) -> u64 {
        let mut next_epoch = 0u64;
        self.update(|stats| {
            next_epoch = Self::apply_advance_transport_recovery_episode(stats, observed_at_ms);
        });
        next_epoch
    }

    pub(crate) fn complete_transport_recovery_for_lifecycle_recovering(&self, observed_at_ms: f64) {
        self.update(|stats| {
            Self::apply_complete_transport_recovery_episode(
                stats,
                observed_at_ms,
                "lifecycleRecovering",
            );
            let observation = XbxEnginePictureRecoveryTransitionObservation {
                observation_id: Self::next_picture_recovery_transition_observation_id(stats),
                episode_id: stats
                    .latest_keyframe_request_episode
                    .as_ref()
                    .map(|episode| episode.episode_id),
                recovery_epoch: Some(stats.transport_recovery_epoch),
                phase: "EpisodeClosed".to_string(),
                from_phase: Some("Decoded".to_string()),
                to_phase: "EpisodeClosed".to_string(),
                cause: Some("lifecycleRecovering".to_string()),
                detail: None,
                rtp_timestamp: None,
                frame_seq: None,
                owner_state: stats.video_owner_state.clone(),
                transport_state: Some(format!("{:?}", stats.transport_state)),
                observed_at_ms,
            };
            stats.latest_picture_recovery_transition_observation = Some(observation);
            Self::refresh_first_frame_latency_observation(stats, observed_at_ms);
        });
    }

    pub(crate) fn record_transport_clean_anchor_with_rtp(
        &self,
        observed_at_ms: f64,
        source_event: &str,
        rtp_timestamp: Option<u32>,
        frame_seq: Option<u64>,
    ) {
        self.update(|stats| {
            if source_event == "chain-clean-anchor-submitted" {
                let Some(submission_epoch) = stats.latest_clean_anchor_submission_epoch else {
                    return;
                };
                let Some(submission_episode_id) = stats.latest_clean_anchor_submission_episode_id
                else {
                    return;
                };
                let Some(submission_rtp_timestamp) =
                    stats.latest_clean_anchor_submission_rtp_timestamp
                else {
                    return;
                };
                let Some(submission_episode) =
                    find_transport_await_episode_candidate_by_id(stats, submission_episode_id)
                else {
                    return;
                };
                let current_owner_matches_submission =
                    latest_transport_recovery_keyframe_episode_id(stats).is_none_or(
                        |current_episode_id| current_episode_id == submission_episode_id,
                    );
                let fallback_commit_allowed = stats.video_anchor_bridge_epoch
                    == Some(submission_epoch)
                    && stats.video_anchor_bridge_source_event.as_deref()
                        == Some("hostVisibleAnchorPending")
                    && (frame_seq
                        .zip(submission_episode.response_frame_seq)
                        .is_some_and(|(displayed_frame_seq, response_frame_seq)| {
                            displayed_frame_seq >= response_frame_seq
                        })
                        || has_serviceable_continuation_visible_for_submission(
                            stats,
                            &submission_episode,
                            observed_at_ms,
                        ));
                if submission_epoch != stats.transport_recovery_epoch
                    || stats.video_anchor_clean_epoch == Some(submission_epoch)
                    || !current_owner_matches_submission
                    || (rtp_timestamp != Some(submission_rtp_timestamp) && !fallback_commit_allowed)
                {
                    return;
                }
            }
            Self::apply_clean_anchor_submission_fact(
                stats,
                stats.transport_recovery_epoch,
                stats.latest_clean_anchor_submission_episode_id,
                rtp_timestamp,
                observed_at_ms,
                source_event,
            );
            Self::apply_transport_clean_anchor(stats, observed_at_ms, source_event);
            let observation = XbxEnginePictureRecoveryTransitionObservation {
                observation_id: Self::next_picture_recovery_transition_observation_id(stats),
                episode_id: stats.latest_clean_anchor_submission_episode_id,
                recovery_epoch: Some(stats.transport_recovery_epoch),
                phase: "CleanAnchorCommitted".to_string(),
                from_phase: Some("Decoded".to_string()),
                to_phase: "CleanAnchorCommitted".to_string(),
                cause: Some(source_event.to_string()),
                detail: Some("mediaGate".to_string()),
                rtp_timestamp,
                frame_seq,
                owner_state: stats.video_owner_state.clone(),
                transport_state: Some(format!("{:?}", stats.transport_state)),
                observed_at_ms,
            };
            stats.latest_picture_recovery_transition_observation = Some(observation);
            Self::refresh_first_frame_latency_observation(stats, observed_at_ms);
            emit_picture_recovery_closure_probe(
                &*stats,
                "clean-anchor",
                observed_at_ms,
                stats.latest_keyframe_request_episode.as_ref(),
                stats.latest_anchor_candidate_ledger.as_ref(),
            );
        });
    }

    pub(crate) fn record_transport_clean_anchor_bridge_with_rtp(
        &self,
        observed_at_ms: f64,
        source_event: &str,
        rtp_timestamp: Option<u32>,
        frame_seq: Option<u64>,
    ) {
        self.update(|stats| {
            let Some(submission_epoch) = stats.latest_clean_anchor_submission_epoch else {
                return;
            };
            let Some(submission_episode_id) = stats.latest_clean_anchor_submission_episode_id
            else {
                return;
            };
            let Some(submission_rtp_timestamp) = stats.latest_clean_anchor_submission_rtp_timestamp
            else {
                return;
            };
            if submission_epoch != stats.transport_recovery_epoch
                || stats.video_anchor_clean_epoch == Some(submission_epoch)
            {
                return;
            }
            if latest_transport_recovery_keyframe_episode_id(stats)
                .is_some_and(|current_episode_id| current_episode_id != submission_episode_id)
            {
                return;
            }
            let Some(episode) =
                find_transport_await_episode_candidate_by_id(stats, submission_episode_id)
            else {
                return;
            };
            let displayed_submission_rtp = rtp_timestamp == Some(submission_rtp_timestamp);
            let displayed_serviceable_continuation = frame_seq
                .zip(episode.response_frame_seq)
                .is_some_and(|(displayed_frame_seq, response_frame_seq)| {
                    displayed_frame_seq >= response_frame_seq
                })
                || has_serviceable_continuation_visible_for_submission(
                    stats,
                    &episode,
                    observed_at_ms,
                );
            if !displayed_submission_rtp && !displayed_serviceable_continuation {
                return;
            }
            Self::apply_transport_clean_anchor_bridge(
                stats,
                observed_at_ms,
                source_event,
                rtp_timestamp,
            );
        });
    }

    pub(crate) fn invalidate_current_transport_clean_anchor(
        &self,
        observed_at_ms: f64,
        reason: &str,
    ) -> bool {
        let mut invalidated = false;
        self.update(|stats| {
            invalidated = Self::apply_invalidate_current_transport_clean_anchor(
                stats,
                observed_at_ms,
                reason,
            );
        });
        invalidated
    }

    pub(crate) fn record_transport_clean_anchor_submission(
        &self,
        submission_epoch: u64,
        submission_episode_id: u64,
        rtp_timestamp: u32,
        observed_at_ms: f64,
        source_event: &str,
    ) {
        self.update(|stats| {
            let submission_episode =
                find_transport_await_episode_candidate_by_id(stats, submission_episode_id);
            let episode_still_active = submission_episode.is_some();
            let current_owner_episode_id = latest_transport_recovery_keyframe_episode_id(stats);
            let current_owner_matches_submission = current_owner_episode_id
                .is_none_or(|current_episode_id| current_episode_id == submission_episode_id);
            if stats.transport_recovery_epoch == submission_epoch
                && current_owner_episode_id.is_some()
                && (!episode_still_active || !current_owner_matches_submission)
            {
                stats.latest_observation_label = Some("cleanAnchorSubmissionIgnored".to_string());
                stats.latest_observation_summary = Some(format!(
                    "reason=ownerFrameAdvanced submissionEpoch={} submissionEpisodeId={} rtpTimestamp={} observedAtMs={:.1}",
                    submission_epoch,
                    submission_episode_id,
                    rtp_timestamp,
                    observed_at_ms
                ));
                return;
            }
            Self::apply_clean_anchor_submission_fact(
                stats,
                submission_epoch,
                Some(submission_episode_id),
                Some(rtp_timestamp),
                observed_at_ms,
                source_event,
            );
            let observation = XbxEnginePictureRecoveryTransitionObservation {
                observation_id: Self::next_picture_recovery_transition_observation_id(stats),
                episode_id: Some(submission_episode_id),
                recovery_epoch: Some(stats.transport_recovery_epoch),
                phase: "CleanAnchorSubmitted".to_string(),
                from_phase: Some("Decoded".to_string()),
                to_phase: "CleanAnchorSubmitted".to_string(),
                cause: Some(source_event.to_string()),
                detail: Some("hostVisibilityPending".to_string()),
                rtp_timestamp: Some(rtp_timestamp),
                frame_seq: None,
                owner_state: stats.video_owner_state.clone(),
                transport_state: Some(format!("{:?}", stats.transport_state)),
                observed_at_ms,
            };
            stats.latest_picture_recovery_transition_observation = Some(observation);
            Self::refresh_first_frame_latency_observation(stats, observed_at_ms);
            emit_picture_recovery_closure_probe(
                &*stats,
                "clean-anchor",
                observed_at_ms,
                stats.latest_keyframe_request_episode.as_ref(),
                stats.latest_anchor_candidate_ledger.as_ref(),
            );
        });
    }

    pub(crate) fn complete_transport_recovery_after_stable_settle(&self, observed_at_ms: f64) {
        self.update(|stats| {
            Self::apply_complete_transport_recovery_episode(
                stats,
                observed_at_ms,
                "stableServingSettled",
            );
            let display_stable = XbxEnginePictureRecoveryTransitionObservation {
                observation_id: Self::next_picture_recovery_transition_observation_id(stats),
                episode_id: stats
                    .latest_keyframe_request_episode
                    .as_ref()
                    .map(|episode| episode.episode_id),
                recovery_epoch: Some(stats.transport_recovery_epoch),
                phase: "DisplayStable".to_string(),
                from_phase: Some("CleanAnchorCommitted".to_string()),
                to_phase: "DisplayStable".to_string(),
                cause: Some("stableServingSettled".to_string()),
                detail: Some("displayGate".to_string()),
                rtp_timestamp: None,
                frame_seq: None,
                owner_state: stats.video_owner_state.clone(),
                transport_state: Some(format!("{:?}", stats.transport_state)),
                observed_at_ms,
            };
            stats.latest_picture_recovery_transition_observation = Some(display_stable);
            Self::refresh_first_frame_latency_observation(stats, observed_at_ms);
        });
    }

    pub(crate) fn record_transport_command_semantic(
        &self,
        command_name: &str,
        status_name: &str,
        status_detail: Option<&str>,
        semantic_detail: Option<&str>,
        _observed_at_ms: f64,
    ) {
        self.update(|stats| {
            let mut summary = format!("command={command_name} status={status_name}");
            if let Some(detail) = status_detail {
                summary.push_str(" detail=");
                summary.push_str(detail);
            }
            if let Some(semantic) = semantic_detail {
                summary.push_str(" semantic=");
                summary.push_str(semantic);
            }
            if stats.latest_observation_label.is_none() {
                stats.latest_observation_label = Some("rtcTransportCommandSemantic".to_string());
            }
            stats.latest_observation_summary = Some(summary);
        });
    }

    pub(crate) fn record_recovery_escalation_success(
        &self,
        observation_id: u64,
        reason: String,
        action: impl Into<String>,
        observed_at_ms: f64,
        advances_recovery_epoch: bool,
    ) {
        if advances_recovery_epoch {
            self.advance_transport_recovery_episode(observed_at_ms);
        }
        let action = action.into();
        self.update(|stats| {
            let context = project_recovery_escalation_context(stats, &reason, &action);
            stats.latest_video_escalation_observation = Some(XbxEngineVideoEscalationObservation {
                observation_id,
                reason,
                action,
                recovery_stage: context.stage,
                recovery_chain_value: context.chain_value,
                recovery_failure_cost: context.failure_cost,
                recovery_window_source: context.window_source,
                observed_at_ms,
            });
            stats.transport_recovery_epoch_at_last_escalation = stats.transport_recovery_epoch;
        });
    }
}
