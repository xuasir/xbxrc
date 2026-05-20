//! 媒体 `XbxEngineMediaRuntimeStats` 的统一写入入口（sink）。
//! RFC：采集面只承载事实；诊断映射在 `diagnostics` / `trace_projection`，不得反向驱动控制决策。

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use crate::diagnostics::observation_bus::{ObservationBus, ObservationEvent};
use crate::transport::rtc::recovery::keyframe_lifecycle::apply_keyframe_episode_lifecycle_field;
use crate::transport::rtc::recovery::runtime_state::project_recovery_escalation_context;
use crate::{
    XbxEngineAnchorCandidateFailureReason, XbxEngineAnchorCandidateLedger,
    XbxEngineAnchorCandidateState, XbxEngineFirstFrameLatencyObservation,
    XbxEngineFrameRecoveryObservation, XbxEngineH264InspectionObservation,
    XbxEngineKeyframeRequestEpisodeObservation, XbxEngineMediaRuntimeStats,
    XbxEnginePictureRecoveryBlockerObservation, XbxEnginePictureRecoveryTransitionObservation,
    XbxEngineRemoteAnswerObservation, XbxEngineRtcBuilderObservation,
    XbxEngineTwccExtensionObservation, XbxEngineTwccRemoteStreamObservation,
    XbxEngineVideoEscalationObservation, XbxEngineVideoFrameDropObservation,
    XbxEngineVideoIngressTerminationObservation, XbxEngineVideoNackObservation,
    XbxEngineVideoPacketGapObservation, XbxEngineVideoRtxReinjectObservation,
    XbxEngineVideoTimelineObservation, XbxEngineVideoTwccObservation,
};

#[derive(Clone)]
pub(crate) struct RuntimeStatsSink {
    // 统一承接 runtime stats 的发布入口，避免热路径散落字段写逻辑。
    observation_bus: ObservationBus,
    picture_recovery_response_trace_cache:
        Arc<Mutex<VecDeque<PictureRecoveryResponseTraceCacheEntry>>>,
}

#[derive(Clone, Copy, Debug, Default)]
struct PictureRecoveryResponseTraceCacheEntry {
    episode_id: u64,
    first_video_packet_sequence: Option<u16>,
    first_keyframe_packet_sequence: Option<u16>,
}

impl RuntimeStatsSink {
    const RECENT_PICTURE_RECOVERY_EPISODE_LIMIT: usize = 32;

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
        let submission_episode_id = stats.latest_clean_anchor_submission_episode_id;
        if let Some(episode) = stats
            .latest_keyframe_request_episode
            .as_mut()
            .filter(|episode| {
                submission_episode_id.is_none_or(|episode_id| episode.episode_id == episode_id)
            })
        {
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
    }

    fn apply_invalidate_current_transport_clean_anchor(
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

    fn next_picture_recovery_transition_observation_id(
        stats: &mut XbxEngineMediaRuntimeStats,
    ) -> u64 {
        stats.picture_recovery_transition_observation_count = stats
            .picture_recovery_transition_observation_count
            .saturating_add(1);
        stats.picture_recovery_transition_observation_count
    }

    fn next_picture_recovery_blocker_observation_id(stats: &mut XbxEngineMediaRuntimeStats) -> u64 {
        stats.picture_recovery_blocker_observation_count = stats
            .picture_recovery_blocker_observation_count
            .saturating_add(1);
        stats.picture_recovery_blocker_observation_count
    }

    fn next_video_ingress_termination_observation_id(
        stats: &mut XbxEngineMediaRuntimeStats,
    ) -> u64 {
        stats.video_ingress_termination_observation_count = stats
            .video_ingress_termination_observation_count
            .saturating_add(1);
        stats.video_ingress_termination_observation_count
    }

    fn next_first_frame_latency_observation_id(stats: &mut XbxEngineMediaRuntimeStats) -> u64 {
        stats.first_frame_latency_observation_count = stats
            .first_frame_latency_observation_count
            .saturating_add(1);
        stats.first_frame_latency_observation_count
    }

    pub(crate) fn new(runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>) -> Self {
        Self {
            observation_bus: ObservationBus::new(runtime_stats),
            picture_recovery_response_trace_cache: Arc::new(Mutex::new(VecDeque::with_capacity(
                Self::RECENT_PICTURE_RECOVERY_EPISODE_LIMIT,
            ))),
        }
    }

    fn update_picture_recovery_response_trace_cache(
        &self,
        episode_id: u64,
        is_keyframe: bool,
        packet_sequence: Option<u16>,
    ) -> (Option<u16>, Option<u16>) {
        let Ok(mut cache) = self.picture_recovery_response_trace_cache.lock() else {
            return (
                packet_sequence,
                if is_keyframe { packet_sequence } else { None },
            );
        };
        if let Some(index) = cache
            .iter()
            .position(|entry| entry.episode_id == episode_id)
        {
            let entry = cache
                .get_mut(index)
                .expect("cache entry index should remain valid");
            if entry.first_video_packet_sequence.is_none() {
                entry.first_video_packet_sequence = packet_sequence;
            }
            if is_keyframe && entry.first_keyframe_packet_sequence.is_none() {
                entry.first_keyframe_packet_sequence = packet_sequence;
            }
            return (
                entry.first_video_packet_sequence,
                entry.first_keyframe_packet_sequence,
            );
        }
        if cache.len() >= Self::RECENT_PICTURE_RECOVERY_EPISODE_LIMIT {
            cache.pop_front();
        }
        let entry = PictureRecoveryResponseTraceCacheEntry {
            episode_id,
            first_video_packet_sequence: packet_sequence,
            first_keyframe_packet_sequence: if is_keyframe { packet_sequence } else { None },
        };
        let first_video_packet_sequence = entry.first_video_packet_sequence;
        let first_keyframe_packet_sequence = entry.first_keyframe_packet_sequence;
        cache.push_back(entry);
        (first_video_packet_sequence, first_keyframe_packet_sequence)
    }

    pub(crate) fn read_shared<T>(
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        project: impl FnOnce(&XbxEngineMediaRuntimeStats) -> T,
    ) -> Option<T> {
        runtime_stats.lock().ok().map(|stats| project(&stats))
    }

    pub(crate) fn update_shared(
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        apply: impl FnOnce(&mut XbxEngineMediaRuntimeStats),
    ) {
        if let Ok(mut stats) = runtime_stats.lock() {
            apply(&mut stats);
        }
    }

    pub(crate) fn update(&self, apply: impl FnOnce(&mut XbxEngineMediaRuntimeStats)) {
        self.observation_bus.update(apply);
    }

    pub(crate) fn read<T>(
        &self,
        project: impl FnOnce(&XbxEngineMediaRuntimeStats) -> T,
    ) -> Option<T> {
        self.observation_bus.read(project)
    }

    pub(crate) fn publish(&self, event: ObservationEvent) {
        self.observation_bus.publish(event);
    }

    pub(crate) fn record_frame_arrival(&self, now_ms: f64, frame_count: u64, fps: f64) {
        self.publish(ObservationEvent::FrameArrival {
            now_ms,
            frame_count,
            fps,
        });
    }

    pub(crate) fn record_stream_dimensions(&self, width: u32, height: u32) {
        if width == 0 {
            return;
        }
        self.publish(ObservationEvent::StreamDimensions { width, height });
    }

    pub(crate) fn record_video_rtx_reinject(
        &self,
        observation: XbxEngineVideoRtxReinjectObservation,
    ) {
        self.publish(ObservationEvent::VideoRtxReinject { observation });
    }

    pub(crate) fn record_video_ingress_close_intent(&self, observed_at_ms: f64, cause: &str) {
        self.update(|stats| {
            stats.latest_video_ingress_close_intent_cause = Some(cause.to_string());
            stats.latest_video_ingress_close_intent_observed_at_ms = Some(observed_at_ms);
            stats.latest_observation_label = Some("rtcVideoIngressCloseIntent".to_string());
            stats.latest_observation_summary =
                Some(format!("cause={cause} observedAtMs={observed_at_ms:.1}"));
            Self::record_video_ingress_termination_internal(
                stats,
                observed_at_ms,
                "closeIntent",
                cause,
                None,
                Some("video-ingress"),
                None,
            );
        });
    }

    pub(crate) fn current_video_ingress_close_cause(&self) -> Option<String> {
        self.read(|stats| stats.latest_video_ingress_close_intent_cause.clone())
            .flatten()
    }

    pub(crate) fn record_feedback_target_availability(
        &self,
        observed_at_ms: f64,
        target: &str,
        state: &str,
        reason: &str,
    ) {
        self.update(|stats| {
            let changed = stats.latest_feedback_target_availability_target.as_deref()
                != Some(target)
                || stats.latest_feedback_target_availability_state.as_deref() != Some(state)
                || stats.latest_feedback_target_availability_reason.as_deref() != Some(reason);
            stats.latest_feedback_target_availability_target = Some(target.to_string());
            stats.latest_feedback_target_availability_state = Some(state.to_string());
            stats.latest_feedback_target_availability_reason = Some(reason.to_string());
            stats.latest_feedback_target_availability_observed_at_ms = Some(observed_at_ms);
            if changed {
                stats.latest_observation_label =
                    Some("feedbackTargetAvailabilityChanged".to_string());
                stats.latest_observation_summary =
                    Some(format!("target={target} state={state} reason={reason}"));
            }
        });
    }

    pub(crate) fn record_twcc_receiver_mapping_missing(
        &self,
        observed_at_ms: f64,
        media_ssrc: Option<u32>,
        pending_feedback_packets: usize,
        dropped_pending_feedback_total: u64,
    ) {
        self.record_feedback_target_availability(
            observed_at_ms,
            "videoTwccFeedback",
            "degraded",
            "twccReceiverMappingMissing",
        );
        self.update(|stats| {
            stats.latest_observation_label = Some("twccReceiverMappingMissing".to_string());
            stats.latest_observation_summary = Some(format!(
                "mediaSsrc={:?} pendingFeedbackPackets={} droppedPendingFeedbackTotal={}",
                media_ssrc, pending_feedback_packets, dropped_pending_feedback_total
            ));
        });
    }

    pub(crate) fn record_twcc_feedback_send_failure(&self, observed_at_ms: f64, reason: &str) {
        let availability_state = if reason.contains("FeedbackTargetUnavailable")
            || reason.contains("MediaSsrcUnavailable")
            || reason.contains("feedback target")
            || reason.contains("ReceiverLookupMiss")
        {
            "unavailable"
        } else {
            "degraded"
        };
        self.record_feedback_target_availability(
            observed_at_ms,
            "videoTwccFeedback",
            availability_state,
            reason,
        );
        self.update(|stats| {
            stats.latest_video_rtcp_send_failure_time_ms = Some(observed_at_ms);
            stats.latest_video_rtcp_send_failure_reason = Some(reason.to_string());
            stats.latest_observation_label = Some("rtcTwccFeedbackSendFailed".to_string());
            stats.latest_observation_summary = Some(format!(
                "twcc feedback send failed at {:.1} reason={reason}",
                observed_at_ms
            ));
        });
    }

    pub(crate) fn record_picture_recovery_blocker(
        &self,
        observed_at_ms: f64,
        gate: &str,
        blocker_kind: &str,
        severity: &str,
        frame_rtp_timestamp: Option<u32>,
        frame_seq: Option<u64>,
    ) {
        self.update(|stats| {
            let (first_observed_at_ms, count) = stats
                .latest_picture_recovery_blocker_observation
                .as_ref()
                .filter(|observation| {
                    observation.episode_id
                        == stats
                            .latest_keyframe_request_episode
                            .as_ref()
                            .map(|episode| episode.episode_id)
                        && observation.recovery_epoch == Some(stats.transport_recovery_epoch)
                        && observation.gate == gate
                        && observation.blocker_kind == blocker_kind
                })
                .map(|observation| {
                    (
                        observation.first_observed_at_ms,
                        observation.count.saturating_add(1),
                    )
                })
                .unwrap_or((observed_at_ms, 1));
            let observation = XbxEnginePictureRecoveryBlockerObservation {
                observation_id: Self::next_picture_recovery_blocker_observation_id(stats),
                episode_id: stats
                    .latest_keyframe_request_episode
                    .as_ref()
                    .map(|episode| episode.episode_id),
                recovery_epoch: Some(stats.transport_recovery_epoch),
                gate: gate.to_string(),
                blocker_kind: blocker_kind.to_string(),
                severity: severity.to_string(),
                first_observed_at_ms,
                observed_at_ms,
                count,
                frame_rtp_timestamp,
                frame_seq,
                owner_state: stats.video_owner_state.clone(),
                transport_state: Some(format!("{:?}", stats.transport_state)),
            };
            stats.latest_picture_recovery_blocker_observation = Some(observation);
        });
    }

    fn record_video_ingress_termination_internal(
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

    fn refresh_first_frame_latency_observation(
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

    pub(crate) fn record_picture_recovery_episode_requested(
        &self,
        episode_id: u64,
        request_reason: Option<String>,
        requested_at_ms: f64,
        deadline_at_ms: Option<f64>,
    ) {
        self.update(|stats| {
            let episode_id =
                reuse_active_transport_recovery_episode_id(stats, request_reason.as_deref())
                    .unwrap_or(episode_id);
            let episode = upsert_picture_recovery_episode(
                stats,
                episode_id,
                |episode| {
                    if episode.request_reason.is_none() {
                        episode.request_reason = request_reason.clone();
                    }
                    if episode.requested_at_ms == 0.0 {
                        episode.requested_at_ms = requested_at_ms;
                    }
                    if episode.deadline_at_ms.is_none() {
                        episode.deadline_at_ms = deadline_at_ms;
                    }
                    if episode.status != "sent" {
                        episode.status = "requested".to_string();
                    }
                    if episode.response_verdict.is_none() {
                        episode.response_verdict = Some("pending".to_string());
                    }
                },
                || XbxEngineKeyframeRequestEpisodeObservation {
                    episode_id,
                    request_reason: request_reason.clone(),
                    request_kind: None,
                    status: "requested".to_string(),
                    status_detail: None,
                    requested_at_ms,
                    sent_at_ms: None,
                    deadline_at_ms,
                    transport_detail: None,
                    first_video_packet_at_ms: None,
                    first_video_packet_rtp_timestamp: None,
                    first_video_packet_is_keyframe: None,
                    first_keyframe_packet_at_ms: None,
                    first_keyframe_decoded_at_ms: None,
                    response_rtp_timestamp: None,
                    response_frame_seq: None,
                    response_verdict: Some("pending".to_string()),
                    lifecycle_phase: None,
                    retired_at_ms: None,
                },
            );
            stats.latest_keyframe_request_episode = Some(episode);
            stats.latest_observation_label = Some("keyframeRequestEpisodeRequested".to_string());
            stats.latest_observation_summary = Some(format!(
                "episodeId={} reason={} deadlineAtMs={}",
                episode_id,
                request_reason.as_deref().unwrap_or("none"),
                deadline_at_ms
                    .map(|value| format!("{value:.1}"))
                    .unwrap_or_else(|| "none".to_string())
            ));
            let observation = XbxEnginePictureRecoveryTransitionObservation {
                observation_id: Self::next_picture_recovery_transition_observation_id(stats),
                episode_id: Some(episode_id),
                recovery_epoch: Some(stats.transport_recovery_epoch),
                phase: "PliRequested".to_string(),
                from_phase: None,
                to_phase: "PliRequested".to_string(),
                cause: request_reason.clone(),
                detail: None,
                rtp_timestamp: None,
                frame_seq: None,
                owner_state: stats.video_owner_state.clone(),
                transport_state: Some(format!("{:?}", stats.transport_state)),
                observed_at_ms: requested_at_ms,
            };
            stats.latest_picture_recovery_transition_observation = Some(observation);
            Self::refresh_first_frame_latency_observation(stats, requested_at_ms);
            emit_picture_recovery_closure_probe(
                &*stats,
                "requested",
                requested_at_ms,
                stats.latest_keyframe_request_episode.as_ref(),
                None,
            );
        });
    }

    pub(crate) fn record_picture_recovery_episode_sent(
        &self,
        request_kind: &str,
        sent_at_ms: f64,
        deadline_at_ms: Option<f64>,
    ) {
        self.update(|stats| {
            let Some(episode_id) = stats
                .latest_keyframe_request_episode
                .as_ref()
                .map(|episode| episode.episode_id)
            else {
                return;
            };
            let episode = upsert_picture_recovery_episode(
                stats,
                episode_id,
                |episode| {
                    apply_picture_recovery_episode_sent(
                        episode,
                        request_kind,
                        sent_at_ms,
                        deadline_at_ms,
                    );
                },
                || {
                    let mut episode = XbxEngineKeyframeRequestEpisodeObservation {
                        episode_id,
                        request_reason: None,
                        request_kind: Some(request_kind.to_string()),
                        status: "sent".to_string(),
                        status_detail: None,
                        requested_at_ms: sent_at_ms,
                        sent_at_ms: Some(sent_at_ms),
                        deadline_at_ms,
                        transport_detail: None,
                        first_video_packet_at_ms: None,
                        first_video_packet_rtp_timestamp: None,
                        first_video_packet_is_keyframe: None,
                        first_keyframe_packet_at_ms: None,
                        first_keyframe_decoded_at_ms: None,
                        response_rtp_timestamp: None,
                        response_frame_seq: None,
                        response_verdict: Some("pending".to_string()),
                        lifecycle_phase: None,
                        retired_at_ms: None,
                    };
                    apply_picture_recovery_episode_sent(
                        &mut episode,
                        request_kind,
                        sent_at_ms,
                        deadline_at_ms,
                    );
                    episode
                },
            );
            stats.latest_keyframe_request_episode = Some(episode.clone());
            stats.latest_observation_label = Some("keyframeRequestEpisodeSent".to_string());
            stats.latest_observation_summary = Some(format!(
                "episodeId={} requestKind={} sentAtMs={:.1}",
                episode.episode_id, request_kind, sent_at_ms
            ));
            let observation = XbxEnginePictureRecoveryTransitionObservation {
                observation_id: Self::next_picture_recovery_transition_observation_id(stats),
                episode_id: Some(episode.episode_id),
                recovery_epoch: Some(stats.transport_recovery_epoch),
                phase: "PliSent".to_string(),
                from_phase: Some("PliRequested".to_string()),
                to_phase: "PliSent".to_string(),
                cause: episode.request_reason.clone(),
                detail: Some(request_kind.to_string()),
                rtp_timestamp: None,
                frame_seq: None,
                owner_state: stats.video_owner_state.clone(),
                transport_state: Some(format!("{:?}", stats.transport_state)),
                observed_at_ms: sent_at_ms,
            };
            stats.latest_picture_recovery_transition_observation = Some(observation);
            Self::refresh_first_frame_latency_observation(stats, sent_at_ms);
            emit_picture_recovery_closure_probe(&*stats, "sent", sent_at_ms, Some(&episode), None);
        });
    }

    pub(crate) fn record_picture_recovery_episode_timeout(&self, observed_at_ms: f64) {
        self.update(|stats| {
            let transport_recovery_epoch = stats.transport_recovery_epoch;
            let video_anchor_clean_epoch = stats.video_anchor_clean_epoch;
            let video_anchor_clean_observed_at_ms = stats.video_anchor_clean_observed_at_ms;
            let mut updated_episode = None;
            let mut should_probe = false;
            let mut count_episode_id = None;
            if let Some(episode) = stats.latest_keyframe_request_episode.as_mut() {
                let Some(deadline_at_ms) = episode.deadline_at_ms else {
                    return;
                };
                if episode.first_keyframe_decoded_at_ms.is_some() {
                    return;
                }
                if video_anchor_clean_epoch == Some(transport_recovery_epoch)
                    && video_anchor_clean_observed_at_ms.is_some()
                {
                    return;
                }
                if episode.sent_at_ms.is_none()
                    || observed_at_ms < deadline_at_ms
                    || matches!(
                        episode.response_verdict.as_deref(),
                        Some("transportDeferred" | "transportFailed" | "missed")
                    )
                {
                    return;
                }
                episode.status_detail = Some("deadlineExpired".to_string());
                episode.status = "missed".to_string();
                episode.response_verdict = Some("missed".to_string());
                count_episode_id = Some(episode.episode_id);
                apply_keyframe_episode_lifecycle_field(
                    transport_recovery_epoch,
                    video_anchor_clean_epoch,
                    video_anchor_clean_observed_at_ms,
                    episode,
                );
                stats.latest_observation_label = Some("keyframeRequestEpisodeMissed".to_string());
                stats.latest_observation_summary = Some(format!(
                    "episodeId={} deadlineAtMs={:.1} observedAtMs={:.1}",
                    episode.episode_id, deadline_at_ms, observed_at_ms
                ));
                updated_episode = Some(episode.clone());
                should_probe = true;
            }
            if let Some(episode_id) = count_episode_id {
                maybe_count_keyframe_sent_terminal_failure(stats, episode_id);
            }
            if let Some(episode) = updated_episode {
                sync_recent_picture_recovery_episode(stats, episode);
            }
            Self::refresh_first_frame_latency_observation(stats, observed_at_ms);
            if should_probe {
                emit_picture_recovery_closure_probe(
                    &*stats,
                    "timeout",
                    observed_at_ms,
                    stats.latest_keyframe_request_episode.as_ref(),
                    None,
                );
            }
        });
    }

    pub(crate) fn record_picture_recovery_episode_deferred(
        &self,
        observed_at_ms: f64,
        detail: &str,
    ) {
        self.update(|stats| {
            let transport_recovery_epoch = stats.transport_recovery_epoch;
            let video_anchor_clean_epoch = stats.video_anchor_clean_epoch;
            let video_anchor_clean_observed_at_ms = stats.video_anchor_clean_observed_at_ms;
            let mut updated_episode = None;
            let mut should_probe = false;
            if let Some(episode) = stats.latest_keyframe_request_episode.as_mut() {
                if episode.sent_at_ms.is_some()
                    || matches!(
                        episode.response_verdict.as_deref(),
                        Some("transportDeferred" | "transportFailed" | "missed")
                    )
                {
                    return;
                }
                episode.status_detail = Some(detail.to_string());
                episode.transport_detail = Some(detail.to_string());
                episode.status = "deferred".to_string();
                episode.response_verdict = Some("transportDeferred".to_string());
                apply_keyframe_episode_lifecycle_field(
                    transport_recovery_epoch,
                    video_anchor_clean_epoch,
                    video_anchor_clean_observed_at_ms,
                    episode,
                );
                stats.latest_observation_label = Some("keyframeRequestEpisodeDeferred".to_string());
                stats.latest_observation_summary = Some(format!(
                    "episodeId={} observedAtMs={:.1} detail={detail}",
                    episode.episode_id, observed_at_ms
                ));
                updated_episode = Some(episode.clone());
                should_probe = true;
            }
            if let Some(episode) = updated_episode {
                sync_recent_picture_recovery_episode(stats, episode);
            }
            Self::refresh_first_frame_latency_observation(stats, observed_at_ms);
            if should_probe {
                emit_picture_recovery_closure_probe(
                    &*stats,
                    "deferred",
                    observed_at_ms,
                    stats.latest_keyframe_request_episode.as_ref(),
                    None,
                );
            }
        });
    }

    pub(crate) fn record_picture_recovery_episode_failed(&self, observed_at_ms: f64, detail: &str) {
        self.update(|stats| {
            let transport_recovery_epoch = stats.transport_recovery_epoch;
            let video_anchor_clean_epoch = stats.video_anchor_clean_epoch;
            let video_anchor_clean_observed_at_ms = stats.video_anchor_clean_observed_at_ms;
            let mut updated_episode = None;
            let mut should_probe = false;
            if let Some(episode) = stats.latest_keyframe_request_episode.as_mut() {
                if episode.sent_at_ms.is_some()
                    || matches!(
                        episode.response_verdict.as_deref(),
                        Some("transportDeferred" | "transportFailed" | "missed")
                    )
                {
                    return;
                }
                episode.status_detail = Some(detail.to_string());
                episode.transport_detail = Some(detail.to_string());
                episode.status = "failed".to_string();
                episode.response_verdict = Some("transportFailed".to_string());
                apply_keyframe_episode_lifecycle_field(
                    transport_recovery_epoch,
                    video_anchor_clean_epoch,
                    video_anchor_clean_observed_at_ms,
                    episode,
                );
                stats.latest_observation_label = Some("keyframeRequestEpisodeFailed".to_string());
                stats.latest_observation_summary = Some(format!(
                    "episodeId={} observedAtMs={:.1} detail={detail}",
                    episode.episode_id, observed_at_ms
                ));
                updated_episode = Some(episode.clone());
                should_probe = true;
            }
            if let Some(episode) = updated_episode {
                sync_recent_picture_recovery_episode(stats, episode);
            }
            Self::refresh_first_frame_latency_observation(stats, observed_at_ms);
            if should_probe {
                emit_picture_recovery_closure_probe(
                    &*stats,
                    "failed",
                    observed_at_ms,
                    stats.latest_keyframe_request_episode.as_ref(),
                    None,
                );
            }
        });
    }

    #[cfg(test)]
    pub(crate) fn record_picture_recovery_episode_unsent_expired(&self, observed_at_ms: f64) {
        self.update(|stats| {
            expire_latest_picture_recovery_episode_if_unsent(stats, observed_at_ms);
        });
    }

    pub(crate) fn record_video_rtcp_send_failure(&self, observed_at_ms: f64, reason: &str) {
        let availability_state = if reason.contains("FeedbackTargetUnavailable")
            || reason.contains("MediaSsrcUnavailable")
            || reason.contains("feedback target")
            || reason.contains("ReceiverLookupMiss")
        {
            "unavailable"
        } else {
            "degraded"
        };
        self.record_feedback_target_availability(
            observed_at_ms,
            "videoRtcpFeedback",
            availability_state,
            reason,
        );
        self.update(|stats| {
            stats.latest_video_rtcp_send_failure_time_ms = Some(observed_at_ms);
            stats.latest_video_rtcp_send_failure_reason = Some(reason.to_string());
            stats.latest_observation_label = Some("rtcVideoRtcpSendFailed".to_string());
            stats.latest_observation_summary = Some(format!(
                "video rtcp send failed at {:.1} reason={reason}",
                observed_at_ms
            ));
        });
    }

    pub(crate) fn record_picture_recovery_episode_packet_seen(
        &self,
        observed_at_ms: f64,
        rtp_timestamp: Option<u32>,
        is_keyframe: bool,
        _packet_sequence: Option<u16>,
    ) {
        self.update(|stats| {
            let mut updated_episode = None;
            let mut should_probe = false;
            let mut pending_transition: Option<(u64, Option<u32>)> = None;
            let latest_h264_observation = stats.latest_h264_inspection_observation.clone();
            let latest_clean_anchor_submission_episode_id =
                stats.latest_clean_anchor_submission_episode_id;
            if let Some(episode) = stats.latest_keyframe_request_episode.as_mut() {
                // 验证1: 检查响应时间是否在合理范围内
                if let Some(sent_at_ms) = episode.sent_at_ms {
                    // 响应不能早于请求
                    if observed_at_ms < sent_at_ms {
                        return;
                    }
                    // 响应超过10秒视为旧包，不接受
                    if observed_at_ms - sent_at_ms > 10000.0 {
                        return;
                    }
                }

                if episode.first_video_packet_at_ms.is_none() {
                    episode.first_video_packet_at_ms = Some(observed_at_ms);
                }
                if episode.first_video_packet_rtp_timestamp.is_none() {
                    episode.first_video_packet_rtp_timestamp = rtp_timestamp;
                }
                if episode.first_video_packet_is_keyframe.is_none() {
                    episode.first_video_packet_is_keyframe = Some(is_keyframe);
                }
                if !is_keyframe {
                    updated_episode = Some(episode.clone());
                } else {
                    let owner_advanced = should_advance_transport_await_owner_frame(
                        episode,
                        observed_at_ms,
                        rtp_timestamp,
                        latest_h264_observation.as_ref(),
                        latest_clean_anchor_submission_episode_id,
                    );
                    if owner_advanced {
                        advance_transport_await_owner_frame(
                            episode,
                            observed_at_ms,
                            rtp_timestamp,
                            "ownerFrameAdvanced",
                        );
                    }
                    if episode.first_keyframe_packet_at_ms.is_none() || owner_advanced {
                        episode.first_keyframe_packet_at_ms = Some(observed_at_ms);
                        episode.response_rtp_timestamp = rtp_timestamp;
                        episode.response_frame_seq = None;
                    }
                    episode.status = "packet-seen".to_string();
                    episode.response_verdict = Some(match episode.deadline_at_ms {
                        Some(deadline_at_ms) if observed_at_ms > deadline_at_ms => {
                            "late".to_string()
                        }
                        Some(_) => "on-time".to_string(),
                        None => "unknown".to_string(),
                    });
                    stats.latest_observation_label =
                        Some("keyframeRequestEpisodePacketSeen".to_string());
                    stats.latest_observation_summary = Some(format!(
                        "episodeId={} rtpTimestamp={} observedAtMs={:.1}",
                        episode.episode_id,
                        rtp_timestamp
                            .map(|value| value.to_string())
                            .unwrap_or_else(|| "none".to_string()),
                        observed_at_ms
                    ));
                    pending_transition = Some((episode.episode_id, rtp_timestamp));
                    updated_episode = Some(episode.clone());
                    should_probe = true;
                }
            }
            if let Some(episode) = updated_episode {
                sync_recent_picture_recovery_episode(stats, episode);
            }
            if let Some((episode_id, rtp_timestamp)) = pending_transition {
                let observation = XbxEnginePictureRecoveryTransitionObservation {
                    observation_id: Self::next_picture_recovery_transition_observation_id(stats),
                    episode_id: Some(episode_id),
                    recovery_epoch: Some(stats.transport_recovery_epoch),
                    phase: "PacketSeen".to_string(),
                    from_phase: Some("PliSent".to_string()),
                    to_phase: "PacketSeen".to_string(),
                    cause: Some("firstKeyframeAccepted".to_string()),
                    detail: Some("packetSeen".to_string()),
                    rtp_timestamp,
                    frame_seq: None,
                    owner_state: stats.video_owner_state.clone(),
                    transport_state: Some(format!("{:?}", stats.transport_state)),
                    observed_at_ms,
                };
                stats.latest_picture_recovery_transition_observation = Some(observation);
            }
            Self::refresh_first_frame_latency_observation(stats, observed_at_ms);
            if should_probe {
                emit_picture_recovery_closure_probe(
                    &*stats,
                    "packet-seen",
                    observed_at_ms,
                    stats.latest_keyframe_request_episode.as_ref(),
                    None,
                );
            }
        });
    }

    pub(crate) fn record_picture_recovery_episode_response_observed(
        &self,
        observed_at_ms: f64,
        rtp_timestamp: Option<u32>,
        is_keyframe: bool,
        detail: &str,
        packet_sequence: Option<u16>,
        response_oos_depth_p75: Option<u16>,
        response_head_missing_active: bool,
        gap_expired_before_keyframe: bool,
    ) {
        let mut summary_first_video_packet_sequence = packet_sequence;
        let mut summary_first_keyframe_packet_sequence =
            if is_keyframe { packet_sequence } else { None };
        self.update(|stats| {
            let mut updated_episode = None;
            let mut should_probe = false;
            let mut pending_transition: Option<(u64, Option<u32>, bool, String)> = None;
            let latest_h264_observation = stats.latest_h264_inspection_observation.clone();
            let latest_clean_anchor_submission_episode_id =
                stats.latest_clean_anchor_submission_episode_id;
            if let Some(episode) = stats.latest_keyframe_request_episode.as_mut() {
                if episode.sent_at_ms.is_none()
                    || matches!(
                        episode.response_verdict.as_deref(),
                        Some("transportDeferred" | "transportFailed")
                    )
                {
                    return;
                }
                if matches!(episode.response_verdict.as_deref(), Some("missed")) && !is_keyframe {
                    return;
                }

                let mut changed = false;
                if episode.first_video_packet_at_ms.is_none() {
                    episode.first_video_packet_at_ms = Some(observed_at_ms);
                    episode.first_video_packet_rtp_timestamp = rtp_timestamp;
                    episode.first_video_packet_is_keyframe = Some(is_keyframe);
                    episode.status = "response-observed".to_string();
                    episode.status_detail = Some(detail.to_string());
                    changed = true;
                }

                let owner_advanced = is_keyframe
                    && should_advance_transport_await_owner_frame(
                        episode,
                        observed_at_ms,
                        rtp_timestamp,
                        latest_h264_observation.as_ref(),
                        latest_clean_anchor_submission_episode_id,
                    );

                if is_keyframe && (episode.first_keyframe_packet_at_ms.is_none() || owner_advanced)
                {
                    if owner_advanced {
                        advance_transport_await_owner_frame(
                            episode,
                            observed_at_ms,
                            rtp_timestamp,
                            detail,
                        );
                    } else {
                        episode.first_keyframe_packet_at_ms = Some(observed_at_ms);
                        if episode.response_rtp_timestamp.is_none() {
                            episode.response_rtp_timestamp = rtp_timestamp;
                        }
                        episode.response_frame_seq = None;
                    }
                    episode.status = "response-observed".to_string();
                    episode.status_detail = Some(detail.to_string());
                    changed = true;
                }

                if is_keyframe && matches!(episode.response_verdict.as_deref(), Some("missed")) {
                    episode.response_verdict = Some(match episode.deadline_at_ms {
                        Some(deadline_at_ms) if observed_at_ms > deadline_at_ms => {
                            "late".to_string()
                        }
                        Some(_) => "on-time".to_string(),
                        None => "unknown".to_string(),
                    });
                    episode.status_detail = Some(detail.to_string());
                    changed = true;
                }

                if !changed {
                    return;
                }

                (
                    summary_first_video_packet_sequence,
                    summary_first_keyframe_packet_sequence,
                ) = self.update_picture_recovery_response_trace_cache(
                    episode.episode_id,
                    is_keyframe,
                    packet_sequence,
                );

                stats.latest_observation_label =
                    Some("keyframeRequestEpisodeResponseObserved".to_string());
                stats.latest_observation_summary =
                    Some(format_picture_recovery_response_observed_summary(
                        episode,
                        observed_at_ms,
                        rtp_timestamp,
                        is_keyframe,
                        detail,
                        summary_first_video_packet_sequence,
                        summary_first_keyframe_packet_sequence,
                        response_oos_depth_p75,
                        response_head_missing_active,
                        gap_expired_before_keyframe,
                    ));
                pending_transition = Some((
                    episode.episode_id,
                    rtp_timestamp,
                    is_keyframe,
                    detail.to_string(),
                ));
                updated_episode = Some(episode.clone());
                should_probe = true;
            }
            if let Some(episode) = updated_episode {
                sync_recent_picture_recovery_episode(stats, episode);
            }
            if let Some((episode_id, rtp_timestamp, is_keyframe, detail)) = pending_transition {
                let observation = XbxEnginePictureRecoveryTransitionObservation {
                    observation_id: Self::next_picture_recovery_transition_observation_id(stats),
                    episode_id: Some(episode_id),
                    recovery_epoch: Some(stats.transport_recovery_epoch),
                    phase: "ResponseObserved".to_string(),
                    from_phase: Some("PliSent".to_string()),
                    to_phase: "ResponseObserved".to_string(),
                    cause: Some(detail),
                    detail: Some(if is_keyframe {
                        "idr".to_string()
                    } else {
                        "continuation".to_string()
                    }),
                    rtp_timestamp,
                    frame_seq: None,
                    owner_state: stats.video_owner_state.clone(),
                    transport_state: Some(format!("{:?}", stats.transport_state)),
                    observed_at_ms,
                };
                stats.latest_picture_recovery_transition_observation = Some(observation);
            }
            Self::refresh_first_frame_latency_observation(stats, observed_at_ms);
            if should_probe {
                emit_picture_recovery_closure_probe(
                    &*stats,
                    "response-observed",
                    observed_at_ms,
                    stats.latest_keyframe_request_episode.as_ref(),
                    None,
                );
            }
        });
    }

    pub(crate) fn record_picture_recovery_episode_decoded(
        &self,
        observed_at_ms: f64,
        rtp_timestamp: u32,
        frame_seq: u64,
    ) {
        self.update(|stats| {
            let mut updated_episode = None;
            let mut should_probe = false;
            let mut pending_transition: Option<(u64, u32, u64)> = None;
            if let Some(episode) = stats.latest_keyframe_request_episode.as_mut() {
                if request_reason_is_transport_recovery_keyframe_family(
                    episode.request_reason.as_deref(),
                )
                    && episode
                        .response_rtp_timestamp
                        .is_some_and(|owner_rtp_timestamp| owner_rtp_timestamp != rtp_timestamp)
                {
                    stats.latest_observation_label =
                        Some("keyframeRequestEpisodeDecodedIgnored".to_string());
                    stats.latest_observation_summary = Some(format!(
                        "episodeId={} ownerRtpTimestamp={} ignoredRtpTimestamp={} observedAtMs={:.1}",
                        episode.episode_id,
                        episode
                            .response_rtp_timestamp
                            .map(|value| value.to_string())
                            .unwrap_or_else(|| "none".to_string()),
                        rtp_timestamp,
                        observed_at_ms
                    ));
                    return;
                }
                if episode.first_keyframe_packet_at_ms.is_none() {
                    episode.first_keyframe_packet_at_ms = Some(observed_at_ms);
                }
                if episode.first_keyframe_decoded_at_ms.is_none() {
                    episode.first_keyframe_decoded_at_ms = Some(observed_at_ms);
                }
                episode.response_rtp_timestamp =
                    Some(episode.response_rtp_timestamp.unwrap_or(rtp_timestamp));
                episode.response_frame_seq = Some(frame_seq);
                episode.status = "decoded".to_string();
                if matches!(
                    episode.response_verdict.as_deref(),
                    Some("pending") | Some("missed")
                ) {
                    episode.response_verdict = Some(match episode.deadline_at_ms {
                        Some(deadline_at_ms) if observed_at_ms > deadline_at_ms => {
                            "late".to_string()
                        }
                        Some(_) => "on-time".to_string(),
                        None => "unknown".to_string(),
                    });
                }
                if episode.status_detail.as_deref() == Some("deadlineExpired") {
                    episode.status_detail = None;
                }
                stats.latest_observation_label = Some("keyframeRequestEpisodeDecoded".to_string());
                stats.latest_observation_summary = Some(format!(
                    "episodeId={} rtpTimestamp={} frameSeq={} observedAtMs={:.1}",
                    episode.episode_id, rtp_timestamp, frame_seq, observed_at_ms
                ));
                pending_transition = Some((episode.episode_id, rtp_timestamp, frame_seq));
                updated_episode = Some(episode.clone());
                should_probe = true;
            }
            if let Some(episode) = updated_episode {
                sync_recent_picture_recovery_episode(stats, episode);
            }
            if let Some((episode_id, rtp_timestamp, frame_seq)) = pending_transition {
                let observation = XbxEnginePictureRecoveryTransitionObservation {
                    observation_id: Self::next_picture_recovery_transition_observation_id(stats),
                    episode_id: Some(episode_id),
                    recovery_epoch: Some(stats.transport_recovery_epoch),
                    phase: "Decoded".to_string(),
                    from_phase: Some("PacketSeen".to_string()),
                    to_phase: "Decoded".to_string(),
                    cause: Some("firstKeyframeAccepted".to_string()),
                    detail: Some("decoded".to_string()),
                    rtp_timestamp: Some(rtp_timestamp),
                    frame_seq: Some(frame_seq),
                    owner_state: stats.video_owner_state.clone(),
                    transport_state: Some(format!("{:?}", stats.transport_state)),
                    observed_at_ms,
                };
                stats.latest_picture_recovery_transition_observation = Some(observation);
            }
            Self::refresh_first_frame_latency_observation(stats, observed_at_ms);
            if should_probe {
                emit_picture_recovery_closure_probe(
                    &*stats,
                    "decoded",
                    observed_at_ms,
                    stats.latest_keyframe_request_episode.as_ref(),
                    None,
                );
            }
        });
    }

    pub(crate) fn record_h264_inspection_observation(
        &self,
        mut observation: XbxEngineH264InspectionObservation,
    ) {
        self.update(|stats| {
            let bump_episode_id =
                stats
                    .latest_keyframe_request_episode
                    .as_ref()
                    .and_then(|episode| {
                        if request_reason_is_transport_recovery_keyframe_family(
                            episode.request_reason.as_deref(),
                        ) && episode.sent_at_ms.is_some()
                            && matches!(
                                observation.bootstrap_reject_reason.as_deref(),
                                Some(
                                    "bootstrapMissingSps"
                                        | "bootstrapMissingPps"
                                        | "inspectionRejectInvalidSliceHeader"
                                )
                            )
                        {
                            Some(episode.episode_id)
                        } else {
                            None
                        }
                    });
            if let Some(episode_id) = bump_episode_id {
                maybe_count_keyframe_sent_terminal_failure(stats, episode_id);
            }
            let selected =
                select_picture_recovery_episode_snapshot_for_h264_inspection(stats, &observation);
            if let Some(ref episode) = selected {
                observation.bound_episode_id = Some(episode.episode_id);
                observation.bound_episode_status = Some(episode.status.clone());
                observation.bound_response_rtp_timestamp = episode.response_rtp_timestamp;
                observation.bound_recovery_epoch = Some(stats.transport_recovery_epoch);
                observation.episode_phase_at_observation = episode.lifecycle_phase.clone();
                observation.is_post_recovery_degradation = Some(
                    episode.first_keyframe_decoded_at_ms.is_some()
                        && stats.transport_recovery_episode_close_reason.as_deref()
                            == Some("stableServingSettled"),
                );
                observation.bound_as_recovery_response =
                    Some(inspection_matches_recovery_picture_recovery_response(
                        stats,
                        episode,
                        &observation,
                    ));
            } else {
                observation.bound_episode_id = None;
                observation.bound_episode_status = None;
                observation.bound_response_rtp_timestamp = None;
                observation.bound_recovery_epoch = None;
                observation.episode_phase_at_observation = None;
                observation.is_post_recovery_degradation = None;
                observation.bound_as_recovery_response = Some(false);
            }
            observation.reject_classification = classify_h264_reject(&observation);
            let summary = format_h264_inspection_summary(&observation);
            emit_picture_recovery_response_diagnosis_probe(
                &*stats,
                selected.as_ref(),
                &observation,
            );
            if let Some(classification) = observation.reject_classification.clone() {
                let (first_observed_at_ms, count) = stats
                    .latest_picture_recovery_blocker_observation
                    .as_ref()
                    .filter(|blocker| {
                        blocker.gate == "media"
                            && blocker.blocker_kind == classification
                            && blocker.episode_id
                                == selected.as_ref().map(|episode| episode.episode_id)
                            && blocker.recovery_epoch == Some(stats.transport_recovery_epoch)
                    })
                    .map(|blocker| {
                        (
                            blocker.first_observed_at_ms,
                            blocker.count.saturating_add(1),
                        )
                    })
                    .unwrap_or((observation.observed_at_ms, 1));
                stats.latest_picture_recovery_blocker_observation =
                    Some(XbxEnginePictureRecoveryBlockerObservation {
                        observation_id: Self::next_picture_recovery_blocker_observation_id(stats),
                        episode_id: selected.as_ref().map(|episode| episode.episode_id),
                        recovery_epoch: Some(stats.transport_recovery_epoch),
                        gate: "media".to_string(),
                        blocker_kind: classification,
                        severity: "warning".to_string(),
                        first_observed_at_ms,
                        observed_at_ms: observation.observed_at_ms,
                        count,
                        frame_rtp_timestamp: observation.frame_rtp_timestamp,
                        frame_seq: None,
                        owner_state: stats.video_owner_state.clone(),
                        transport_state: Some(format!("{:?}", stats.transport_state)),
                    });
            }
            stats.latest_h264_inspection_observation = Some(observation);
            stats.latest_observation_label = Some("h264InspectionObserved".to_string());
            stats.latest_observation_summary = Some(summary);
        });
    }

    pub(crate) fn record_host_video_timing(
        &self,
        host_display_interval_ms: Option<f64>,
        host_frame_age_budget_ms: Option<f64>,
    ) {
        self.publish(ObservationEvent::HostVideoTiming {
            host_display_interval_ms,
            host_frame_age_budget_ms,
        });
    }

    pub(crate) fn record_transport_metrics(
        &self,
        video_rtt_ms: Option<f64>,
        video_rtt_source: Option<String>,
        inbound_video_loss_ratio_5s: f64,
        inbound_video_loss_ratio_1s: f64,
        transport_path: Option<String>,
        transport_candidate_pair: Option<String>,
        transport_protocol: Option<String>,
        transport_address_family: Option<String>,
        inbound_video_bitrate_kbps: f64,
        inbound_primary_video_bytes_total: u64,
    ) {
        self.publish(ObservationEvent::TransportMetrics {
            video_rtt_ms,
            video_rtt_source,
            inbound_video_loss_ratio_5s,
            inbound_video_loss_ratio_1s,
            transport_path,
            transport_candidate_pair,
            transport_protocol,
            transport_address_family,
            inbound_video_bitrate_kbps,
            inbound_primary_video_bytes_total,
        });
    }

    pub(crate) fn record_rtc_builder_observation(
        &self,
        observation: XbxEngineRtcBuilderObservation,
    ) {
        self.publish(ObservationEvent::RtcBuilderConfigured { observation });
    }

    pub(crate) fn record_twcc_remote_stream_observation(
        &self,
        observation: XbxEngineTwccRemoteStreamObservation,
    ) {
        self.publish(ObservationEvent::TwccRemoteStreamBound { observation });
    }

    pub(crate) fn record_remote_answer_observation(
        &self,
        observation: XbxEngineRemoteAnswerObservation,
    ) {
        self.publish(ObservationEvent::RemoteAnswerApplied { observation });
    }

    pub(crate) fn record_twcc_inbound_extension_observation(
        &self,
        observation: XbxEngineTwccExtensionObservation,
    ) {
        self.publish(ObservationEvent::TwccInboundExtensionObserved { observation });
    }

    pub(crate) fn record_video_frame_drop(&self, observation: XbxEngineVideoFrameDropObservation) {
        self.publish(ObservationEvent::VideoFrameDrop { observation });
    }

    pub(crate) fn record_frame_recovery_observation(
        &self,
        observation: XbxEngineFrameRecoveryObservation,
    ) {
        self.publish(ObservationEvent::FrameRecovery { observation });
    }

    pub(crate) fn add_inbound_video_packet_loss_estimate(&self, packet_count: u16) {
        self.publish(ObservationEvent::InboundVideoPacketLossEstimate { packet_count });
    }

    #[allow(dead_code)]
    pub(crate) fn add_video_loss_finalized(&self, packet_count: usize) {
        self.publish(ObservationEvent::VideoLossFinalized { packet_count });
    }

    pub(crate) fn set_video_pending_missing_packets(&self, pending_count: usize) {
        self.publish(ObservationEvent::VideoPendingMissingPackets { pending_count });
    }

    pub(crate) fn record_nack_sent(&self, batch_len: usize, pending_count: usize) {
        self.publish(ObservationEvent::NackSent {
            batch_len,
            pending_count,
        });
    }

    pub(crate) fn record_latest_video_nack_observation(
        &self,
        observation: XbxEngineVideoNackObservation,
    ) {
        self.publish(ObservationEvent::LatestVideoNackObservation { observation });
    }

    pub(crate) fn record_latest_video_twcc_observation(
        &self,
        observation: XbxEngineVideoTwccObservation,
    ) {
        self.publish(ObservationEvent::LatestVideoTwccObservation { observation });
    }

    pub(crate) fn record_nack_recovered(
        &self,
        was_late: bool,
        recovery_time_ms: f64,
        pending_count: usize,
        observation: XbxEngineVideoNackObservation,
    ) {
        self.publish(ObservationEvent::NackRecovered {
            was_late,
            recovery_time_ms,
            pending_count,
            observation,
        });
    }

    pub(crate) fn record_latest_video_packet_gap(
        &self,
        observation: XbxEngineVideoPacketGapObservation,
        latest_sequence: u16,
    ) {
        self.publish(ObservationEvent::LatestVideoPacketGap {
            observation,
            latest_sequence,
        });
    }

    pub(crate) fn record_video_timeline_observation(
        &self,
        observation: XbxEngineVideoTimelineObservation,
    ) {
        self.publish(ObservationEvent::VideoTimelineObserved { observation });
    }

    pub(crate) fn record_video_receiver_observation(
        &self,
        observation: crate::XbxEngineVideoReceiverObservation,
    ) {
        self.publish(ObservationEvent::VideoReceiverObserved { observation });
    }

    pub(crate) fn record_anchor_candidate_ledger(
        &self,
        recovery_epoch: u64,
        frame_rtp_timestamp: Option<u32>,
        state: XbxEngineAnchorCandidateState,
        source_event: &str,
        failure_reason: Option<XbxEngineAnchorCandidateFailureReason>,
        observed_at_ms: f64,
    ) {
        self.update(|stats| {
            let ledger = XbxEngineAnchorCandidateLedger {
                recovery_epoch,
                frame_rtp_timestamp,
                state,
                source_event: source_event.to_string(),
                failure_reason,
                observed_at_ms,
            };
            stats.latest_anchor_candidate_ledger = Some(ledger.clone());
            let bound_episode = select_episode_snapshot_for_anchor_ledger(stats, &ledger);
            emit_picture_recovery_closure_probe(
                &*stats,
                "anchor-candidate",
                observed_at_ms,
                bound_episode.as_ref(),
                Some(&ledger),
            );
        });
    }

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

fn upsert_picture_recovery_episode(
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

fn reuse_active_transport_recovery_episode_id(
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

fn keyframe_episode_is_terminal(episode: &XbxEngineKeyframeRequestEpisodeObservation) -> bool {
    matches!(
        episode.response_verdict.as_deref(),
        Some("cleanAnchorCommitted" | "transportDeferred" | "transportFailed" | "unsentExpired")
    )
}

fn should_advance_transport_await_owner_frame(
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

fn advance_transport_await_owner_frame(
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

fn retire_transport_await_episode_for_new_recovery_epoch(
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
fn maybe_count_keyframe_sent_terminal_failure(
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

fn trim_recent_picture_recovery_episodes(stats: &mut XbxEngineMediaRuntimeStats) {
    if stats.recent_keyframe_request_episodes.len()
        <= RuntimeStatsSink::RECENT_PICTURE_RECOVERY_EPISODE_LIMIT
    {
        return;
    }
    let overflow = stats.recent_keyframe_request_episodes.len()
        - RuntimeStatsSink::RECENT_PICTURE_RECOVERY_EPISODE_LIMIT;
    stats.recent_keyframe_request_episodes.drain(0..overflow);
}

fn sync_recent_picture_recovery_episode(
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

fn format_picture_recovery_response_observed_summary(
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

fn format_h264_inspection_summary(observation: &XbxEngineH264InspectionObservation) -> String {
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

fn emit_picture_recovery_response_diagnosis_probe(
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

fn apply_picture_recovery_episode_sent(
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

fn format_optional_latency_ms(value: Option<f64>) -> String {
    value
        .map(|latency_ms| format!("{latency_ms:.1}"))
        .unwrap_or_else(|| "none".to_string())
}

fn enrich_picture_recovery_first_frame_latency_detail(
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
fn recovery_window_has_clean_anchor(stats: &XbxEngineMediaRuntimeStats) -> bool {
    stats.video_anchor_clean_observed_at_ms.is_some()
        && stats.video_anchor_clean_epoch == Some(stats.transport_recovery_epoch)
}

fn request_reason_is_transport_recovery_keyframe_family(request_reason: Option<&str>) -> bool {
    matches!(
        request_reason,
        Some("receiverWaitingKeyframe" | "transportExpiredDeadline")
    )
}

fn keyframe_episode_observability_active(
    episode: &XbxEngineKeyframeRequestEpisodeObservation,
) -> bool {
    episode.retired_at_ms.is_none()
}

fn episode_keeps_transport_recovery_family_context(
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

fn unresolved_transport_await_episode_keeps_serviceable_continuation_bridge(
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

fn transport_await_episode_matches_serviceable_continuation(
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

fn collect_keyframe_episode_candidates(
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

fn find_transport_await_episode_candidate_by_id(
    stats: &XbxEngineMediaRuntimeStats,
    episode_id: u64,
) -> Option<XbxEngineKeyframeRequestEpisodeObservation> {
    collect_keyframe_episode_candidates(stats)
        .into_iter()
        .find(|episode| {
            episode.episode_id == episode_id
                && keyframe_episode_observability_active(episode)
                && request_reason_is_transport_recovery_keyframe_family(
                    episode.request_reason.as_deref(),
                )
        })
}

fn latest_transport_recovery_keyframe_episode_id(
    stats: &XbxEngineMediaRuntimeStats,
) -> Option<u64> {
    stats
        .latest_keyframe_request_episode
        .as_ref()
        .filter(|episode| {
            keyframe_episode_observability_active(episode)
                && request_reason_is_transport_recovery_keyframe_family(
                    episode.request_reason.as_deref(),
                )
        })
        .map(|episode| episode.episode_id)
}

fn current_transport_recovery_keyframe_episode_snapshot(
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

fn current_transport_recovery_keyframe_episode_id(
    stats: &XbxEngineMediaRuntimeStats,
) -> Option<u64> {
    current_transport_recovery_keyframe_episode_snapshot(stats).map(|episode| episode.episode_id)
}

fn select_picture_recovery_episode_snapshot_for_h264_inspection(
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

fn inspection_matches_recovery_picture_recovery_response(
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

fn transport_recovery_serviceable_continuation(
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

fn inspection_matches_transport_recovery_continuation_family(
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

fn has_serviceable_continuation_visible_for_submission(
    stats: &XbxEngineMediaRuntimeStats,
    episode: &XbxEngineKeyframeRequestEpisodeObservation,
    observed_at_ms: f64,
) -> bool {
    stats
        .latest_h264_inspection_observation
        .as_ref()
        .is_some_and(|inspection| {
            inspection.bound_episode_id == Some(episode.episode_id)
                && inspection.observed_at_ms <= observed_at_ms
                && transport_await_episode_matches_serviceable_continuation(
                    stats, episode, inspection,
                )
        })
}

fn classify_h264_reject(observation: &XbxEngineH264InspectionObservation) -> Option<String> {
    if observation.continuation_verdict.as_deref() == Some("receiverLocalContinuation") {
        return Some("receiverLocalContinuation".to_string());
    }
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
    Some("outOfRecoveryContextContinuation".to_string())
}

/// 将 anchor ledger 与同帧/同 recovery 上下文的 episode 绑定，避免 probe 硬拼全局 latest。
fn select_episode_snapshot_for_anchor_ledger(
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

fn emit_picture_recovery_closure_probe(
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
    crate::xbx_log_warn!(
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

#[cfg(test)]
pub(crate) fn expire_latest_picture_recovery_episode_if_unsent(
    stats: &mut XbxEngineMediaRuntimeStats,
    observed_at_ms: f64,
) -> bool {
    let transport_recovery_epoch = stats.transport_recovery_epoch;
    let video_anchor_clean_epoch = stats.video_anchor_clean_epoch;
    let video_anchor_clean_observed_at_ms = stats.video_anchor_clean_observed_at_ms;
    let mut updated_episode = None;
    let mut latest_summary = None;
    if let Some(episode) = stats.latest_keyframe_request_episode.as_mut() {
        if episode.sent_at_ms.is_some()
            || episode.status != "requested"
            || !matches!(episode.response_verdict.as_deref(), None | Some("pending"))
        {
            return false;
        }
        episode.status_detail = Some("expiredUnsent".to_string());
        episode.status = "expired-unsent".to_string();
        episode.response_verdict = Some("unsentExpired".to_string());
        apply_keyframe_episode_lifecycle_field(
            transport_recovery_epoch,
            video_anchor_clean_epoch,
            video_anchor_clean_observed_at_ms,
            episode,
        );
        latest_summary = Some(format!(
            "episodeId={} requestedAtMs={:.1} observedAtMs={:.1}",
            episode.episode_id, episode.requested_at_ms, observed_at_ms
        ));
        updated_episode = Some(episode.clone());
    }
    if latest_summary.is_some() {
        stats.latest_observation_label = Some("keyframeRequestEpisodeUnsentExpired".to_string());
        stats.latest_observation_summary = latest_summary;
    }
    if let Some(episode) = updated_episode {
        sync_recent_picture_recovery_episode(stats, episode);
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use crate::XbxEngineMediaRuntimeStats;

    use super::RuntimeStatsSink;

    #[test]
    fn repeated_begin_transport_recovery_episode_is_idempotent() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let sink = RuntimeStatsSink::new(runtime_stats.clone());

        assert_eq!(sink.begin_transport_recovery_episode(10.0), 1);
        assert_eq!(sink.begin_transport_recovery_episode(20.0), 1);

        let stats = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(stats.transport_recovery_epoch, 1);
        assert!(stats.transport_recovery_episode_active);
        assert_eq!(stats.transport_recovery_episode_opened_at_ms, Some(10.0));
    }

    #[test]
    fn clean_anchor_keeps_transport_recovery_episode_open_until_stable_settle() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let sink = RuntimeStatsSink::new(runtime_stats.clone());

        sink.begin_transport_recovery_episode(10.0);
        sink.record_transport_clean_anchor_with_rtp(20.0, "test-clean-anchor", None, None);

        let stats = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(stats.video_anchor_clean_epoch, Some(1));
        assert_eq!(stats.video_anchor_clean_observed_at_ms, Some(20.0));
        assert_eq!(
            stats.video_anchor_clean_source_event.as_deref(),
            Some("test-clean-anchor")
        );
        assert!(stats.transport_recovery_episode_active);
        assert_eq!(stats.transport_recovery_episode_closed_at_ms, None);
        assert_eq!(stats.transport_recovery_episode_close_reason, None);
    }

    #[test]
    fn clean_anchor_sets_retired_at_ms_on_latest_keyframe_episode() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let sink = RuntimeStatsSink::new(runtime_stats.clone());

        sink.begin_transport_recovery_episode(10.0);
        sink.record_picture_recovery_episode_requested(
            7,
            Some("receiverWaitingKeyframe".to_string()),
            15.0,
            None,
        );
        sink.record_picture_recovery_episode_sent("pli", 16.0, None);
        sink.record_picture_recovery_episode_response_observed(
            17.0,
            Some(77_001),
            true,
            "firstAcceptedIdr",
            Some(1),
            None,
            false,
            false,
        );
        sink.record_transport_clean_anchor_submission(
            1,
            7,
            77_001,
            18.0,
            "chain-clean-anchor-submitted",
        );
        sink.record_transport_clean_anchor_with_rtp(
            20.0,
            "chain-clean-anchor-submitted",
            Some(77_001),
            None,
        );

        let stats = runtime_stats.lock().expect("runtime stats lock");
        let episode = stats
            .latest_keyframe_request_episode
            .as_ref()
            .expect("keyframe episode");
        assert_eq!(episode.retired_at_ms, Some(20.0));
        assert_eq!(episode.status, "succeeded");
    }

    #[test]
    fn advancing_transport_recovery_episode_clears_stale_anchor() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let sink = RuntimeStatsSink::new(runtime_stats.clone());

        sink.begin_transport_recovery_episode(10.0);
        sink.record_transport_clean_anchor_submission(
            1,
            7001,
            77_001,
            18.0,
            "chain-clean-anchor-submitted",
        );
        sink.record_transport_clean_anchor_with_rtp(
            20.0,
            "chain-clean-anchor-submitted",
            Some(77_001),
            None,
        );
        assert_eq!(sink.advance_transport_recovery_episode(30.0), 2);

        let stats = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(stats.transport_recovery_epoch, 2);
        assert!(stats.transport_recovery_episode_active);
        assert_eq!(stats.transport_recovery_episode_opened_at_ms, Some(30.0));
        assert_eq!(stats.video_anchor_clean_epoch, None);
        assert_eq!(stats.video_anchor_clean_observed_at_ms, None);
        assert_eq!(stats.video_anchor_clean_source_event, None);
    }

    #[test]
    fn lifecycle_recovering_completes_active_episode() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let sink = RuntimeStatsSink::new(runtime_stats.clone());

        sink.begin_transport_recovery_episode(10.0);
        sink.complete_transport_recovery_for_lifecycle_recovering(40.0);

        let stats = runtime_stats.lock().expect("runtime stats lock");
        assert!(!stats.transport_recovery_episode_active);
        assert_eq!(stats.transport_recovery_episode_closed_at_ms, Some(40.0));
        assert_eq!(
            stats.transport_recovery_episode_close_reason.as_deref(),
            Some("lifecycleRecovering")
        );
        assert_eq!(stats.video_anchor_clean_epoch, None);
        assert_eq!(stats.video_anchor_clean_observed_at_ms, None);
        assert_eq!(stats.video_anchor_clean_source_event, None);
    }

    #[test]
    fn stable_settle_completes_active_episode_after_clean_anchor() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let sink = RuntimeStatsSink::new(runtime_stats.clone());

        sink.begin_transport_recovery_episode(10.0);
        sink.record_transport_clean_anchor_with_rtp(20.0, "test-clean-anchor", None, None);
        sink.complete_transport_recovery_after_stable_settle(40.0);

        let stats = runtime_stats.lock().expect("runtime stats lock");
        assert!(!stats.transport_recovery_episode_active);
        assert_eq!(stats.transport_recovery_episode_closed_at_ms, Some(40.0));
        assert_eq!(
            stats.transport_recovery_episode_close_reason.as_deref(),
            Some("stableServingSettled")
        );
        assert_eq!(stats.video_anchor_clean_epoch, Some(1));
    }

    #[test]
    fn stale_epoch_clean_anchor_submission_does_not_promote_current_transport_recovery() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let sink = RuntimeStatsSink::new(runtime_stats.clone());

        sink.begin_transport_recovery_episode(10.0);
        sink.advance_transport_recovery_episode(20.0);
        sink.record_transport_clean_anchor_submission(
            1,
            7001,
            9_000,
            30.0,
            "chain-clean-anchor-submitted",
        );

        let stats = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(stats.transport_recovery_epoch, 2);
        assert_eq!(stats.video_anchor_clean_epoch, None);
        assert_eq!(stats.latest_clean_anchor_submission_epoch, Some(1));
        assert_eq!(
            stats.latest_clean_anchor_submission_rtp_timestamp,
            Some(9_000)
        );
    }

    #[test]
    fn keyframe_request_episode_packet_seen_and_decoded_resolve_verdict() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let sink = RuntimeStatsSink::new(runtime_stats.clone());

        sink.record_picture_recovery_episode_requested(
            77,
            Some("receiverWaitingKeyframe".to_string()),
            100.0,
            None,
        );
        sink.record_picture_recovery_episode_sent("pli", 120.0, Some(200.0));
        sink.record_picture_recovery_episode_packet_seen(150.0, Some(123456789), true, Some(321));
        sink.record_picture_recovery_episode_decoded(160.0, 123456789, 42);

        let stats = runtime_stats.lock().expect("runtime stats lock");
        let episode = stats
            .latest_keyframe_request_episode
            .as_ref()
            .expect("episode should exist");
        assert_eq!(episode.status, "decoded");
        assert_eq!(episode.request_kind.as_deref(), Some("pli"));
        assert_eq!(episode.sent_at_ms, Some(120.0));
        assert_eq!(episode.deadline_at_ms, Some(200.0));
        assert_eq!(episode.first_keyframe_packet_at_ms, Some(150.0));
        assert_eq!(episode.first_keyframe_decoded_at_ms, Some(160.0));
        assert_eq!(episode.response_rtp_timestamp, Some(123456789));
        assert_eq!(episode.response_frame_seq, Some(42));
        assert_eq!(episode.response_verdict.as_deref(), Some("on-time"));
        assert!(episode.transport_detail.as_deref().is_some_and(|detail| {
            detail.contains("firstFrameLatencyTrace")
                && detail.contains("controlReadyToPliSentMs=none")
                && detail.contains("pliSentToFirstIdrPacketMs=30.0")
                && detail.contains("firstIdrPacketToFirstDecodeMs=10.0")
        }));
        assert_eq!(
            stats.latest_observation_label.as_deref(),
            Some("keyframeRequestEpisodeDecoded")
        );
    }

    #[test]
    fn transport_await_refresh_reuses_same_episode_within_active_recovery() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let sink = RuntimeStatsSink::new(runtime_stats.clone());

        sink.begin_transport_recovery_episode(10.0);
        sink.record_picture_recovery_episode_requested(
            1001,
            Some("receiverWaitingKeyframe".to_string()),
            20.0,
            Some(120.0),
        );
        sink.record_picture_recovery_episode_sent("pli", 21.0, Some(121.0));
        sink.record_picture_recovery_episode_requested(
            1002,
            Some("receiverWaitingKeyframe".to_string()),
            40.0,
            Some(140.0),
        );
        sink.record_picture_recovery_episode_sent("pli", 41.0, Some(141.0));
        sink.record_picture_recovery_episode_packet_seen(55.0, Some(777_111), true, Some(88));
        sink.record_picture_recovery_episode_decoded(60.0, 777_111, 9001);

        let stats = runtime_stats.lock().expect("runtime stats lock");
        let episode = stats
            .latest_keyframe_request_episode
            .as_ref()
            .expect("episode should exist");
        assert_eq!(episode.episode_id, 1001);
        assert_eq!(episode.requested_at_ms, 20.0);
        assert_eq!(episode.sent_at_ms, Some(21.0));
        assert_eq!(episode.deadline_at_ms, Some(120.0));
        assert_eq!(episode.first_keyframe_packet_at_ms, Some(55.0));
        assert_eq!(episode.first_keyframe_decoded_at_ms, Some(60.0));
        assert_eq!(episode.response_rtp_timestamp, Some(777_111));
        assert_eq!(episode.response_frame_seq, Some(9001));
    }

    #[test]
    fn advancing_transport_recovery_retires_previous_transport_await_episode() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let sink = RuntimeStatsSink::new(runtime_stats.clone());

        sink.begin_transport_recovery_episode(10.0);
        sink.record_picture_recovery_episode_requested(
            2001,
            Some("receiverWaitingKeyframe".to_string()),
            20.0,
            None,
        );
        sink.record_picture_recovery_episode_sent("pli", 21.0, Some(121.0));
        sink.advance_transport_recovery_episode(50.0);
        sink.record_picture_recovery_episode_requested(
            2002,
            Some("receiverWaitingKeyframe".to_string()),
            60.0,
            None,
        );

        let stats = runtime_stats.lock().expect("runtime stats lock");
        let latest = stats
            .latest_keyframe_request_episode
            .as_ref()
            .expect("latest episode should exist");
        assert_eq!(latest.episode_id, 2002);
        let previous = stats
            .recent_keyframe_request_episodes
            .iter()
            .find(|episode| episode.episode_id == 2001)
            .expect("previous episode should remain in recent list");
        assert_eq!(previous.retired_at_ms, Some(50.0));
        assert_eq!(
            previous.status_detail.as_deref(),
            Some("supersededByNewRecoveryEpoch")
        );
    }

    #[test]
    fn keyframe_request_episode_response_observed_tracks_non_keyframe_then_rejected_keyframe() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let sink = RuntimeStatsSink::new(runtime_stats.clone());

        sink.record_picture_recovery_episode_requested(
            90,
            Some("receiverWaitingKeyframe".to_string()),
            100.0,
            None,
        );
        sink.record_picture_recovery_episode_sent("pli", 120.0, Some(200.0));
        sink.record_picture_recovery_episode_response_observed(
            140.0,
            Some(123),
            false,
            "firstResponseNonKeyframe",
            Some(111),
            Some(5),
            true,
            false,
        );
        sink.record_picture_recovery_episode_response_observed(
            170.0,
            Some(456),
            true,
            "bootstrapMissingSps",
            Some(222),
            Some(7),
            true,
            true,
        );

        let stats = runtime_stats.lock().expect("runtime stats lock");
        let episode = stats
            .latest_keyframe_request_episode
            .as_ref()
            .expect("episode should exist");
        assert_eq!(episode.status, "response-observed");
        assert_eq!(
            episode.status_detail.as_deref(),
            Some("bootstrapMissingSps")
        );
        assert_eq!(episode.first_video_packet_at_ms, Some(140.0));
        assert_eq!(episode.first_video_packet_rtp_timestamp, Some(123));
        assert_eq!(episode.first_video_packet_is_keyframe, Some(false));
        assert_eq!(episode.first_keyframe_packet_at_ms, Some(170.0));
        assert_eq!(episode.response_rtp_timestamp, Some(456));
        assert_eq!(episode.response_verdict.as_deref(), Some("pending"));
        assert_eq!(
            stats.latest_observation_label.as_deref(),
            Some("keyframeRequestEpisodeResponseObserved")
        );
        let summary = stats
            .latest_observation_summary
            .as_deref()
            .expect("response-observed summary");
        assert!(summary.contains("detail=bootstrapMissingSps"));
        assert!(summary.contains("sentToFirstPacketMs=50.0"));
        assert!(summary.contains("firstVideoPacketIsKeyframe=false"));
        assert!(summary.contains("firstVideoPacketSeq=111"));
        assert!(summary.contains("firstKeyframePacketSeq=222"));
        assert!(summary.contains("oosDepthP75=7"));
        assert!(summary.contains("firstKeyframeArrivalLagMs=50.0"));
        assert!(summary.contains("headMissingActive=true"));
        assert!(summary.contains("gapExpiredBeforeKeyframe=true"));
    }

    #[test]
    fn newer_keyframe_response_advances_transport_await_owner_frame() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let sink = RuntimeStatsSink::new(runtime_stats.clone());

        sink.begin_transport_recovery_episode(10.0);
        sink.record_picture_recovery_episode_requested(
            9010,
            Some("receiverWaitingKeyframe".to_string()),
            100.0,
            None,
        );
        sink.record_picture_recovery_episode_sent("pli", 120.0, Some(300.0));
        sink.record_picture_recovery_episode_response_observed(
            150.0,
            Some(10_001),
            true,
            "firstAcceptedIdr",
            Some(11),
            None,
            false,
            false,
        );
        sink.record_picture_recovery_episode_response_observed(
            180.0,
            Some(10_101),
            true,
            "ownerFrameAdvanced",
            Some(22),
            None,
            false,
            false,
        );

        let stats = runtime_stats.lock().expect("runtime stats lock");
        let episode = stats
            .latest_keyframe_request_episode
            .as_ref()
            .expect("episode should exist");
        assert_eq!(episode.response_rtp_timestamp, Some(10_101));
        assert_eq!(episode.first_keyframe_packet_at_ms, Some(180.0));
        assert_eq!(episode.first_keyframe_decoded_at_ms, None);
        assert_eq!(episode.status_detail.as_deref(), Some("ownerFrameAdvanced"));
        assert_eq!(episode.response_verdict.as_deref(), Some("pending"));
    }

    #[test]
    fn serviceable_continuation_prevents_transport_await_owner_frame_advance() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let sink = RuntimeStatsSink::new(runtime_stats.clone());

        sink.begin_transport_recovery_episode(10.0);
        sink.record_picture_recovery_episode_requested(
            9016,
            Some("receiverWaitingKeyframe".to_string()),
            100.0,
            None,
        );
        sink.record_picture_recovery_episode_sent("pli", 120.0, Some(300.0));
        sink.record_picture_recovery_episode_response_observed(
            150.0,
            Some(10_001),
            true,
            "firstAcceptedIdr",
            Some(11),
            None,
            false,
            false,
        );
        sink.record_h264_inspection_observation(crate::XbxEngineH264InspectionObservation {
            observation_id: 1,
            observed_at_ms: 170.0,
            frame_rtp_timestamp: Some(10_333),
            admission_accepted: true,
            bootstrap_ready: false,
            bootstrap_reject_reason: Some("bootstrapMissingIdr".to_string()),
            committed_sps_present: true,
            committed_pps_present: true,
            delta_continuation_ready: true,
            continuation_verdict: Some("receiverLocalContinuation".to_string()),
            ..Default::default()
        });
        sink.record_picture_recovery_episode_response_observed(
            180.0,
            Some(10_101),
            true,
            "ownerFrameAdvanced",
            Some(22),
            None,
            false,
            false,
        );

        let stats = runtime_stats.lock().expect("runtime stats lock");
        let episode = stats
            .latest_keyframe_request_episode
            .as_ref()
            .expect("episode should exist");
        assert_eq!(episode.response_rtp_timestamp, Some(10_001));
        assert_eq!(episode.first_keyframe_packet_at_ms, Some(150.0));
        assert_eq!(episode.status_detail.as_deref(), Some("firstAcceptedIdr"));
    }

    #[test]
    fn stale_owner_decoded_does_not_update_current_transport_await_episode() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let sink = RuntimeStatsSink::new(runtime_stats.clone());

        sink.begin_transport_recovery_episode(10.0);
        sink.record_picture_recovery_episode_requested(
            9011,
            Some("receiverWaitingKeyframe".to_string()),
            100.0,
            None,
        );
        sink.record_picture_recovery_episode_sent("pli", 120.0, Some(300.0));
        sink.record_picture_recovery_episode_response_observed(
            150.0,
            Some(10_001),
            true,
            "firstAcceptedIdr",
            Some(11),
            None,
            false,
            false,
        );
        sink.record_picture_recovery_episode_response_observed(
            180.0,
            Some(10_101),
            true,
            "ownerFrameAdvanced",
            Some(22),
            None,
            false,
            false,
        );
        sink.record_picture_recovery_episode_decoded(210.0, 10_001, 77);

        let stats = runtime_stats.lock().expect("runtime stats lock");
        let episode = stats
            .latest_keyframe_request_episode
            .as_ref()
            .expect("episode should exist");
        assert_eq!(episode.response_rtp_timestamp, Some(10_101));
        assert_eq!(episode.first_keyframe_decoded_at_ms, None);
        assert_eq!(
            stats.latest_observation_label.as_deref(),
            Some("keyframeRequestEpisodeDecodedIgnored")
        );
    }

    #[test]
    fn stale_owner_clean_anchor_submission_stays_pending_within_same_episode() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let sink = RuntimeStatsSink::new(runtime_stats.clone());

        sink.begin_transport_recovery_episode(10.0);
        sink.record_picture_recovery_episode_requested(
            9012,
            Some("receiverWaitingKeyframe".to_string()),
            100.0,
            None,
        );
        sink.record_picture_recovery_episode_sent("pli", 120.0, Some(300.0));
        sink.record_picture_recovery_episode_response_observed(
            150.0,
            Some(10_001),
            true,
            "firstAcceptedIdr",
            Some(11),
            None,
            false,
            false,
        );
        sink.record_picture_recovery_episode_response_observed(
            180.0,
            Some(10_101),
            true,
            "ownerFrameAdvanced",
            Some(22),
            None,
            false,
            false,
        );
        sink.record_transport_clean_anchor_submission(
            1,
            9012,
            10_001,
            220.0,
            "chain-clean-anchor-submitted",
        );

        let stats = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(stats.video_anchor_clean_epoch, None);
        assert_eq!(stats.latest_clean_anchor_submission_epoch, Some(1));
        assert_eq!(stats.latest_clean_anchor_submission_episode_id, Some(9012));
        assert_eq!(
            stats.latest_clean_anchor_submission_rtp_timestamp,
            Some(10_001)
        );
        let transition = stats
            .latest_picture_recovery_transition_observation
            .as_ref()
            .expect("clean anchor submission transition");
        assert_eq!(transition.phase, "CleanAnchorSubmitted");
        assert_eq!(transition.rtp_timestamp, Some(10_001));
    }

    #[test]
    fn clean_anchor_submission_waits_for_visible_fact_before_commit() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let sink = RuntimeStatsSink::new(runtime_stats.clone());

        sink.begin_transport_recovery_episode(10.0);
        sink.record_picture_recovery_episode_requested(
            9013,
            Some("receiverWaitingKeyframe".to_string()),
            100.0,
            None,
        );
        sink.record_picture_recovery_episode_sent("pli", 120.0, Some(300.0));
        sink.record_picture_recovery_episode_response_observed(
            150.0,
            Some(10_201),
            true,
            "firstAcceptedIdr",
            Some(11),
            None,
            false,
            false,
        );
        sink.record_picture_recovery_episode_decoded(180.0, 10_201, 77);
        sink.record_transport_clean_anchor_submission(
            1,
            9013,
            10_201,
            220.0,
            "chain-clean-anchor-submitted",
        );

        let stats = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(stats.video_anchor_clean_epoch, None);
        assert_eq!(stats.latest_clean_anchor_submission_epoch, Some(1));
        assert_eq!(
            stats.latest_clean_anchor_submission_rtp_timestamp,
            Some(10_201)
        );
        let transition = stats
            .latest_picture_recovery_transition_observation
            .as_ref()
            .expect("clean anchor submission transition");
        assert_eq!(transition.phase, "CleanAnchorSubmitted");
        assert_eq!(transition.detail.as_deref(), Some("hostVisibilityPending"));
    }

    #[test]
    fn visible_fact_does_not_commit_submission_for_advanced_episode() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let sink = RuntimeStatsSink::new(runtime_stats.clone());

        sink.begin_transport_recovery_episode(10.0);
        sink.record_picture_recovery_episode_requested(
            9014,
            Some("receiverWaitingKeyframe".to_string()),
            100.0,
            None,
        );
        sink.record_picture_recovery_episode_sent("pli", 120.0, Some(300.0));
        sink.record_picture_recovery_episode_response_observed(
            150.0,
            Some(10_301),
            true,
            "firstAcceptedIdr",
            Some(11),
            None,
            false,
            false,
        );
        sink.record_picture_recovery_episode_decoded(180.0, 10_301, 77);
        sink.record_transport_clean_anchor_submission(
            1,
            9014,
            10_301,
            220.0,
            "chain-clean-anchor-submitted",
        );
        sink.update(|stats| {
            stats.latest_keyframe_request_episode =
                Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                    episode_id: 9015,
                    request_reason: Some("receiverWaitingKeyframe".to_string()),
                    status: "waiting-response".to_string(),
                    requested_at_ms: 230.0,
                    ..Default::default()
                });
        });

        sink.record_transport_clean_anchor_with_rtp(
            240.0,
            "chain-clean-anchor-submitted",
            Some(10_301),
            None,
        );

        let stats = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(stats.video_anchor_clean_epoch, None);
        assert_eq!(stats.latest_clean_anchor_submission_episode_id, Some(9014));
        let latest_episode = stats
            .latest_keyframe_request_episode
            .as_ref()
            .expect("latest episode should exist");
        assert_eq!(latest_episode.episode_id, 9015);
        assert_eq!(latest_episode.response_verdict, None);
    }

    #[test]
    fn keyframe_request_episode_decoded_after_timeout_clears_missed_verdict() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let sink = RuntimeStatsSink::new(runtime_stats.clone());

        sink.record_picture_recovery_episode_requested(
            901,
            Some("receiverWaitingKeyframe".to_string()),
            100.0,
            None,
        );
        sink.record_picture_recovery_episode_sent("pli", 120.0, Some(200.0));
        sink.record_picture_recovery_episode_timeout(200.0);

        {
            let stats = runtime_stats.lock().expect("runtime stats lock");
            let episode = stats
                .latest_keyframe_request_episode
                .as_ref()
                .expect("episode");
            assert_eq!(episode.response_verdict.as_deref(), Some("missed"));
        }

        sink.record_picture_recovery_episode_decoded(210.0, 999_001, 1001);

        let stats = runtime_stats.lock().expect("runtime stats lock");
        let episode = stats
            .latest_keyframe_request_episode
            .as_ref()
            .expect("episode");
        assert_eq!(episode.status, "decoded");
        assert_eq!(episode.response_verdict.as_deref(), Some("late"));
        assert_eq!(episode.lifecycle_phase.as_deref(), Some("decoded"));
    }

    #[test]
    fn keyframe_request_episode_timeout_skipped_when_transport_clean_anchor_already_observed() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let sink = RuntimeStatsSink::new(runtime_stats.clone());

        sink.begin_transport_recovery_episode(10.0);
        sink.record_picture_recovery_episode_requested(
            902,
            Some("receiverWaitingKeyframe".to_string()),
            100.0,
            None,
        );
        sink.record_picture_recovery_episode_sent("pli", 120.0, Some(500.0));
        sink.record_transport_clean_anchor_with_rtp(180.0, "test-clean-anchor", None, None);

        sink.record_picture_recovery_episode_timeout(600.0);

        let stats = runtime_stats.lock().expect("runtime stats lock");
        let episode = stats
            .latest_keyframe_request_episode
            .as_ref()
            .expect("episode");
        assert_eq!(episode.status, "succeeded");
        assert_eq!(
            episode.response_verdict.as_deref(),
            Some("cleanAnchorCommitted")
        );
    }

    #[test]
    fn first_frame_latency_prefers_clean_anchor_gap_over_missing_pli_when_decode_exists() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let sink = RuntimeStatsSink::new(runtime_stats.clone());

        sink.begin_transport_recovery_episode(10.0);
        sink.record_picture_recovery_episode_requested(
            903,
            Some("receiverWaitingKeyframe".to_string()),
            100.0,
            None,
        );
        sink.record_picture_recovery_episode_packet_seen(150.0, Some(123456789), true, Some(321));
        sink.record_picture_recovery_episode_decoded(160.0, 123456789, 42);

        let stats = runtime_stats.lock().expect("runtime stats lock");
        let observation = stats
            .latest_first_frame_latency_observation
            .as_ref()
            .expect("first frame latency observation");
        assert_eq!(observation.terminal_phase.as_deref(), Some("Decoded"));
        assert_eq!(
            observation.incomplete_reason.as_deref(),
            Some("noCleanAnchorCommit")
        );
        assert_eq!(observation.control_ready_to_pli_sent_ms, None);
        assert_eq!(observation.pli_sent_to_first_idr_packet_ms, None);
        assert_eq!(observation.first_idr_packet_to_first_decode_ms, Some(10.0));
    }

    #[test]
    fn first_frame_latency_observation_records_complete_stage_breakdown() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let sink = RuntimeStatsSink::new(runtime_stats.clone());

        sink.begin_transport_recovery_episode(10.0);
        RuntimeStatsSink::update_shared(&runtime_stats, |stats| {
            stats.control_ready_at_ms = Some(100.0);
        });
        sink.record_picture_recovery_episode_requested(
            904,
            Some("receiverWaitingKeyframe".to_string()),
            110.0,
            None,
        );
        sink.record_picture_recovery_episode_sent("pli", 120.0, Some(300.0));
        sink.record_picture_recovery_episode_packet_seen(150.0, Some(456_789), true, Some(322));
        sink.record_picture_recovery_episode_decoded(165.0, 456_789, 43);
        sink.record_transport_clean_anchor_submission(
            1,
            904,
            456_789,
            170.0,
            "chain-clean-anchor-submitted",
        );
        sink.record_transport_clean_anchor_with_rtp(
            180.0,
            "chain-clean-anchor-submitted",
            Some(456_789),
            None,
        );
        sink.complete_transport_recovery_after_stable_settle(210.0);

        let stats = runtime_stats.lock().expect("runtime stats lock");
        let observation = stats
            .latest_first_frame_latency_observation
            .as_ref()
            .expect("first frame latency observation");
        assert_eq!(observation.episode_id, Some(904));
        assert_eq!(observation.recovery_epoch, Some(1));
        assert_eq!(observation.control_ready_to_pli_sent_ms, Some(20.0));
        assert_eq!(observation.pli_sent_to_first_idr_packet_ms, Some(30.0));
        assert_eq!(observation.first_idr_packet_to_first_decode_ms, Some(15.0));
        assert_eq!(
            observation.first_decode_to_clean_anchor_committed_ms,
            Some(15.0)
        );
        assert_eq!(
            observation.clean_anchor_committed_to_display_stable_ms,
            Some(30.0)
        );
        assert_eq!(observation.terminal_phase.as_deref(), Some("DisplayStable"));
        assert_eq!(observation.incomplete_reason, None);

        let episode = stats
            .latest_keyframe_request_episode
            .as_ref()
            .expect("episode");
        assert!(episode.transport_detail.as_deref().is_some_and(|detail| {
            detail.contains("firstFrameLatencyTrace")
                && detail.contains("controlReadyToPliSentMs=20.0")
                && detail.contains("pliSentToFirstIdrPacketMs=30.0")
                && detail.contains("firstIdrPacketToFirstDecodeMs=15.0")
                && detail.contains("firstDecodeToCleanAnchorCommittedMs=15.0")
                && detail.contains("cleanAnchorCommittedToDisplayStableMs=30.0")
        }));
    }

    #[test]
    fn first_frame_latency_observation_marks_no_idr_packet_when_only_pli_was_sent() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let sink = RuntimeStatsSink::new(runtime_stats.clone());

        sink.begin_transport_recovery_episode(10.0);
        RuntimeStatsSink::update_shared(&runtime_stats, |stats| {
            stats.control_ready_at_ms = Some(100.0);
        });
        sink.record_picture_recovery_episode_requested(
            905,
            Some("receiverWaitingKeyframe".to_string()),
            110.0,
            None,
        );
        sink.record_picture_recovery_episode_sent("pli", 120.0, Some(260.0));

        let stats = runtime_stats.lock().expect("runtime stats lock");
        let observation = stats
            .latest_first_frame_latency_observation
            .as_ref()
            .expect("first frame latency observation");
        assert_eq!(observation.episode_id, Some(905));
        assert_eq!(
            observation.terminal_phase.as_deref(),
            Some("WaitingResponse")
        );
        assert_eq!(
            observation.incomplete_reason.as_deref(),
            Some("noIdrPacket")
        );
        assert_eq!(observation.control_ready_to_pli_sent_ms, Some(20.0));
        assert_eq!(observation.pli_sent_to_first_idr_packet_ms, None);
        assert_eq!(observation.first_idr_packet_to_first_decode_ms, None);
        assert_eq!(observation.first_decode_to_clean_anchor_committed_ms, None);
        assert_eq!(
            observation.clean_anchor_committed_to_display_stable_ms,
            None
        );

        let episode = stats
            .latest_keyframe_request_episode
            .as_ref()
            .expect("episode");
        assert!(episode.transport_detail.as_deref().is_some_and(|detail| {
            detail.contains("firstFrameLatencyTrace")
                && detail.contains("controlReadyToPliSentMs=20.0")
                && detail.contains("pliSentToFirstIdrPacketMs=none")
        }));
    }

    #[test]
    fn first_frame_latency_observation_marks_continuation_seen_while_awaiting_idr() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let sink = RuntimeStatsSink::new(runtime_stats.clone());

        sink.begin_transport_recovery_episode(10.0);
        RuntimeStatsSink::update_shared(&runtime_stats, |stats| {
            stats.control_ready_at_ms = Some(100.0);
        });
        sink.record_picture_recovery_episode_requested(
            906,
            Some("receiverWaitingKeyframe".to_string()),
            110.0,
            None,
        );
        sink.record_picture_recovery_episode_sent("pli", 120.0, Some(320.0));
        sink.record_picture_recovery_episode_response_observed(
            155.0,
            Some(123_456),
            false,
            "continuationOnlyWhileAwaitingIdr",
            Some(333),
            Some(4),
            false,
            false,
        );

        let stats = runtime_stats.lock().expect("runtime stats lock");
        let observation = stats
            .latest_first_frame_latency_observation
            .as_ref()
            .expect("first frame latency observation");
        assert_eq!(observation.episode_id, Some(906));
        assert_eq!(
            observation.terminal_phase.as_deref(),
            Some("ContinuationSeen")
        );
        assert_eq!(
            observation.incomplete_reason.as_deref(),
            Some("continuationOnlyAwaitingIdr")
        );
        assert_eq!(observation.control_ready_to_pli_sent_ms, Some(20.0));
        assert_eq!(observation.pli_sent_to_first_idr_packet_ms, None);
        assert_eq!(observation.first_idr_packet_to_first_decode_ms, None);

        let episode = stats
            .latest_keyframe_request_episode
            .as_ref()
            .expect("episode");
        assert_eq!(episode.first_video_packet_at_ms, Some(155.0));
        assert_eq!(episode.first_video_packet_is_keyframe, Some(false));
        assert_eq!(episode.first_keyframe_packet_at_ms, None);
        assert!(episode.transport_detail.as_deref().is_some_and(|detail| {
            detail.contains("firstFrameLatencyTrace")
                && detail.contains("controlReadyToPliSentMs=20.0")
                && detail.contains("pliSentToFirstIdrPacketMs=none")
        }));
    }

    #[test]
    fn keyframe_request_episode_timeout_marks_missed_when_no_response_arrives() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let sink = RuntimeStatsSink::new(runtime_stats.clone());

        sink.record_picture_recovery_episode_requested(
            88,
            Some("receiverWaitingKeyframe".to_string()),
            100.0,
            None,
        );
        sink.record_picture_recovery_episode_sent("control", 120.0, Some(200.0));
        sink.record_picture_recovery_episode_timeout(199.0);

        {
            let stats = runtime_stats.lock().expect("runtime stats lock");
            let episode = stats
                .latest_keyframe_request_episode
                .as_ref()
                .expect("episode should exist");
            assert_eq!(episode.status, "sent");
            assert_eq!(episode.response_verdict.as_deref(), Some("pending"));
        }

        sink.record_picture_recovery_episode_timeout(200.0);

        let stats = runtime_stats.lock().expect("runtime stats lock");
        let episode = stats
            .latest_keyframe_request_episode
            .as_ref()
            .expect("episode should exist");
        assert_eq!(episode.status, "missed");
        assert_eq!(episode.response_verdict.as_deref(), Some("missed"));
        assert_eq!(
            stats.latest_observation_label.as_deref(),
            Some("keyframeRequestEpisodeMissed")
        );
    }

    #[test]
    fn keyframe_request_episode_deferred_marks_unsent_terminal() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let sink = RuntimeStatsSink::new(runtime_stats.clone());

        sink.record_picture_recovery_episode_requested(
            89,
            Some("ingressWaitKeyframe".to_string()),
            100.0,
            None,
        );
        sink.record_picture_recovery_episode_deferred(120.0, "familyInFlight:controlPending");

        let stats = runtime_stats.lock().expect("runtime stats lock");
        let episode = stats
            .latest_keyframe_request_episode
            .as_ref()
            .expect("episode should exist");
        assert_eq!(episode.status, "deferred");
        assert_eq!(
            episode.response_verdict.as_deref(),
            Some("transportDeferred")
        );
        assert_eq!(
            stats.latest_observation_label.as_deref(),
            Some("keyframeRequestEpisodeDeferred")
        );
    }

    #[test]
    fn keyframe_request_episode_unsent_expiry_marks_terminal() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let sink = RuntimeStatsSink::new(runtime_stats.clone());

        sink.record_picture_recovery_episode_requested(
            90,
            Some("ingressWaitKeyframe".to_string()),
            100.0,
            None,
        );
        sink.record_picture_recovery_episode_unsent_expired(360.0);

        let stats = runtime_stats.lock().expect("runtime stats lock");
        let episode = stats
            .latest_keyframe_request_episode
            .as_ref()
            .expect("episode should exist");
        assert_eq!(episode.status, "expired-unsent");
        assert_eq!(episode.response_verdict.as_deref(), Some("unsentExpired"));
        assert_eq!(
            stats.latest_observation_label.as_deref(),
            Some("keyframeRequestEpisodeUnsentExpired")
        );
    }

    #[test]
    fn keyframe_response_observed_keeps_lifecycle_in_sync_between_latest_and_recent() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let sink = RuntimeStatsSink::new(runtime_stats.clone());
        sink.record_picture_recovery_episode_requested(
            1,
            Some("receiverWaitingKeyframe".to_string()),
            100.0,
            None,
        );
        sink.record_picture_recovery_episode_sent("pli", 110.0, Some(500.0));
        sink.record_picture_recovery_episode_response_observed(
            120.0,
            Some(999),
            false,
            "firstResponseNonKeyframe",
            Some(44),
            Some(3),
            false,
            false,
        );
        let stats = runtime_stats.lock().expect("runtime stats lock");
        let latest = stats
            .latest_keyframe_request_episode
            .as_ref()
            .expect("latest episode");
        let recent = stats
            .recent_keyframe_request_episodes
            .iter()
            .find(|e| e.episode_id == 1)
            .expect("recent episode");
        assert_eq!(latest.lifecycle_phase, recent.lifecycle_phase);
        assert_eq!(latest.lifecycle_phase.as_deref(), Some("packetSeen"));
    }

    #[test]
    fn h264_inspection_binds_episode_when_frame_rtp_matches_response() {
        use crate::XbxEngineH264InspectionObservation;

        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let sink = RuntimeStatsSink::new(runtime_stats.clone());
        sink.record_picture_recovery_episode_requested(
            42,
            Some("receiverWaitingKeyframe".to_string()),
            100.0,
            None,
        );
        sink.record_picture_recovery_episode_sent("pli", 110.0, None);
        sink.record_picture_recovery_episode_packet_seen(120.0, Some(777), true, Some(88));
        sink.record_h264_inspection_observation(XbxEngineH264InspectionObservation {
            observation_id: 1,
            frame_rtp_timestamp: Some(777),
            nal_types: Vec::new(),
            nal_count: 0,
            vcl_nal_count: 0,
            has_inband_sps: false,
            has_inband_pps: false,
            committed_sps_present: false,
            committed_pps_present: false,
            slice_headers_valid: true,
            delta_continuation_ready: true,
            parameter_sets_changed: false,
            config_changed: false,
            is_idr: true,
            sample_width: None,
            sample_height: None,
            bootstrap_ready: true,
            bootstrap_reject_reason: None,
            admission_accepted: true,
            observed_at_ms: 125.0,
            ..Default::default()
        });
        let stats = runtime_stats.lock().expect("runtime stats lock");
        let h264 = stats
            .latest_h264_inspection_observation
            .as_ref()
            .expect("h264 observation");
        assert_eq!(h264.bound_episode_id, Some(42));
        assert!(h264.bound_as_recovery_response.unwrap_or(false));
        assert_eq!(
            stats.latest_observation_label.as_deref(),
            Some("h264InspectionObserved")
        );
        let summary = stats
            .latest_observation_summary
            .as_deref()
            .expect("h264 summary");
        assert!(summary.contains("rtpTimestamp=777"));
        assert!(summary.contains("isIdr=true"));
        assert!(summary.contains("boundEpisodeId=42"));
        assert!(summary.contains("boundAsRecoveryResponse=true"));
    }

    #[test]
    fn h264_inspection_marks_post_recovery_degradation_after_stable_settle() {
        use crate::XbxEngineH264InspectionObservation;

        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let sink = RuntimeStatsSink::new(runtime_stats.clone());

        sink.begin_transport_recovery_episode(10.0);
        sink.record_picture_recovery_episode_requested(
            43,
            Some("receiverWaitingKeyframe".to_string()),
            100.0,
            None,
        );
        sink.record_picture_recovery_episode_sent("pli", 110.0, Some(300.0));
        sink.record_picture_recovery_episode_packet_seen(140.0, Some(888), true, Some(99));
        sink.record_picture_recovery_episode_decoded(160.0, 888, 44);
        sink.complete_transport_recovery_after_stable_settle(190.0);

        sink.record_h264_inspection_observation(XbxEngineH264InspectionObservation {
            observation_id: 2,
            frame_rtp_timestamp: Some(888),
            nal_types: Vec::new(),
            nal_count: 0,
            vcl_nal_count: 0,
            has_inband_sps: false,
            has_inband_pps: false,
            committed_sps_present: true,
            committed_pps_present: true,
            slice_headers_valid: true,
            delta_continuation_ready: true,
            parameter_sets_changed: false,
            config_changed: false,
            is_idr: false,
            sample_width: None,
            sample_height: None,
            bootstrap_ready: false,
            bootstrap_reject_reason: Some("bootstrapMissingIdr".to_string()),
            continuation_verdict: Some("receiverLocalContinuation".to_string()),
            admission_accepted: true,
            observed_at_ms: 200.0,
            ..Default::default()
        });

        let stats = runtime_stats.lock().expect("runtime stats lock");
        let h264 = stats
            .latest_h264_inspection_observation
            .as_ref()
            .expect("h264 observation");
        assert_eq!(h264.bound_episode_id, Some(43));
        assert_eq!(h264.bound_recovery_epoch, Some(1));
        assert_eq!(h264.is_post_recovery_degradation, Some(true));
        assert_eq!(
            h264.reject_classification.as_deref(),
            Some("receiverLocalContinuation")
        );
    }

    #[test]
    fn h264_continuation_binds_most_progressed_active_transport_await_episode() {
        use crate::XbxEngineH264InspectionObservation;

        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let sink = RuntimeStatsSink::new(runtime_stats.clone());

        sink.begin_transport_recovery_episode(10.0);

        sink.record_picture_recovery_episode_requested(
            42,
            Some("receiverWaitingKeyframe".to_string()),
            100.0,
            Some(400.0),
        );
        sink.record_picture_recovery_episode_sent("pli", 110.0, Some(400.0));
        sink.record_picture_recovery_episode_packet_seen(140.0, Some(0x1111_0001), true, Some(12));
        sink.update(|stats| {
            stats.latest_keyframe_request_episode =
                Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                    episode_id: 84,
                    request_reason: Some("displaySupplyCritical".to_string()),
                    request_kind: Some("pli".to_string()),
                    status: "sent".to_string(),
                    status_detail: None,
                    requested_at_ms: 200.0,
                    sent_at_ms: Some(210.0),
                    deadline_at_ms: Some(500.0),
                    transport_detail: None,
                    first_video_packet_at_ms: None,
                    first_video_packet_rtp_timestamp: None,
                    first_video_packet_is_keyframe: None,
                    first_keyframe_packet_at_ms: None,
                    first_keyframe_decoded_at_ms: None,
                    response_rtp_timestamp: None,
                    response_frame_seq: None,
                    response_verdict: Some("pending".to_string()),
                    lifecycle_phase: Some("sent".to_string()),
                    retired_at_ms: None,
                    ..Default::default()
                });
        });

        sink.record_h264_inspection_observation(XbxEngineH264InspectionObservation {
            observation_id: 3,
            frame_rtp_timestamp: Some(0x1111_1001),
            nal_types: vec!["SliceLayerWithoutPartitioningNonIdr".to_string()],
            nal_count: 1,
            vcl_nal_count: 1,
            has_inband_sps: false,
            has_inband_pps: false,
            committed_sps_present: true,
            committed_pps_present: true,
            slice_headers_valid: true,
            delta_continuation_ready: true,
            parameter_sets_changed: false,
            config_changed: false,
            is_idr: false,
            sample_width: None,
            sample_height: None,
            bootstrap_ready: false,
            bootstrap_reject_reason: Some("bootstrapMissingIdr".to_string()),
            continuation_verdict: Some("receiverLocalContinuation".to_string()),
            admission_accepted: true,
            observed_at_ms: 230.0,
            ..Default::default()
        });

        let stats = runtime_stats.lock().expect("runtime stats lock");
        let h264 = stats
            .latest_h264_inspection_observation
            .as_ref()
            .expect("h264 observation");
        assert_eq!(h264.bound_episode_id, Some(42));
        assert_eq!(h264.bound_recovery_epoch, Some(1));
        assert!(h264.bound_as_recovery_response.unwrap_or(false));
        assert_eq!(
            h264.reject_classification.as_deref(),
            Some("receiverLocalContinuation")
        );
    }

    #[test]
    fn missed_transport_await_episode_keeps_serviceable_continuation_family_binding() {
        use crate::XbxEngineH264InspectionObservation;

        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let sink = RuntimeStatsSink::new(runtime_stats.clone());

        sink.begin_transport_recovery_episode(10.0);
        sink.record_picture_recovery_episode_requested(
            9101,
            Some("receiverWaitingKeyframe".to_string()),
            100.0,
            None,
        );
        sink.record_picture_recovery_episode_sent("pli", 120.0, Some(160.0));
        sink.record_picture_recovery_episode_response_observed(
            140.0,
            Some(20_001),
            true,
            "firstAcceptedIdr",
            Some(11),
            None,
            false,
            false,
        );
        sink.record_picture_recovery_episode_timeout(180.0);
        sink.record_h264_inspection_observation(XbxEngineH264InspectionObservation {
            observation_id: 4,
            frame_rtp_timestamp: Some(20_333),
            nal_types: vec!["SliceLayerWithoutPartitioningNonIdr".to_string()],
            nal_count: 1,
            vcl_nal_count: 1,
            has_inband_sps: false,
            has_inband_pps: false,
            committed_sps_present: true,
            committed_pps_present: true,
            slice_headers_valid: true,
            delta_continuation_ready: true,
            parameter_sets_changed: false,
            config_changed: false,
            is_idr: false,
            sample_width: None,
            sample_height: None,
            bootstrap_ready: false,
            bootstrap_reject_reason: Some("bootstrapMissingIdr".to_string()),
            continuation_verdict: Some("receiverLocalContinuation".to_string()),
            admission_accepted: true,
            observed_at_ms: 190.0,
            ..Default::default()
        });

        let stats = runtime_stats.lock().expect("runtime stats lock");
        let episode = stats
            .latest_keyframe_request_episode
            .as_ref()
            .expect("episode should exist");
        assert_eq!(episode.status, "missed");
        let h264 = stats
            .latest_h264_inspection_observation
            .as_ref()
            .expect("h264 observation");
        assert_eq!(h264.bound_episode_id, Some(9101));
        assert!(h264.bound_as_recovery_response.unwrap_or(false));
        assert_eq!(h264.bound_response_rtp_timestamp, Some(20_001));
        assert_eq!(
            h264.reject_classification.as_deref(),
            Some("receiverLocalContinuation")
        );
    }

    #[test]
    fn playback_recovered_keeps_unresolved_sent_transport_await_continuation_bound() {
        use crate::XbxEngineH264InspectionObservation;

        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let sink = RuntimeStatsSink::new(runtime_stats.clone());

        sink.begin_transport_recovery_episode(10.0);
        sink.record_picture_recovery_episode_requested(
            9201,
            Some("receiverWaitingKeyframe".to_string()),
            100.0,
            None,
        );
        sink.record_picture_recovery_episode_sent("pli", 120.0, Some(300.0));
        sink.update(|stats| {
            stats.recovery_playback_recovered_at_ms = Some(260.0);
            stats.recovery_playback_recovered_phase = Some("PlaybackRecovered".to_string());
        });
        sink.record_h264_inspection_observation(XbxEngineH264InspectionObservation {
            observation_id: 5,
            frame_rtp_timestamp: Some(30_333),
            nal_types: vec!["SliceLayerWithoutPartitioningNonIdr".to_string()],
            nal_count: 1,
            vcl_nal_count: 1,
            has_inband_sps: false,
            has_inband_pps: false,
            committed_sps_present: true,
            committed_pps_present: true,
            slice_headers_valid: true,
            delta_continuation_ready: true,
            parameter_sets_changed: false,
            config_changed: false,
            is_idr: false,
            sample_width: None,
            sample_height: None,
            bootstrap_ready: false,
            bootstrap_reject_reason: Some("bootstrapMissingIdr".to_string()),
            continuation_verdict: Some("receiverLocalContinuation".to_string()),
            admission_accepted: true,
            observed_at_ms: 15_200.0,
            ..Default::default()
        });

        let stats = runtime_stats.lock().expect("runtime stats lock");
        let episode = stats
            .latest_keyframe_request_episode
            .as_ref()
            .expect("episode should exist");
        assert_eq!(episode.status, "sent");
        let h264 = stats
            .latest_h264_inspection_observation
            .as_ref()
            .expect("h264 observation");
        assert_eq!(h264.bound_episode_id, Some(9201));
        assert_eq!(h264.bound_episode_status.as_deref(), Some("sent"));
        assert_eq!(h264.bound_recovery_epoch, Some(1));
        assert!(h264.bound_as_recovery_response.unwrap_or(false));
    }

    #[test]
    fn sent_transport_await_bridge_persists_after_first_serviceable_continuation_binding() {
        use crate::XbxEngineH264InspectionObservation;

        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let sink = RuntimeStatsSink::new(runtime_stats.clone());

        sink.begin_transport_recovery_episode(10.0);
        sink.record_picture_recovery_episode_requested(
            9202,
            Some("receiverWaitingKeyframe".to_string()),
            100.0,
            None,
        );
        sink.record_picture_recovery_episode_sent("pli", 120.0, Some(300.0));
        sink.update(|stats| {
            stats.latest_h264_inspection_observation = Some(XbxEngineH264InspectionObservation {
                observation_id: 6,
                frame_rtp_timestamp: Some(31_001),
                committed_sps_present: true,
                committed_pps_present: true,
                delta_continuation_ready: true,
                continuation_verdict: Some("receiverLocalContinuation".to_string()),
                bootstrap_reject_reason: Some("bootstrapMissingIdr".to_string()),
                admission_accepted: true,
                observed_at_ms: 500.0,
                bound_episode_id: Some(9202),
                bound_recovery_epoch: Some(1),
                bound_as_recovery_response: Some(true),
                ..Default::default()
            });
        });
        sink.record_h264_inspection_observation(XbxEngineH264InspectionObservation {
            observation_id: 7,
            frame_rtp_timestamp: Some(31_333),
            nal_types: vec!["SliceLayerWithoutPartitioningNonIdr".to_string()],
            nal_count: 1,
            vcl_nal_count: 1,
            has_inband_sps: false,
            has_inband_pps: false,
            committed_sps_present: true,
            committed_pps_present: true,
            slice_headers_valid: true,
            delta_continuation_ready: true,
            parameter_sets_changed: false,
            config_changed: false,
            is_idr: false,
            sample_width: None,
            sample_height: None,
            bootstrap_ready: false,
            bootstrap_reject_reason: Some("bootstrapMissingIdr".to_string()),
            continuation_verdict: Some("receiverLocalContinuation".to_string()),
            admission_accepted: true,
            observed_at_ms: 20_500.0,
            ..Default::default()
        });

        let stats = runtime_stats.lock().expect("runtime stats lock");
        let h264 = stats
            .latest_h264_inspection_observation
            .as_ref()
            .expect("h264 observation");
        assert_eq!(h264.bound_episode_id, Some(9202));
        assert_eq!(h264.bound_episode_status.as_deref(), Some("sent"));
        assert!(h264.bound_as_recovery_response.unwrap_or(false));
        assert_eq!(
            h264.reject_classification.as_deref(),
            Some("receiverLocalContinuation")
        );
    }

    #[test]
    fn transport_recovery_family_continuation_follows_latest_decoded_transport_expired_owner() {
        use crate::XbxEngineH264InspectionObservation;

        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let sink = RuntimeStatsSink::new(runtime_stats.clone());

        sink.begin_transport_recovery_episode(10.0);
        sink.record_picture_recovery_episode_requested(
            3446,
            Some("receiverWaitingKeyframe".to_string()),
            100.0,
            None,
        );
        sink.record_picture_recovery_episode_sent("pli", 120.0, Some(300.0));
        sink.record_picture_recovery_episode_packet_seen(150.0, Some(2_436_161_177), true, None);
        sink.record_picture_recovery_episode_decoded(170.0, 2_436_161_177, 1201);

        sink.record_picture_recovery_episode_requested(
            3593,
            Some("transportExpiredDeadline".to_string()),
            200.0,
            None,
        );
        sink.record_picture_recovery_episode_sent("pli", 205.0, Some(260.0));
        sink.record_picture_recovery_episode_packet_seen(210.0, Some(2_441_661_257), true, None);
        sink.record_picture_recovery_episode_decoded(211.0, 2_441_661_257, 1342);

        sink.record_h264_inspection_observation(XbxEngineH264InspectionObservation {
            observation_id: 8,
            frame_rtp_timestamp: Some(2_441_664_407),
            nal_types: vec!["SliceLayerWithoutPartitioningNonIdr".to_string()],
            nal_count: 1,
            vcl_nal_count: 1,
            has_inband_sps: false,
            has_inband_pps: false,
            committed_sps_present: true,
            committed_pps_present: true,
            slice_headers_valid: true,
            delta_continuation_ready: true,
            parameter_sets_changed: false,
            config_changed: false,
            is_idr: false,
            sample_width: None,
            sample_height: None,
            bootstrap_ready: false,
            bootstrap_reject_reason: Some("bootstrapMissingIdr".to_string()),
            continuation_verdict: Some("receiverLocalContinuation".to_string()),
            admission_accepted: true,
            observed_at_ms: 212.0,
            ..Default::default()
        });

        let stats = runtime_stats.lock().expect("runtime stats lock");
        let h264 = stats
            .latest_h264_inspection_observation
            .as_ref()
            .expect("h264 observation");
        assert_eq!(h264.bound_episode_id, Some(3593));
        assert_eq!(h264.bound_episode_status.as_deref(), Some("decoded"));
        assert_eq!(h264.bound_response_rtp_timestamp, Some(2_441_661_257));
        assert!(h264.bound_as_recovery_response.unwrap_or(false));
        let blocker = stats
            .latest_picture_recovery_blocker_observation
            .as_ref()
            .expect("blocker observation");
        assert_eq!(blocker.episode_id, Some(3593));
        assert_eq!(blocker.blocker_kind.as_str(), "receiverLocalContinuation");
    }

    #[test]
    fn transport_expired_deadline_decoded_ignores_stale_owner_rtp() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let sink = RuntimeStatsSink::new(runtime_stats.clone());

        sink.begin_transport_recovery_episode(10.0);
        sink.record_picture_recovery_episode_requested(
            9301,
            Some("transportExpiredDeadline".to_string()),
            100.0,
            None,
        );
        sink.record_picture_recovery_episode_sent("pli", 120.0, Some(300.0));
        sink.record_picture_recovery_episode_packet_seen(150.0, Some(40_001), true, None);
        sink.record_picture_recovery_episode_decoded(170.0, 50_001, 1444);

        let stats = runtime_stats.lock().expect("runtime stats lock");
        let episode = stats
            .latest_keyframe_request_episode
            .as_ref()
            .expect("episode should exist");
        assert_eq!(episode.response_rtp_timestamp, Some(40_001));
        assert_eq!(episode.first_keyframe_decoded_at_ms, None);
        assert_eq!(
            stats.latest_observation_label.as_deref(),
            Some("keyframeRequestEpisodeDecodedIgnored")
        );
    }

    #[test]
    fn rx_closed_keeps_close_intent_upstream_cause_after_other_observations() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let sink = RuntimeStatsSink::new(runtime_stats.clone());

        sink.record_video_ingress_close_intent(10.0, "rebuildPeerConnection");
        sink.record_feedback_target_availability(
            11.0,
            "videoRtcpFeedback",
            "degraded",
            "twccReceiverMappingMissing",
        );
        sink.record_video_ingress_rx_closed(12.0, None);

        let stats = runtime_stats.lock().expect("runtime stats lock");
        let termination = stats
            .latest_video_ingress_termination_observation
            .as_ref()
            .expect("termination observation");
        assert_eq!(termination.cause, "upstreamSenderDropped");
        assert_eq!(
            termination.upstream_cause.as_deref(),
            Some("rebuildPeerConnection")
        );
    }

    #[test]
    fn video_rtcp_send_failure_updates_feedback_target_availability() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let sink = RuntimeStatsSink::new(runtime_stats.clone());

        sink.record_video_rtcp_send_failure(20.0, "xbxEngineRtcVideoRtcpFeedbackTargetUnavailable");

        let stats = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(
            stats.latest_feedback_target_availability_target.as_deref(),
            Some("videoRtcpFeedback")
        );
        assert_eq!(
            stats.latest_feedback_target_availability_state.as_deref(),
            Some("unavailable")
        );
        assert_eq!(
            stats.latest_feedback_target_availability_reason.as_deref(),
            Some("xbxEngineRtcVideoRtcpFeedbackTargetUnavailable")
        );
    }
}
