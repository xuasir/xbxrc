// 由 `runtime_stats_sink` 模块拆分；采集面只写事实，不驱动控制决策。

use crate::transport::rtc::recovery::keyframe_lifecycle::apply_keyframe_episode_lifecycle_field;
use crate::{
    XbxEngineH264InspectionObservation, XbxEngineKeyframeRequestEpisodeObservation,
    XbxEnginePictureRecoveryBlockerObservation, XbxEnginePictureRecoveryTransitionObservation,
};

use super::support::*;
use super::RuntimeStatsSink;

fn should_record_h264_inspection_as_picture_blocker(
    observation: &XbxEngineH264InspectionObservation,
    stats: &crate::XbxEngineMediaRuntimeStats,
) -> bool {
    use crate::transport::rtc::recovery::contract::{
        has_current_clean_anchor_from_stats, is_soft_missing_idr_bootstrap_reject_reason,
    };
    if observation.admission_accepted
        && observation.committed_sps_present
        && observation.committed_pps_present
        && observation.delta_continuation_ready
        && is_soft_missing_idr_bootstrap_reject_reason(
            observation.bootstrap_reject_reason.as_deref(),
        )
    {
        return false;
    }
    if has_current_clean_anchor_from_stats(stats)
        && observation.reject_classification.as_deref() == Some("receiverLocalContinuation")
    {
        return false;
    }
    true
}

impl RuntimeStatsSink {
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
            let unsolicited_bootstrap_idr = is_keyframe
                && detail == "firstKeyframeAccepted"
                && latest_h264_observation.as_ref().is_some_and(|observation| {
                    observation.is_idr
                        && observation.bootstrap_ready
                        && observation.admission_accepted
                        && (observation.parameter_sets_changed || observation.config_changed)
                        && rtp_timestamp
                            .is_some_and(|rtp| observation.frame_rtp_timestamp == Some(rtp))
                });
            let episode_ready_for_binding = stats
                .latest_keyframe_request_episode
                .as_ref()
                .is_some_and(|episode| {
                    episode.sent_at_ms.is_some()
                        && !matches!(
                            episode.response_verdict.as_deref(),
                            Some("transportDeferred" | "transportFailed")
                        )
                });
            if episode_ready_for_binding {
                let Some(episode) = stats.latest_keyframe_request_episode.as_mut() else {
                    return;
                };
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
            } else if stats.latest_keyframe_request_episode.is_some() && !unsolicited_bootstrap_idr
            {
                return;
            } else if unsolicited_bootstrap_idr {
                stats.latest_observation_label =
                    Some("unsolicitedBootstrapIdrResponseObserved".to_string());
                stats.latest_observation_summary = Some(format!(
                    "rtpTimestamp={} detail={} observedAtMs={:.1}",
                    rtp_timestamp
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "none".to_string()),
                    detail,
                    observed_at_ms
                ));
                pending_transition = Some((0, rtp_timestamp, is_keyframe, detail.to_string()));
                should_probe = true;
            }
            if let Some(episode) = updated_episode {
                sync_recent_picture_recovery_episode(stats, episode);
            }
            if let Some((episode_id, rtp_timestamp, is_keyframe, detail)) = pending_transition {
                let observation = XbxEnginePictureRecoveryTransitionObservation {
                    observation_id: Self::next_picture_recovery_transition_observation_id(stats),
                    episode_id: (episode_id > 0).then_some(episode_id),
                    recovery_epoch: Some(stats.transport_recovery_epoch),
                    phase: "ResponseObserved".to_string(),
                    from_phase: if episode_id > 0 {
                        Some("PliSent".to_string())
                    } else {
                        Some("BootstrapUnsolicited".to_string())
                    },
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
            let (pending_transition, should_probe) =
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
                let pending_transition = Some((Some(episode.episode_id), rtp_timestamp, frame_seq));
                updated_episode = Some(episode.clone());
                (pending_transition, true)
            } else {
                stats.latest_observation_label = Some("keyframeDecodedMediaRecovered".to_string());
                stats.latest_observation_summary = Some(format!(
                    "rtpTimestamp={} frameSeq={} observedAtMs={:.1}",
                    rtp_timestamp, frame_seq, observed_at_ms
                ));
                (Some((None, rtp_timestamp, frame_seq)), true)
            };
            if let Some(episode) = updated_episode {
                sync_recent_picture_recovery_episode(stats, episode);
            }
            stats.receive_keyframe_response_state = Some("usable-idr".to_string());
            stats.receive_keyframe_required = Some(false);
            stats.receive_keyframe_required_cause = Some("none".to_string());
            stats.receive_picture_recovery_terminal_candidate = Some(false);
            stats.latest_receive_picture_recovery_terminal_reason = None;
            stats.receive_keyframe_sent_count_unresolved = 0;
            stats.recovery_decoder_reference_synced_at_ms = Some(observed_at_ms);
            stats.latest_video_decode_ok_time_ms = Some(observed_at_ms);
            stats.latest_video_decode_ok_rtp_timestamp = Some(rtp_timestamp);
            stats.latest_clean_anchor_submission_epoch = Some(stats.transport_recovery_epoch);
            stats.latest_clean_anchor_submission_episode_id =
                pending_transition.and_then(|(episode_id, _, _)| episode_id);
            stats.latest_clean_anchor_submission_rtp_timestamp = Some(rtp_timestamp);
            stats.latest_clean_anchor_submission_observed_at_ms = Some(observed_at_ms);
            stats.latest_clean_anchor_submission_source_event =
                Some("decoded-usable-idr".to_string());
            Self::apply_transport_clean_anchor(stats, observed_at_ms, "decoded-usable-idr");
            if let Some((episode_id, rtp_timestamp, frame_seq)) = pending_transition {
                let observation = XbxEnginePictureRecoveryTransitionObservation {
                    observation_id: Self::next_picture_recovery_transition_observation_id(stats),
                    episode_id,
                    recovery_epoch: Some(stats.transport_recovery_epoch),
                    phase: "CleanAnchorCommitted".to_string(),
                    from_phase: Some("Decoded".to_string()),
                    to_phase: "CleanAnchorCommitted".to_string(),
                    cause: Some("decoded-usable-idr".to_string()),
                    detail: Some("mediaRecovered".to_string()),
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
                if !should_record_h264_inspection_as_picture_blocker(&observation, &*stats) {
                    // Insert 已 Accept 的 soft continuation：保留 reject 分类，不记 picture blocker。
                } else {
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
                            observation_id: Self::next_picture_recovery_blocker_observation_id(
                                stats,
                            ),
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
            }
            if observation.is_idr
                && (observation.parameter_sets_changed || observation.config_changed)
                && stats.recovery_displayed_idr_at_ms.is_some()
            {
                stats.video_parameter_sets_changed_at_ms = Some(observation.observed_at_ms);
            }
            stats.latest_h264_inspection_observation = Some(observation);
            stats.latest_observation_label = Some("h264InspectionObserved".to_string());
            stats.latest_observation_summary = Some(summary);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::XbxEngineMediaRuntimeStats;

    #[test]
    fn stale_displayed_idr_does_not_suppress_receiver_local_continuation_blocker() {
        let stats = XbxEngineMediaRuntimeStats {
            recovery_displayed_idr_at_ms: Some(120.0),
            ..Default::default()
        };
        let observation = XbxEngineH264InspectionObservation {
            admission_accepted: false,
            reject_classification: Some("receiverLocalContinuation".to_string()),
            ..Default::default()
        };

        assert!(should_record_h264_inspection_as_picture_blocker(
            &observation,
            &stats
        ));
    }

    #[test]
    fn current_clean_anchor_suppresses_receiver_local_continuation_blocker() {
        let stats = XbxEngineMediaRuntimeStats {
            transport_recovery_epoch: 3,
            video_anchor_clean_epoch: Some(3),
            video_anchor_clean_observed_at_ms: Some(120.0),
            video_anchor_clean_source_event: Some("decoded-usable-idr".to_string()),
            ..Default::default()
        };
        let observation = XbxEngineH264InspectionObservation {
            admission_accepted: false,
            reject_classification: Some("receiverLocalContinuation".to_string()),
            ..Default::default()
        };

        assert!(!should_record_h264_inspection_as_picture_blocker(
            &observation,
            &stats
        ));
    }
}

#[cfg(test)]
impl RuntimeStatsSink {
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
}

impl RuntimeStatsSink {
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
}

#[cfg(test)]
impl RuntimeStatsSink {
    pub(crate) fn record_picture_recovery_episode_unsent_expired(&self, observed_at_ms: f64) {
        self.update(|stats| {
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
                    return;
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
                stats.latest_observation_label =
                    Some("keyframeRequestEpisodeUnsentExpired".to_string());
                stats.latest_observation_summary = latest_summary;
            }
            if let Some(episode) = updated_episode {
                sync_recent_picture_recovery_episode(stats, episode);
            }
        });
    }
}
