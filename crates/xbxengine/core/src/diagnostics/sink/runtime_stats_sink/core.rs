// 由 `runtime_stats_sink` 模块拆分；采集面只写事实，不驱动控制决策。

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use crate::diagnostics::observation_bus::{ObservationBus, ObservationEvent};
use crate::{XbxEngineMediaRuntimeStats, XbxEngineVideoRtxReinjectObservation};

use super::PictureRecoveryResponseTraceCacheEntry;
use super::RuntimeStatsSink;

impl RuntimeStatsSink {
    pub(super) const RECENT_PICTURE_RECOVERY_EPISODE_LIMIT: usize = 32;

    pub(crate) fn new(runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>) -> Self {
        Self {
            observation_bus: ObservationBus::new(runtime_stats),
            picture_recovery_response_trace_cache: Arc::new(Mutex::new(VecDeque::with_capacity(
                Self::RECENT_PICTURE_RECOVERY_EPISODE_LIMIT,
            ))),
        }
    }

    pub(super) fn update_picture_recovery_response_trace_cache(
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
}
