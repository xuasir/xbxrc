//! 媒体 `XbxEngineMediaRuntimeStats` 的统一写入入口（sink）。
//! RFC：采集面只承载事实；诊断映射在 `diagnostics` / `trace_projection`，不得反向驱动控制决策。

use std::sync::{Arc, Mutex};

use crate::diagnostics::observation_bus::{ObservationBus, ObservationEvent};
use crate::transport::rtc::recovery::keyframe_lifecycle::apply_keyframe_episode_lifecycle_field;
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
        stats.keyframe_consecutive_sent_failures = 0;
        stats.keyframe_sent_failure_last_counted_episode_id = None;
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
            sync_recent_keyframe_request_episode(stats, updated);
        }
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
            emit_keyframe_closure_probe(
                &*stats,
                "requested",
                requested_at_ms,
                stats.latest_keyframe_request_episode.as_ref(),
                None,
            );
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
            emit_keyframe_closure_probe(&*stats, "sent", sent_at_ms, Some(&episode), None);
        });
    }

    pub(crate) fn record_keyframe_request_episode_timeout(&self, observed_at_ms: f64) {
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
                sync_recent_keyframe_request_episode(stats, episode);
            }
            if should_probe {
                emit_keyframe_closure_probe(
                    &*stats,
                    "timeout",
                    observed_at_ms,
                    stats.latest_keyframe_request_episode.as_ref(),
                    None,
                );
            }
        });
    }

    pub(crate) fn record_keyframe_request_episode_deferred(
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
                sync_recent_keyframe_request_episode(stats, episode);
            }
            if should_probe {
                emit_keyframe_closure_probe(
                    &*stats,
                    "deferred",
                    observed_at_ms,
                    stats.latest_keyframe_request_episode.as_ref(),
                    None,
                );
            }
        });
    }

    pub(crate) fn record_keyframe_request_episode_failed(&self, observed_at_ms: f64, detail: &str) {
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
                sync_recent_keyframe_request_episode(stats, episode);
            }
            if should_probe {
                emit_keyframe_closure_probe(
                    &*stats,
                    "failed",
                    observed_at_ms,
                    stats.latest_keyframe_request_episode.as_ref(),
                    None,
                );
            }
        });
    }

    #[allow(dead_code)]
    pub(crate) fn record_keyframe_request_episode_unsent_expired(&self, observed_at_ms: f64) {
        self.update(|stats| {
            expire_latest_keyframe_request_episode_if_unsent(stats, observed_at_ms);
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
        self.update(|stats| {
            let mut updated_episode = None;
            let mut should_probe = false;
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

                // 验证2: 如果已有响应RTP时间戳，检查是否匹配
                if is_keyframe {
                    if let Some(response_ts) = episode.response_rtp_timestamp {
                        if let Some(current_ts) = rtp_timestamp {
                            // 如果RTP时间戳不匹配，说明不是对这个请求的响应
                            if current_ts != response_ts {
                                return;
                            }
                        }
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
                    if episode.first_keyframe_packet_at_ms.is_none() {
                        episode.first_keyframe_packet_at_ms = Some(observed_at_ms);
                    }
                    if episode.response_rtp_timestamp.is_none() {
                        episode.response_rtp_timestamp = rtp_timestamp;
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
                    updated_episode = Some(episode.clone());
                    should_probe = true;
                }
            }
            if let Some(episode) = updated_episode {
                sync_recent_keyframe_request_episode(stats, episode);
            }
            if should_probe {
                emit_keyframe_closure_probe(
                    &*stats,
                    "packet-seen",
                    observed_at_ms,
                    stats.latest_keyframe_request_episode.as_ref(),
                    None,
                );
            }
        });
    }

    pub(crate) fn record_keyframe_request_episode_response_observed(
        &self,
        observed_at_ms: f64,
        rtp_timestamp: Option<u32>,
        is_keyframe: bool,
        detail: &str,
    ) {
        self.update(|stats| {
            let mut updated_episode = None;
            let mut should_probe = false;
            if let Some(episode) = stats.latest_keyframe_request_episode.as_mut() {
                if episode.sent_at_ms.is_none()
                    || matches!(
                        episode.response_verdict.as_deref(),
                        Some("transportDeferred" | "transportFailed" | "missed")
                    )
                {
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

                if is_keyframe && episode.first_keyframe_packet_at_ms.is_none() {
                    episode.first_keyframe_packet_at_ms = Some(observed_at_ms);
                    if episode.response_rtp_timestamp.is_none() {
                        episode.response_rtp_timestamp = rtp_timestamp;
                    }
                    episode.status = "response-observed".to_string();
                    episode.status_detail = Some(detail.to_string());
                    changed = true;
                }

                if !changed {
                    return;
                }

                stats.latest_observation_label =
                    Some("keyframeRequestEpisodeResponseObserved".to_string());
                stats.latest_observation_summary = Some(format_keyframe_response_observed_summary(
                    episode,
                    observed_at_ms,
                    rtp_timestamp,
                    is_keyframe,
                    detail,
                ));
                updated_episode = Some(episode.clone());
                should_probe = true;
            }
            if let Some(episode) = updated_episode {
                sync_recent_keyframe_request_episode(stats, episode);
            }
            if should_probe {
                emit_keyframe_closure_probe(
                    &*stats,
                    "response-observed",
                    observed_at_ms,
                    stats.latest_keyframe_request_episode.as_ref(),
                    None,
                );
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
            let mut should_probe = false;
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
                should_probe = true;
            }
            if let Some(episode) = updated_episode {
                sync_recent_keyframe_request_episode(stats, episode);
            }
            if should_probe {
                emit_keyframe_closure_probe(
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
                        if episode.request_reason.as_deref() == Some("transportAwaitRecoveryAnchor")
                            && episode.sent_at_ms.is_some()
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
            let selected = select_episode_snapshot_for_h264_inspection(stats, &observation);
            if let Some(ref episode) = selected {
                observation.bound_episode_id = Some(episode.episode_id);
                observation.bound_episode_status = Some(episode.status.clone());
                observation.bound_response_rtp_timestamp = episode.response_rtp_timestamp;
                observation.bound_as_recovery_response =
                    Some(inspection_matches_recovery_keyframe_response(
                        episode,
                        observation.frame_rtp_timestamp,
                    ));
            } else {
                observation.bound_episode_id = None;
                observation.bound_episode_status = None;
                observation.bound_response_rtp_timestamp = None;
                observation.bound_as_recovery_response = Some(false);
            }
            let summary = format_h264_inspection_summary(&observation);
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
            emit_keyframe_closure_probe(
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
        });
    }

    pub(crate) fn record_transport_clean_anchor(&self, observed_at_ms: f64, source_event: &str) {
        self.update(|stats| {
            Self::apply_transport_clean_anchor(stats, observed_at_ms, source_event);
            emit_keyframe_closure_probe(
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
    apply_keyframe_episode_lifecycle_field(
        transport_recovery_epoch,
        video_anchor_clean_epoch,
        video_anchor_clean_observed_at_ms,
        &mut episode,
    );
    stats.recent_keyframe_request_episodes.push(episode.clone());
    trim_recent_keyframe_request_episodes(stats);
    episode
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
    mut episode: XbxEngineKeyframeRequestEpisodeObservation,
) {
    let episode_id = episode.episode_id;
    apply_keyframe_episode_lifecycle_field(
        stats.transport_recovery_epoch,
        stats.video_anchor_clean_epoch,
        stats.video_anchor_clean_observed_at_ms,
        &mut episode,
    );
    if let Some(index) = stats
        .recent_keyframe_request_episodes
        .iter()
        .position(|candidate| candidate.episode_id == episode.episode_id)
    {
        stats.recent_keyframe_request_episodes[index] = episode;
    } else {
        stats.recent_keyframe_request_episodes.push(episode);
        trim_recent_keyframe_request_episodes(stats);
    }
    // `latest` 与 `recent` 曾分叉：recent 在 sync 时刷新了 lifecycle，latest 需同步，probe 才一致。
    if let Some(latest) = stats.latest_keyframe_request_episode.as_mut() {
        if latest.episode_id == episode_id {
            apply_keyframe_episode_lifecycle_field(
                stats.transport_recovery_epoch,
                stats.video_anchor_clean_epoch,
                stats.video_anchor_clean_observed_at_ms,
                latest,
            );
        }
    }
}

fn format_keyframe_response_observed_summary(
    episode: &XbxEngineKeyframeRequestEpisodeObservation,
    observed_at_ms: f64,
    rtp_timestamp: Option<u32>,
    is_keyframe: bool,
    detail: &str,
) -> String {
    let sent_to_first_packet_ms = episode
        .sent_at_ms
        .map(|sent_at_ms| (observed_at_ms - sent_at_ms).max(0.0));
    format!(
        "episodeId={} rtpTimestamp={} isKeyframe={} detail={} sentToFirstPacketMs={} firstVideoPacketAtMs={} firstVideoPacketIsKeyframe={}",
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
        episode
            .first_video_packet_is_keyframe
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_string()),
    )
}

fn format_h264_inspection_summary(observation: &XbxEngineH264InspectionObservation) -> String {
    format!(
        "rtpTimestamp={} isIdr={} bootstrapReady={} bootstrapRejectReason={} admissionAccepted={} boundEpisodeId={} boundAsRecoveryResponse={}",
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

/// 当前 recovery epoch 下是否已观测到与本 epoch 对齐的 clean anchor（控制态，非单次 probe 语义）。
fn recovery_window_has_clean_anchor(stats: &XbxEngineMediaRuntimeStats) -> bool {
    stats.video_anchor_clean_observed_at_ms.is_some()
        && stats.video_anchor_clean_epoch == Some(stats.transport_recovery_epoch)
}

fn keyframe_episode_observability_active(
    episode: &XbxEngineKeyframeRequestEpisodeObservation,
) -> bool {
    episode.retired_at_ms.is_none()
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

fn select_episode_snapshot_for_h264_inspection(
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
    const WINDOW_MS: f64 = 10_000.0;
    let mut best: Option<XbxEngineKeyframeRequestEpisodeObservation> = None;
    let mut best_delta = f64::INFINITY;
    for episode in candidates.iter().filter(|episode| {
        keyframe_episode_observability_active(episode)
            && episode.request_reason.as_deref() == Some("transportAwaitRecoveryAnchor")
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

fn inspection_matches_recovery_keyframe_response(
    episode: &XbxEngineKeyframeRequestEpisodeObservation,
    frame_rtp_timestamp: Option<u32>,
) -> bool {
    if episode.request_reason.as_deref() != Some("transportAwaitRecoveryAnchor") {
        return false;
    }
    if !matches!(
        episode.status.as_str(),
        "packet-seen" | "decoded" | "response-observed"
    ) {
        return false;
    }
    match (episode.response_rtp_timestamp, frame_rtp_timestamp) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
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
                    && episode.request_reason.as_deref() == Some("transportAwaitRecoveryAnchor")
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
            && episode.request_reason.as_deref() == Some("transportAwaitRecoveryAnchor")
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

fn emit_keyframe_closure_probe(
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

pub(crate) fn expire_latest_keyframe_request_episode_if_unsent(
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
        sync_recent_keyframe_request_episode(stats, episode);
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
        sink.record_transport_clean_anchor(20.0, "chain-clean-anchor-submitted");

        let stats = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(stats.video_anchor_clean_epoch, Some(1));
        assert_eq!(stats.video_anchor_clean_observed_at_ms, Some(20.0));
        assert_eq!(
            stats.video_anchor_clean_source_event.as_deref(),
            Some("chain-clean-anchor-submitted")
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
        sink.record_keyframe_request_episode_requested(
            7,
            Some("transportAwaitRecoveryAnchor".to_string()),
            15.0,
            None,
        );
        sink.record_keyframe_request_episode_sent("pli", 16.0, None);
        sink.record_transport_clean_anchor(20.0, "chain-clean-anchor-submitted");

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
        sink.record_transport_clean_anchor(20.0, "chain-clean-anchor-submitted");
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
        sink.record_transport_clean_anchor(20.0, "chain-clean-anchor-submitted");
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
            Some("transportAwaitRecoveryAnchor".to_string()),
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
    fn keyframe_request_episode_response_observed_tracks_non_keyframe_then_rejected_keyframe() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let sink = RuntimeStatsSink::new(runtime_stats.clone());

        sink.record_keyframe_request_episode_requested(
            90,
            Some("transportAwaitRecoveryAnchor".to_string()),
            100.0,
            None,
        );
        sink.record_keyframe_request_episode_sent("pli", 120.0, Some(200.0));
        sink.record_keyframe_request_episode_response_observed(
            140.0,
            Some(123),
            false,
            "firstResponseNonKeyframe",
        );
        sink.record_keyframe_request_episode_response_observed(
            170.0,
            Some(456),
            true,
            "bootstrapMissingSps",
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
    }

    #[test]
    fn keyframe_request_episode_timeout_marks_missed_when_no_response_arrives() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let sink = RuntimeStatsSink::new(runtime_stats.clone());

        sink.record_keyframe_request_episode_requested(
            88,
            Some("transportAwaitRecoveryAnchor".to_string()),
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

    #[test]
    fn keyframe_request_episode_deferred_marks_unsent_terminal() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let sink = RuntimeStatsSink::new(runtime_stats.clone());

        sink.record_keyframe_request_episode_requested(
            89,
            Some("ingressWaitKeyframe".to_string()),
            100.0,
            None,
        );
        sink.record_keyframe_request_episode_deferred(120.0, "familyInFlight:controlPending");

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

        sink.record_keyframe_request_episode_requested(
            90,
            Some("ingressWaitKeyframe".to_string()),
            100.0,
            None,
        );
        sink.record_keyframe_request_episode_unsent_expired(360.0);

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
        sink.record_keyframe_request_episode_requested(
            1,
            Some("transportAwaitRecoveryAnchor".to_string()),
            100.0,
            None,
        );
        sink.record_keyframe_request_episode_sent("pli", 110.0, Some(500.0));
        sink.record_keyframe_request_episode_response_observed(
            120.0,
            Some(999),
            false,
            "firstResponseNonKeyframe",
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
        sink.record_keyframe_request_episode_requested(
            42,
            Some("transportAwaitRecoveryAnchor".to_string()),
            100.0,
            None,
        );
        sink.record_keyframe_request_episode_sent("pli", 110.0, None);
        sink.record_keyframe_request_episode_packet_seen(120.0, Some(777), true);
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
}
