use std::sync::{Arc, Mutex};

use crate::diagnostics::observation_bus::{ObservationBus, ObservationEvent};
use crate::transport::rtc::recovery::runtime_state::project_recovery_escalation_context;
use crate::{
    XbxEngineAnchorCandidateFailureReason, XbxEngineAnchorCandidateLedger,
    XbxEngineAnchorCandidateState, XbxEngineFrameRecoveryObservation,
    XbxEngineH264InspectionObservation, XbxEngineKeyframeRequestEpisodeObservation,
    XbxEngineMediaRuntimeStats, XbxEngineRemoteAnswerObservation, XbxEngineRtcBuilderObservation,
    XbxEngineTwccExtensionObservation, XbxEngineTwccRemoteStreamObservation,
    XbxEngineVideoEscalationObservation, XbxEngineVideoFrameDropObservation,
    XbxEngineVideoNackObservation, XbxEngineVideoPacketGapObservation,
    XbxEngineVideoRtxReinjectObservation, XbxEngineVideoTimelineObservation,
    XbxEngineVideoTwccObservation,
};

#[derive(Clone)]
pub(crate) struct RuntimeStatsSink {
    // 统一承接 runtime stats 的发布入口，避免热路径散落字段写逻辑。
    observation_bus: ObservationBus,
}

impl RuntimeStatsSink {
    const RECENT_KEYFRAME_REQUEST_EPISODE_LIMIT: usize = 32;

    pub(crate) fn apply_begin_transport_recovery_episode(
        stats: &mut XbxEngineMediaRuntimeStats,
        observed_at_ms: f64,
    ) -> u64 {
        if stats.transport_recovery_episode_active {
            return stats.transport_recovery_epoch;
        }
        stats.transport_recovery_epoch = stats.transport_recovery_epoch.saturating_add(1);
        stats.transport_recovery_episode_active = true;
        stats.transport_recovery_episode_opened_at_ms = Some(observed_at_ms);
        stats.transport_recovery_episode_closed_at_ms = None;
        stats.transport_recovery_episode_close_reason = None;
        Self::apply_clear_transport_clean_anchor(stats);
        stats.transport_recovery_epoch
    }

    pub(crate) fn apply_advance_transport_recovery_episode(
        stats: &mut XbxEngineMediaRuntimeStats,
        observed_at_ms: f64,
    ) -> u64 {
        stats.transport_recovery_epoch = stats.transport_recovery_epoch.saturating_add(1);
        stats.transport_recovery_episode_active = true;
        stats.transport_recovery_episode_opened_at_ms = Some(observed_at_ms);
        stats.transport_recovery_episode_closed_at_ms = None;
        stats.transport_recovery_episode_close_reason = None;
        Self::apply_clear_transport_clean_anchor(stats);
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
    }

    pub(crate) fn apply_clear_transport_clean_anchor(stats: &mut XbxEngineMediaRuntimeStats) {
        stats.video_anchor_clean_epoch = None;
        stats.video_anchor_clean_observed_at_ms = None;
        stats.video_anchor_clean_source_event = None;
    }

    pub(crate) fn new(runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>) -> Self {
        Self {
            observation_bus: ObservationBus::new(runtime_stats),
        }
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

    pub(crate) fn record_keyframe_request_episode_requested(
        &self,
        episode_id: u64,
        request_reason: Option<String>,
        requested_at_ms: f64,
        deadline_at_ms: Option<f64>,
    ) {
        self.update(|stats| {
            let episode = upsert_keyframe_request_episode(
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
                    requested_at_ms,
                    sent_at_ms: None,
                    deadline_at_ms,
                    first_keyframe_packet_at_ms: None,
                    first_keyframe_decoded_at_ms: None,
                    response_rtp_timestamp: None,
                    response_frame_seq: None,
                    response_verdict: Some("pending".to_string()),
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
        });
    }

    pub(crate) fn record_keyframe_request_episode_sent(
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
            let episode = upsert_keyframe_request_episode(
                stats,
                episode_id,
                |episode| {
                    apply_keyframe_request_episode_sent(
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
                        requested_at_ms: sent_at_ms,
                        sent_at_ms: Some(sent_at_ms),
                        deadline_at_ms,
                        first_keyframe_packet_at_ms: None,
                        first_keyframe_decoded_at_ms: None,
                        response_rtp_timestamp: None,
                        response_frame_seq: None,
                        response_verdict: Some("pending".to_string()),
                    };
                    apply_keyframe_request_episode_sent(
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
        });
    }

    pub(crate) fn record_keyframe_request_episode_timeout(&self, observed_at_ms: f64) {
        self.update(|stats| {
            let mut updated_episode = None;
            if let Some(episode) = stats.latest_keyframe_request_episode.as_mut() {
                let Some(deadline_at_ms) = episode.deadline_at_ms else {
                    return;
                };
                if episode.sent_at_ms.is_none()
                    || observed_at_ms < deadline_at_ms
                    || !matches!(episode.response_verdict.as_deref(), None | Some("pending"))
                {
                    return;
                }
                episode.status = "missed".to_string();
                episode.response_verdict = Some("missed".to_string());
                stats.latest_observation_label = Some("keyframeRequestEpisodeMissed".to_string());
                stats.latest_observation_summary = Some(format!(
                    "episodeId={} deadlineAtMs={:.1} observedAtMs={:.1}",
                    episode.episode_id, deadline_at_ms, observed_at_ms
                ));
                updated_episode = Some(episode.clone());
            }
            if let Some(episode) = updated_episode {
                sync_recent_keyframe_request_episode(stats, episode);
            }
        });
    }

    pub(crate) fn record_video_rtcp_send_failure(&self, observed_at_ms: f64, reason: &str) {
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

    pub(crate) fn record_keyframe_request_episode_packet_seen(
        &self,
        observed_at_ms: f64,
        rtp_timestamp: Option<u32>,
        is_keyframe: bool,
    ) {
        if !is_keyframe {
            return;
        }
        self.update(|stats| {
            let mut updated_episode = None;
            if let Some(episode) = stats.latest_keyframe_request_episode.as_mut() {
                if episode.first_keyframe_packet_at_ms.is_none() {
                    episode.first_keyframe_packet_at_ms = Some(observed_at_ms);
                }
                if episode.response_rtp_timestamp.is_none() {
                    episode.response_rtp_timestamp = rtp_timestamp;
                }
                episode.status = "packet-seen".to_string();
                episode.response_verdict = Some(match episode.deadline_at_ms {
                    Some(deadline_at_ms) if observed_at_ms > deadline_at_ms => "late".to_string(),
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
                updated_episode = Some(episode.clone());
            }
            if let Some(episode) = updated_episode {
                sync_recent_keyframe_request_episode(stats, episode);
            }
        });
    }

    pub(crate) fn record_keyframe_request_episode_decoded(
        &self,
        observed_at_ms: f64,
        rtp_timestamp: u32,
        frame_seq: u64,
    ) {
        self.update(|stats| {
            let mut updated_episode = None;
            if let Some(episode) = stats.latest_keyframe_request_episode.as_mut() {
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
                if episode.response_verdict.as_deref() == Some("pending") {
                    episode.response_verdict = Some(match episode.deadline_at_ms {
                        Some(deadline_at_ms) if observed_at_ms > deadline_at_ms => {
                            "late".to_string()
                        }
                        Some(_) => "on-time".to_string(),
                        None => "unknown".to_string(),
                    });
                }
                stats.latest_observation_label = Some("keyframeRequestEpisodeDecoded".to_string());
                stats.latest_observation_summary = Some(format!(
                    "episodeId={} rtpTimestamp={} frameSeq={} observedAtMs={:.1}",
                    episode.episode_id, rtp_timestamp, frame_seq, observed_at_ms
                ));
                updated_episode = Some(episode.clone());
            }
            if let Some(episode) = updated_episode {
                sync_recent_keyframe_request_episode(stats, episode);
            }
        });
    }

    pub(crate) fn record_h264_inspection_observation(
        &self,
        observation: XbxEngineH264InspectionObservation,
    ) {
        self.update(|stats| {
            stats.latest_h264_inspection_observation = Some(observation);
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
            stats.latest_anchor_candidate_ledger = Some(XbxEngineAnchorCandidateLedger {
                recovery_epoch,
                frame_rtp_timestamp,
                state,
                source_event: source_event.to_string(),
                failure_reason,
                observed_at_ms,
            });
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
        });
    }

    pub(crate) fn record_transport_clean_anchor(&self, observed_at_ms: f64, source_event: &str) {
        self.update(|stats| {
            Self::apply_transport_clean_anchor(stats, observed_at_ms, source_event);
        });
    }

    pub(crate) fn complete_transport_recovery_after_stable_settle(&self, observed_at_ms: f64) {
        self.update(|stats| {
            Self::apply_complete_transport_recovery_episode(
                stats,
                observed_at_ms,
                "stableServingSettled",
            );
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

fn upsert_keyframe_request_episode(
    stats: &mut XbxEngineMediaRuntimeStats,
    episode_id: u64,
    update: impl FnOnce(&mut XbxEngineKeyframeRequestEpisodeObservation),
    create: impl FnOnce() -> XbxEngineKeyframeRequestEpisodeObservation,
) -> XbxEngineKeyframeRequestEpisodeObservation {
    if let Some(index) = stats
        .recent_keyframe_request_episodes
        .iter()
        .position(|episode| episode.episode_id == episode_id)
    {
        let episode = &mut stats.recent_keyframe_request_episodes[index];
        update(episode);
        let cloned = episode.clone();
        stats.recent_keyframe_request_episodes.remove(index);
        stats.recent_keyframe_request_episodes.push(cloned.clone());
        trim_recent_keyframe_request_episodes(stats);
        return cloned;
    }

    let mut episode = create();
    update(&mut episode);
    stats.recent_keyframe_request_episodes.push(episode.clone());
    trim_recent_keyframe_request_episodes(stats);
    episode
}

fn trim_recent_keyframe_request_episodes(stats: &mut XbxEngineMediaRuntimeStats) {
    if stats.recent_keyframe_request_episodes.len()
        <= RuntimeStatsSink::RECENT_KEYFRAME_REQUEST_EPISODE_LIMIT
    {
        return;
    }
    let overflow = stats.recent_keyframe_request_episodes.len()
        - RuntimeStatsSink::RECENT_KEYFRAME_REQUEST_EPISODE_LIMIT;
    stats.recent_keyframe_request_episodes.drain(0..overflow);
}

fn sync_recent_keyframe_request_episode(
    stats: &mut XbxEngineMediaRuntimeStats,
    episode: XbxEngineKeyframeRequestEpisodeObservation,
) {
    if let Some(index) = stats
        .recent_keyframe_request_episodes
        .iter()
        .position(|candidate| candidate.episode_id == episode.episode_id)
    {
        stats.recent_keyframe_request_episodes[index] = episode;
        return;
    }
    stats.recent_keyframe_request_episodes.push(episode);
    trim_recent_keyframe_request_episodes(stats);
}

fn apply_keyframe_request_episode_sent(
    episode: &mut XbxEngineKeyframeRequestEpisodeObservation,
    request_kind: &str,
    sent_at_ms: f64,
    deadline_at_ms: Option<f64>,
) {
    if episode.request_kind.is_none() {
        episode.request_kind = Some(request_kind.to_string());
    }
    episode.status = "sent".to_string();
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
        sink.record_transport_clean_anchor(20.0, "chain-clean-keyframe-submitted");

        let stats = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(stats.video_anchor_clean_epoch, Some(1));
        assert_eq!(stats.video_anchor_clean_observed_at_ms, Some(20.0));
        assert_eq!(
            stats.video_anchor_clean_source_event.as_deref(),
            Some("chain-clean-keyframe-submitted")
        );
        assert!(stats.transport_recovery_episode_active);
        assert_eq!(stats.transport_recovery_episode_closed_at_ms, None);
        assert_eq!(stats.transport_recovery_episode_close_reason, None);
    }

    #[test]
    fn advancing_transport_recovery_episode_clears_stale_anchor() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let sink = RuntimeStatsSink::new(runtime_stats.clone());

        sink.begin_transport_recovery_episode(10.0);
        sink.record_transport_clean_anchor(20.0, "chain-clean-keyframe-submitted");
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
        sink.record_transport_clean_anchor(20.0, "chain-clean-keyframe-submitted");
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
    fn keyframe_request_episode_packet_seen_and_decoded_resolve_verdict() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let sink = RuntimeStatsSink::new(runtime_stats.clone());

        sink.record_keyframe_request_episode_requested(
            77,
            Some("transportAwaitRecoveryKeyframe".to_string()),
            100.0,
            None,
        );
        sink.record_keyframe_request_episode_sent("pli", 120.0, Some(200.0));
        sink.record_keyframe_request_episode_packet_seen(150.0, Some(123456789), true);
        sink.record_keyframe_request_episode_decoded(160.0, 123456789, 42);

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
        assert_eq!(
            stats.latest_observation_label.as_deref(),
            Some("keyframeRequestEpisodeDecoded")
        );
    }

    #[test]
    fn keyframe_request_episode_timeout_marks_missed_when_no_response_arrives() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let sink = RuntimeStatsSink::new(runtime_stats.clone());

        sink.record_keyframe_request_episode_requested(
            88,
            Some("transportAwaitRecoveryKeyframe".to_string()),
            100.0,
            None,
        );
        sink.record_keyframe_request_episode_sent("control", 120.0, Some(200.0));
        sink.record_keyframe_request_episode_timeout(199.0);

        {
            let stats = runtime_stats.lock().expect("runtime stats lock");
            let episode = stats
                .latest_keyframe_request_episode
                .as_ref()
                .expect("episode should exist");
            assert_eq!(episode.status, "sent");
            assert_eq!(episode.response_verdict.as_deref(), Some("pending"));
        }

        sink.record_keyframe_request_episode_timeout(200.0);

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
}
