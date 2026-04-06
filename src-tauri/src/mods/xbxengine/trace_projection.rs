use serde_json::json;
use xbxengine_protocol::XbxEngineStatsDto;

use crate::mods::runtime_trace::RuntimeTraceRecorderRef;

const DIRECT_GAMING_STATE_SAMPLE_INTERVAL_MS: f64 = 1_000.0;
const HOST_PRESENT_STATE_SAMPLE_EPOCH_INTERVAL: u64 = 60;
const VIDEO_TRACK_STATE_SAMPLE_INTERVAL_MS: f64 = 1_000.0;

#[derive(Default)]
pub(super) struct RuntimeTraceObservationState {
    packet_gap_observation_id: Option<u64>,
    frame_drop_observation_id: Option<u64>,
    frame_recovery_observation_id: Option<u64>,
    nack_observation_id: Option<u64>,
    escalation_observation_id: Option<u64>,
    recovery_decision_ledger_signature: Option<(u64, Option<String>, Option<String>)>,
    bwe_observation_id: Option<u64>,
    twcc_observation_id: Option<u64>,
    rtc_builder_observation_id: Option<u64>,
    twcc_remote_stream_observation_id: Option<u64>,
    remote_answer_observation_id: Option<u64>,
    twcc_extension_observation_id: Option<u64>,
    data_channel_catalog_observation_id: Option<u64>,
    timeline_observation_id: Option<u64>,
    anchor_candidate_observation: Option<(u64, Option<u32>, String, Option<String>, f64)>,
    h264_inspection_observation: Option<xbxengine_protocol::XbxEngineH264InspectionObservationDto>,
    decode_candidate_decision_id: Option<u64>,
    render_candidate_decision_id: Option<u64>,
    recovery_keyframe_request_count: Option<u64>,
    recovery_decoder_reset_count: Option<u64>,
    recovery_reconnect_count: Option<u64>,
    keyframe_request_episode:
        Option<xbxengine_protocol::XbxEngineKeyframeRequestEpisodeObservationDto>,
    recovery_hard_fallback_timer_ms: Option<f64>,
    recovery_hard_fallback_trigger_reason: Option<String>,
    recovery_hard_fallback_timer_reset_reason: Option<String>,
    decoder_recovery_state: Option<(
        Option<String>,
        Option<String>,
        Option<String>,
        Option<i32>,
        Option<f64>,
    )>,
    transport_state: Option<String>,
    transport_path: Option<String>,
    transport_candidate_pair: Option<String>,
    transport_protocol: Option<String>,
    transport_address_family: Option<String>,
    latest_video_track_status: Option<xbxengine_protocol::XbxEngineVideoTrackStatusDto>,
    video_remb_bps: Option<u32>,
    remote_profile_baseline: Option<String>,
    remote_profile_dynamic: Option<String>,
    remote_profile_effective_label: Option<String>,
    session_phase: Option<String>,
    transport_strategy_profile: Option<String>,
    recovery_strategy_profile: Option<String>,
    recovery_diagnosis: Option<String>,
    direct_gaming_bitrate_band: Option<String>,
    runtime_summary: Option<String>,
    primary_issue_chain: Option<String>,
    latest_decision_summary: Option<String>,
    recovery_owner_state: Option<String>,
    recovery_owner_reason: Option<String>,
    video_owner_source: Option<String>,
    video_owner_observed_at_ms: Option<f64>,
    video_owner_observed_at_bucket: Option<u64>,
    unified_lifecycle: Option<String>,
    video_health: Option<String>,
    stall_kind: Option<String>,
    host_present_enqueue_count_total: Option<u64>,
    host_present_drop_count_total: Option<u64>,
    host_present_overwrite_count_total: Option<u64>,
    host_no_pending_take_count_total: Option<u64>,
    host_no_pending_streak: Option<u32>,
    host_no_pending_max_streak: Option<u32>,
    host_no_pending_pressure_level: Option<String>,
    host_display_tick_epoch: Option<u64>,
    host_present_epoch: Option<u64>,
    host_cadence_phase: Option<String>,
    host_descriptor_upload_mode: Option<String>,
    host_descriptor_metal_import_count_total: Option<u64>,
    host_descriptor_cpu_upload_count_total: Option<u64>,
    actual_video_bitrate_source: Option<String>,
    twcc_observation_state: Option<String>,
    latest_observation_label: Option<String>,
    latest_observation_summary: Option<String>,
    latest_target_remb_action: Option<String>,
    latest_target_remb_summary: Option<String>,
    timeline_chain_state: Option<String>,
    timeline_chain_reason: Option<String>,
    video_decoder_stalled: Option<bool>,
    video_renderer_stalled: Option<bool>,
    video_track_state_signature: Option<(
        String,
        Option<u32>,
        Option<u32>,
        Option<String>,
        xbxengine_protocol::XbxEngineTransportStateDto,
    )>,
    video_track_state_bucket: Option<u64>,
}

pub(super) fn should_skip_trace_tick(session_id: Option<&str>, stats: &XbxEngineStatsDto) -> bool {
    session_id.is_none() && stats.transport_state.as_deref() == Some("Closed")
}

/// 统一观测快照：把 UI 与离线分析真正关心的状态压成单条 snapshot，避免继续手工拼
/// `statsSnapshot + directGamingState + hostPresentState`。
pub(super) fn build_observability_snapshot(stats: &XbxEngineStatsDto) -> serde_json::Value {
    let unified_lifecycle = resolve_unified_lifecycle(stats);
    json!({
        "resolution": stats.resolution,
        "fps": stats.fps,
        "rtt": stats.rtt,
        "frameBudget": latest_frame_budget_snapshot(stats),
        "runtimeSummary": stats.runtime_summary,
        "primaryIssueChain": stats.primary_issue_chain,
        "latestDecisionSummary": stats.latest_decision_summary,
        "videoOwner": {
            "state": stats.recovery_owner_state,
            "reason": stats.recovery_owner_reason,
            "source": stats.video_owner_source,
            "observedAtMs": stats.video_owner_observed_at_ms,
        },
        "remoteProfile": {
            "baseline": stats.remote_profile_baseline,
            "dynamic": stats.remote_profile_dynamic,
            "effectiveLabel": stats.remote_profile_effective_label,
        },
        "transport": {
            "path": stats.transport_path,
            "candidatePair": stats.transport_candidate_pair,
            "protocol": stats.transport_protocol,
            "addressFamily": stats.transport_address_family,
            "state": stats.transport_state,
            "strategyProfile": stats.transport_strategy_profile,
            "videoRttSource": stats.video_rtt_source,
            "videoRembBps": stats.video_remb_bps,
        },
        "recovery": {
            "lifecycle": unified_lifecycle,
            "streamLifecyclePhase": unified_lifecycle,
            "sessionPhase": stats.session_phase,
            "strategyProfile": stats.recovery_strategy_profile,
            "diagnosis": stats.recovery_diagnosis,
            "videoHealth": stats.video_health,
            "videoOwnerState": stats.recovery_owner_state,
            "videoOwnerReason": stats.recovery_owner_reason,
            "videoOwnerSource": stats.video_owner_source,
            "videoOwnerObservedAtMs": stats.video_owner_observed_at_ms,
            "stallKind": stats.stall_kind,
            "keyframeRequestCount": stats.recovery_keyframe_request_count,
            "decoderResetCount": stats.recovery_decoder_reset_count,
            "reconnectCount": stats.recovery_reconnect_count,
            "hardFallbackTimerMs": stats.recovery_hard_fallback_timer_ms,
            "hardFallbackTriggerReason": stats.recovery_hard_fallback_trigger_reason,
            "hardFallbackTimerResetReason": stats.recovery_hard_fallback_timer_reset_reason,
            "lastAction": stats.last_recovery_action,
            "lastActionAtMs": stats.last_recovery_action_at_ms,
            "lastReason": stats.last_recovery_reason,
            "reconnectTriggerSource": stats.reconnect_trigger_source,
            "decoderState": stats.video_decoder_recovery_state,
            "decoderEvent": stats.video_decoder_recovery_event,
            "decoderDetail": stats.video_decoder_recovery_detail,
            "decoderStatus": stats.video_decoder_recovery_status,
            "decoderStateChangedAtMs": stats.video_decoder_recovery_state_changed_at_ms,
        },
        "directGaming": {
            "bitrateBand": stats.direct_gaming_bitrate_band,
        },
        "bitrate": {
            "display": stats.br,
            "videoDisplay": stats.br,
            "totalKbps": stats.inbound_bitrate_kbps,
            "inboundKbps": stats.inbound_bitrate_kbps,
            "videoKbps": stats.inbound_video_bitrate_kbps,
            "audioKbps": stats.inbound_audio_bitrate_kbps,
            "actualVideoKbps": stats.video_actual_bitrate_kbps,
            "actualVideoBitrateSource": stats.actual_video_bitrate_source,
            "bytesTotal": stats.inbound_bytes_total,
            "videoBytesTotal": stats.inbound_video_bytes_total,
            "audioBytesTotal": stats.inbound_audio_bytes_total,
        },
        "audio": {
            "latestAudioPlayoutTimeMs": stats.latest_audio_playout_time_ms,
            "audioPlayoutLatencyMs": stats.audio_playout_latency_ms,
            "audioVideoPlayoutDeltaMs": stats.audio_video_playout_delta_ms,
        },
        "bwe": {
            "mode": stats.video_bwe_mode,
            "reason": stats.video_bwe_reason,
            "targetKbps": stats.video_target_remb_kbps,
            "observedRembKbps": stats.video_observed_remb_kbps,
            "actualVideoKbps": stats.video_actual_bitrate_kbps,
            "actualVideoBitrateSource": stats.actual_video_bitrate_source,
        },
        "twcc": {
            "state": stats.twcc_observation_state,
            "receiveKbps": stats.video_twcc_receive_bitrate_kbps,
            "lossRatio": stats.video_twcc_loss_ratio,
            "deliveryRatio": stats.video_twcc_delivery_ratio,
            "feedbackIntervalMs": stats.video_twcc_feedback_interval_ms,
            "coverageRatio": stats
                .latest_video_twcc_observation
                .as_ref()
                .and_then(|twcc| twcc.coverage_ratio),
            "ledgerHitRatio": stats
                .latest_video_twcc_observation
                .as_ref()
                .and_then(|twcc| twcc.ledger_hit_ratio),
            "sampleValid": stats
                .latest_video_twcc_observation
                .as_ref()
                .map(|twcc| twcc.twcc_sample_valid),
            "invalidReason": stats
                .latest_video_twcc_observation
                .as_ref()
                .and_then(|twcc| twcc.twcc_invalid_reason.clone()),
        },
        "buildFingerprint": stats.build_fingerprint,
        "video": {
            "inboundFps": stats.inbound_video_fps,
            "decodeFps": stats.decode_fps,
            "presentFps": stats.present_fps,
            "inboundPrimaryBytesTotal": stats
                .latest_video_track_status
                .as_ref()
                .map(|status| status.video_bytes_total)
                .or(stats.inbound_video_bytes_total),
            "inboundVideoPacketsTotal": stats
                .latest_video_track_status
                .as_ref()
                .map(|status| status.video_packet_count_total)
                .or(stats.inbound_video_packet_count_total),
            "inboundAudioBytesTotal": stats
                .latest_video_track_status
                .as_ref()
                .map(|status| status.audio_bytes_total)
                .or(stats.inbound_audio_bytes_total),
            "packetAgeMs": stats.packet_age_ms,
            "decodeAgeMs": stats.decode_age_ms,
            "presentAgeMs": stats.present_age_ms,
            "packetToDecodeMs": stats.packet_to_decode_ms,
            "decodeToPresentMs": stats.decode_to_present_ms,
            "packetToPresentMs": stats.packet_to_present_ms,
            "decoderStalled": stats.video_decoder_stalled,
            "rendererStalled": stats.video_renderer_stalled,
            "decodeInputDropCountTotal": stats.video_decode_input_drop_count_total,
            "decodeOutputDropCountTotal": stats.video_decode_output_drop_count_total,
            "pacerSubmitCountTotal": stats.video_pacer_submit_count_total,
            "pacerDropCountTotal": stats.video_pacer_drop_count_total,
            "rendererSubmitCountTotal": stats.video_renderer_submit_count_total,
            "rendererDropCountTotal": stats.video_renderer_drop_count_total,
            "presentEnqueueCountTotal": stats.video_present_submit_count_total,
            "presentDropCountTotal": stats.video_present_drop_count_total,
            "presentOverwriteCountTotal": stats.video_present_overwrite_count_total,
            "noPendingTakeCountTotal": stats.host_no_pending_take_count_total,
            "noPendingStreak": stats.host_no_pending_streak,
            "noPendingMaxStreak": stats.host_no_pending_max_streak,
            "noPendingPressureLevel": stats.host_no_pending_pressure_level,
            "displayTickEpoch": stats.host_display_tick_epoch,
            "presentEpoch": stats.video_present_epoch,
            "cadencePhase": stats.host_cadence_phase,
            "descriptorUploadMode": stats.video_present_descriptor_upload_mode,
            "descriptorMetalImportCountTotal": stats.video_present_descriptor_metal_import_count_total,
            "descriptorCpuUploadCountTotal": stats.video_present_descriptor_cpu_upload_count_total,
        },
        "latest": {
            "packetGap": stats.latest_video_packet_gap,
            "frameDrop": stats.latest_video_frame_drop,
            "frameRecovery": stats.latest_video_frame_recovery_observation,
            "nack": stats.latest_video_nack_observation,
            "keyframeRequestEpisode": stats.latest_keyframe_request_episode,
            "h264Inspection": h264_inspection_payload(stats.latest_h264_inspection_observation.as_ref()),
            "timeline": stats.latest_video_timeline_observation,
            "decodeCandidate": stats.latest_decode_candidate_decision,
            "renderCandidate": stats.latest_render_candidate_decision,
            "escalation": stats.latest_video_escalation_observation,
            "recoveryDecisionLedger": stats.latest_recovery_decision_ledger,
            "bwe": stats.latest_video_bwe_observation,
            "twcc": stats.latest_video_twcc_observation,
        },
    })
}

fn resolve_unified_lifecycle(stats: &XbxEngineStatsDto) -> &'static str {
    if let Some(phase) = stats.stream_lifecycle_phase.as_deref() {
        match phase {
            "startup" => return "startup",
            "recovering" => return "recovering",
            "ramp-up" => return "ramp-up",
            "steady" => return "steady",
            "degraded" => return "degraded",
            "failed" => return "failed",
            "closed" => return "closed",
            _ => {}
        }
    }
    if let Some(phase) = stats.session_phase.as_deref() {
        match phase {
            "connecting" | "handshaking" | "priming" | "startup" => return "startup",
            "recovering" => return "recovering",
            "ramp-up" => return "ramp-up",
            "steady" => return "steady",
            "degraded" => return "degraded",
            "failed" => return "failed",
            "closed" => return "closed",
            _ => {}
        }
    }
    match stats.transport_state.as_deref() {
        Some("Closed") => "closed",
        Some("Failed") => "failed",
        _ => "startup",
    }
}

fn latest_frame_budget_snapshot(stats: &XbxEngineStatsDto) -> Option<serde_json::Value> {
    let mut latest: Option<(&str, f64, &xbxengine_protocol::XbxEngineFrameBudgetDto)> = None;
    if let Some(frame_recovery) = stats.latest_video_frame_recovery_observation.as_ref() {
        if let Some(frame_budget) = frame_recovery.frame_budget.as_ref() {
            latest = Some(("frameRecovery", frame_recovery.observed_at_ms, frame_budget));
        }
    }
    if let Some(nack) = stats.latest_video_nack_observation.as_ref() {
        if let Some(frame_budget) = nack.frame_budget.as_ref() {
            if latest.is_none_or(|(_, observed_at_ms, _)| nack.observed_at_ms >= observed_at_ms) {
                latest = Some(("nack", nack.observed_at_ms, frame_budget));
            }
        }
    }
    if let Some(frame_drop) = stats.latest_video_frame_drop.as_ref() {
        if let Some(frame_budget) = frame_drop.frame_budget.as_ref() {
            if latest
                .is_none_or(|(_, observed_at_ms, _)| frame_drop.observed_at_ms >= observed_at_ms)
            {
                latest = Some(("frameDrop", frame_drop.observed_at_ms, frame_budget));
            }
        }
    }

    latest.map(|(source, observed_at_ms, budget)| {
        json!({
            "source": source,
            "observedAtMs": observed_at_ms,
            "recoveryStage": budget.recovery_stage,
            "chainValue": budget.chain_value,
            "rttSlack": budget.rtt_slack,
            "failureCost": budget.failure_cost,
            "windowSource": budget.window_source,
        })
    })
}

pub(super) fn record_runtime_trace_observations(
    runtime_trace: &RuntimeTraceRecorderRef,
    observation_state: &mut RuntimeTraceObservationState,
    session_id: Option<&str>,
    stats: &XbxEngineStatsDto,
) {
    if let Some(packet_gap) = stats.latest_video_packet_gap.as_ref() {
        if observation_state.packet_gap_observation_id != Some(packet_gap.observation_id) {
            observation_state.packet_gap_observation_id = Some(packet_gap.observation_id);
            runtime_trace.record_event(
                "xbxengine",
                "packetGapDetected",
                session_id,
                json!({
                    "observationId": packet_gap.observation_id,
                    "expectedSequence": packet_gap.expected_sequence,
                    "receivedSequence": packet_gap.received_sequence,
                    "missingCount": packet_gap.missing_count,
                    "source": packet_gap.source,
                    "frameRtpTimestamp": packet_gap.frame_rtp_timestamp,
                    "framePacketCount": packet_gap.frame_packet_count,
                    "frameMissingCount": packet_gap.frame_missing_count,
                    "frameIsKeyframe": packet_gap.frame_is_keyframe,
                    "frameImportance": packet_gap.frame_importance,
                    "observedAtMs": packet_gap.observed_at_ms,
                }),
            );
        }
    }

    if let Some(frame_drop) = stats.latest_video_frame_drop.as_ref() {
        if observation_state.frame_drop_observation_id != Some(frame_drop.observation_id) {
            observation_state.frame_drop_observation_id = Some(frame_drop.observation_id);
            let event_name = if frame_drop.reason == "dropLate" {
                "frameDeadlineMissed"
            } else {
                "frameDropped"
            };
            runtime_trace.record_event(
                "xbxengine",
                event_name,
                session_id,
                json!({
                    "observationId": frame_drop.observation_id,
                    "reason": frame_drop.reason,
                    "stage": frame_drop.stage,
                    "action": frame_drop.action,
                    "detail": frame_drop.detail,
                    "frameRtpTimestamp": frame_drop.frame_rtp_timestamp,
                    "frameSeq": frame_drop.frame_seq,
                    "frameRecoveryDisposition": frame_drop.frame_recovery_disposition,
                    "frameUnrecoverableReason": frame_drop.frame_unrecoverable_reason,
                    "frameBudget": frame_drop.frame_budget,
                    "observedAtMs": frame_drop.observed_at_ms,
                    "width": frame_drop.width,
                    "height": frame_drop.height,
                    "isKeyframe": frame_drop.is_keyframe,
                    "queueDepth": frame_drop.queue_depth,
                }),
            );
            let decision_event_name = match frame_drop.stage.as_deref() {
                Some("decode") => Some("decodeCandidateDecision"),
                Some("pacer" | "render") => Some("renderCandidateDecision"),
                _ => None,
            };
            if let Some(decision_event_name) = decision_event_name {
                runtime_trace.record_event(
                    "xbxengine",
                    decision_event_name,
                    session_id,
                    json!({
                        "observationId": frame_drop.observation_id,
                        "stage": frame_drop.stage,
                        "action": frame_drop.action,
                        "detail": frame_drop.detail,
                        "reason": frame_drop.reason,
                        "frameSeq": frame_drop.frame_seq,
                        "frameRtpTimestamp": frame_drop.frame_rtp_timestamp,
                        "frameRecoveryDisposition": frame_drop.frame_recovery_disposition,
                        "frameUnrecoverableReason": frame_drop.frame_unrecoverable_reason,
                        "frameBudget": frame_drop.frame_budget,
                        "queueDepth": frame_drop.queue_depth,
                        "observedAtMs": frame_drop.observed_at_ms,
                    }),
                );
            }
        }
    }

    if let Some(frame_recovery) = stats.latest_video_frame_recovery_observation.as_ref() {
        if observation_state.frame_recovery_observation_id != Some(frame_recovery.observation_id) {
            observation_state.frame_recovery_observation_id = Some(frame_recovery.observation_id);
            runtime_trace.record_event(
                "xbxengine",
                "frameRecoveryObserved",
                session_id,
                json!({
                    "observationId": frame_recovery.observation_id,
                    "action": frame_recovery.action,
                    "frameRtpTimestamp": frame_recovery.frame_rtp_timestamp,
                    "framePlayoutDeadlineAtMs": frame_recovery.frame_playout_deadline_at_ms,
                    "frameRecoveryDisposition": frame_recovery.frame_recovery_disposition,
                    "frameUnrecoverableReason": frame_recovery.frame_unrecoverable_reason,
                    "frameBudget": frame_recovery.frame_budget,
                    "observedAtMs": frame_recovery.observed_at_ms,
                }),
            );
        }
    }

    let decoder_recovery_state = (
        stats.video_decoder_recovery_state.clone(),
        stats.video_decoder_recovery_event.clone(),
        stats.video_decoder_recovery_detail.clone(),
        stats.video_decoder_recovery_status,
        stats.video_decoder_recovery_state_changed_at_ms,
    );
    if observation_state.decoder_recovery_state.as_ref() != Some(&decoder_recovery_state) {
        observation_state.decoder_recovery_state = Some(decoder_recovery_state.clone());
        runtime_trace.record_event(
            "xbxengine",
            "decoderRecoveryStateChanged",
            session_id,
            json!({
                "state": decoder_recovery_state.0,
                "event": decoder_recovery_state.1,
                "detail": decoder_recovery_state.2,
                "status": decoder_recovery_state.3,
                "stateChangedAtMs": decoder_recovery_state.4,
            }),
        );
    }

    if let Some(decode_candidate) = stats.latest_decode_candidate_decision.as_ref() {
        if observation_state.decode_candidate_decision_id != Some(decode_candidate.decision_id) {
            observation_state.decode_candidate_decision_id = Some(decode_candidate.decision_id);
            runtime_trace.record_event(
                "xbxengine",
                "decodeCandidateStateTransition",
                session_id,
                json!({
                    "decisionId": decode_candidate.decision_id,
                    "state": decode_candidate.state,
                    "action": decode_candidate.action,
                    "detail": decode_candidate.detail,
                    "frameSeq": decode_candidate.frame_seq,
                    "observedAtMs": decode_candidate.observed_at_ms,
                }),
            );
        }
    }

    if let Some(render_candidate) = stats.latest_render_candidate_decision.as_ref() {
        if observation_state.render_candidate_decision_id != Some(render_candidate.decision_id) {
            observation_state.render_candidate_decision_id = Some(render_candidate.decision_id);
            runtime_trace.record_event(
                "xbxengine",
                "renderCandidateStateTransition",
                session_id,
                json!({
                    "decisionId": render_candidate.decision_id,
                    "state": render_candidate.state,
                    "action": render_candidate.action,
                    "detail": render_candidate.detail,
                    "frameSeq": render_candidate.frame_seq,
                    "observedAtMs": render_candidate.observed_at_ms,
                }),
            );
        }
    }

    if let Some(nack) = stats.latest_video_nack_observation.as_ref() {
        if observation_state.nack_observation_id != Some(nack.observation_id) {
            observation_state.nack_observation_id = Some(nack.observation_id);
            let event_name = match nack.action.as_str() {
                "expiredDeadline" | "expiredMaxAge" | "expiredRetryBudget"
                | "expiredChainBroken" => "nackExpired",
                "recovered" | "recoveredLate" => "nackRecovered",
                "skipped" => "nackSkipped",
                _ => "nackSent",
            };
            runtime_trace.record_event(
                "xbxengine",
                event_name,
                session_id,
                json!({
                    "observationId": nack.observation_id,
                    "action": nack.action,
                    "source": nack.source,
                    "firstSequence": nack.first_sequence,
                    "lastSequence": nack.last_sequence,
                    "packetCount": nack.packet_count,
                    "retryCount": nack.retry_count,
                    "frameRtpTimestamp": nack.frame_rtp_timestamp,
                    "frameIsKeyframe": nack.frame_is_keyframe,
                    "frameImportance": nack.frame_importance,
                    "deadlineAtMs": nack.deadline_at_ms,
                    "estimatedRecoveryArrivalMs": nack.estimated_recovery_arrival_ms,
                    "nackDisposition": nack.nack_disposition,
                    "framePlayoutDeadlineAtMs": nack.frame_playout_deadline_at_ms,
                    "frameUnrecoverableReason": nack.frame_unrecoverable_reason,
                    "frameBudget": nack.frame_budget,
                    "observedAtMs": nack.observed_at_ms,
                }),
            );
        }
    }

    if let Some(timeline) = stats.latest_video_timeline_observation.as_ref() {
        if observation_state.timeline_observation_id != Some(timeline.observation_id) {
            observation_state.timeline_observation_id = Some(timeline.observation_id);
            let previous_chain_state = observation_state.timeline_chain_state.clone();
            let previous_chain_reason = observation_state.timeline_chain_reason.clone();
            runtime_trace.record_event(
                "xbxengine",
                "videoTimelineObserved",
                session_id,
                json!({
                    "observationId": timeline.observation_id,
                    "sourceEvent": timeline.source_event,
                    "gap": timeline.gap,
                    "frame": timeline.frame,
                    "chain": timeline.chain,
                    "observedAtMs": timeline.observed_at_ms,
                }),
            );
            let chain_state_changed =
                previous_chain_state.as_deref() != Some(timeline.chain.state.as_str());
            if chain_state_changed || is_chain_transition_source_event(&timeline.source_event) {
                runtime_trace.record_event(
                    "xbxengine",
                    "videoChainTransition",
                    session_id,
                    json!({
                        "observationId": timeline.observation_id,
                        "sourceEvent": timeline.source_event,
                        "previousChainState": previous_chain_state,
                        "previousChainReason": previous_chain_reason,
                        "state": timeline.chain.state,
                        "reason": timeline.chain.reason,
                        "chain": {
                            "state": timeline.chain.state,
                            "reason": timeline.chain.reason,
                        },
                        "observedAtMs": timeline.observed_at_ms,
                    }),
                );
            }
            if is_timeout_source_event(&timeline.source_event) {
                runtime_trace.record_event(
                    "xbxengine",
                    "videoTimeoutTransition",
                    session_id,
                    json!({
                        "observationId": timeline.observation_id,
                        "sourceEvent": timeline.source_event,
                        "chain": {
                            "state": timeline.chain.state,
                            "reason": timeline.chain.reason,
                        },
                        "observedAtMs": timeline.observed_at_ms,
                    }),
                );
            }
            if is_chain_flush_source_event(&timeline.source_event) {
                runtime_trace.record_event(
                    "xbxengine",
                    "videoBacklogFlushed",
                    session_id,
                    json!({
                        "observationId": timeline.observation_id,
                        "sourceEvent": timeline.source_event,
                        "gap": timeline.gap,
                        "frame": timeline.frame,
                        "chain": timeline.chain,
                        "observedAtMs": timeline.observed_at_ms,
                    }),
                );
            }
            observation_state.timeline_chain_state = Some(timeline.chain.state.clone());
            observation_state.timeline_chain_reason = timeline.chain.reason.clone();
        }
    }

    if let Some(candidate) = stats.latest_anchor_candidate_ledger.as_ref() {
        let current = (
            candidate.recovery_epoch,
            candidate.frame_rtp_timestamp,
            candidate.state.clone(),
            candidate.failure_reason.clone(),
            candidate.observed_at_ms,
        );
        if observation_state.anchor_candidate_observation.as_ref() != Some(&current) {
            observation_state.anchor_candidate_observation = Some(current);
            runtime_trace.record_event(
                "xbxengine",
                "videoAnchorCandidateObserved",
                session_id,
                json!({
                    "recoveryEpoch": candidate.recovery_epoch,
                    "frameRtpTimestamp": candidate.frame_rtp_timestamp,
                    "state": candidate.state,
                    "sourceEvent": candidate.source_event,
                    "failureReason": candidate.failure_reason,
                    "observedAtMs": candidate.observed_at_ms,
                }),
            );
        }
    }

    if let Some(escalation) = stats.latest_video_escalation_observation.as_ref() {
        if observation_state.escalation_observation_id != Some(escalation.observation_id) {
            observation_state.escalation_observation_id = Some(escalation.observation_id);
            runtime_trace.record_decision(
                "xbxengine",
                "videoEscalation",
                session_id,
                json!({
                    "observationId": escalation.observation_id,
                    "reason": escalation.reason,
                    "action": escalation.action,
                    "recoveryStage": escalation.recovery_stage,
                    "recoveryChainValue": escalation.recovery_chain_value,
                    "recoveryFailureCost": escalation.recovery_failure_cost,
                    "recoveryWindowSource": escalation.recovery_window_source,
                    "observedAtMs": escalation.observed_at_ms,
                }),
            );
        }
    }

    if let Some(ledger) = stats.latest_recovery_decision_ledger.as_ref() {
        let signature = (
            ledger.decision_id,
            ledger.command_result.clone(),
            ledger.command_detail.clone(),
        );
        if observation_state
            .recovery_decision_ledger_signature
            .as_ref()
            != Some(&signature)
        {
            observation_state.recovery_decision_ledger_signature = Some(signature);
            runtime_trace.record_decision(
                "xbxengine",
                "recoveryDecisionLedger",
                session_id,
                json!({
                    "decisionId": ledger.decision_id,
                    "stateBefore": ledger.state_before,
                    "stateAfter": ledger.state_after,
                    "inputSignal": ledger.input_signal,
                    "gateResult": ledger.gate_result,
                    "actionSelected": ledger.action_selected,
                    "budgetBefore": ledger.budget_before,
                    "budgetAfter": ledger.budget_after,
                    "commandResult": ledger.command_result,
                    "commandDetail": ledger.command_detail,
                    "observedAtMs": ledger.observed_at_ms,
                }),
            );
        }
    }

    if let Some(bwe) = stats.latest_video_bwe_observation.as_ref() {
        if observation_state.bwe_observation_id != Some(bwe.observation_id) {
            observation_state.bwe_observation_id = Some(bwe.observation_id);
            runtime_trace.record_decision(
                "xbxengine",
                "bweUpdated",
                session_id,
                json!({
                    "observationId": bwe.observation_id,
                    "mode": bwe.mode,
                    "decisionReason": bwe.decision_reason,
                    "targetRembKbps": bwe.target_remb_kbps,
                    "observedRembKbps": bwe.observed_remb_kbps,
                    "actualVideoBitrateKbps": stats.video_actual_bitrate_kbps,
                    "actualVideoBitrateSource": stats.actual_video_bitrate_source,
                    "lossRatio": bwe.loss_ratio,
                    "rttMs": bwe.rtt_ms,
                    "transportPath": bwe.transport_path,
                    "twccFeedbackIntervalMs": bwe.twcc_feedback_interval_ms,
                    "twccObservedPacketCount": bwe.twcc_observed_packet_count,
                    "twccCoveredSequenceSpan": bwe.twcc_covered_sequence_span,
                    "twccReceiveBitrateKbps": bwe.twcc_receive_bitrate_kbps,
                    "twccDeliveryRatio": bwe.twcc_delivery_ratio,
                    "twccLossRatio": bwe.twcc_loss_ratio,
                    "observedAtMs": bwe.observed_at_ms,
                }),
            );
        }
    }

    if let Some(twcc) = stats.latest_video_twcc_observation.as_ref() {
        if observation_state.twcc_observation_id != Some(twcc.observation_id) {
            observation_state.twcc_observation_id = Some(twcc.observation_id);
            let event_name = if twcc.source == "local-feedback" {
                "twccFeedbackSent"
            } else {
                "twccFeedbackObserved"
            };
            runtime_trace.record_event(
                "xbxengine",
                event_name,
                session_id,
                json!({
                    "observationId": twcc.observation_id,
                    "source": twcc.source,
                    "quality": twcc.quality.as_str(),
                    "feedbackPacketCount": twcc.feedback_packet_count,
                    "coveredSequenceStart": twcc.covered_sequence_start,
                    "coveredSequenceEnd": twcc.covered_sequence_end,
                    "coveredSequenceSpan": twcc.covered_sequence_span,
                    "observedPacketCount": twcc.observed_packet_count,
                    "observedByteCount": twcc.observed_byte_count,
                    "coverageRatio": twcc.coverage_ratio,
                    "ledgerHitRatio": twcc.ledger_hit_ratio,
                    "feedbackIntervalMs": twcc.feedback_interval_ms,
                    "state": stats.twcc_observation_state,
                    "arrivalSpanMs": twcc.arrival_span_ms,
                    "receiveBitrateKbps": twcc.receive_bitrate_kbps,
                    "twccSampleValid": twcc.twcc_sample_valid,
                    "twccInvalidReason": twcc.twcc_invalid_reason,
                    "deliveryRatio": twcc.delivery_ratio,
                    "packetLossRatio": twcc.packet_loss_ratio,
                    "observedAtMs": twcc.observed_at_ms,
                }),
            );
        }
    }

    if let Some(observation) = stats.latest_rtc_builder_observation.as_ref() {
        if observation_state.rtc_builder_observation_id != Some(observation.observation_id) {
            observation_state.rtc_builder_observation_id = Some(observation.observation_id);
            runtime_trace.record_event(
                "xbxengine",
                "rtcBuilderConfigured",
                session_id,
                json!({
                    "observationId": observation.observation_id,
                    "controlledTwccRegistry": observation.controlled_twcc_registry,
                    "feedbackIntervalMs": observation.feedback_interval_ms,
                    "registeredHeaderExtensions": observation.registered_header_extensions,
                    "registeredRtcpFeedback": observation.registered_rtcp_feedback,
                    "observedAtMs": observation.observed_at_ms,
                }),
            );
        }
    }

    if let Some(observation) = stats.latest_twcc_remote_stream_observation.as_ref() {
        if observation_state.twcc_remote_stream_observation_id != Some(observation.observation_id) {
            observation_state.twcc_remote_stream_observation_id = Some(observation.observation_id);
            runtime_trace.record_event(
                "xbxengine",
                "twccRemoteStreamBound",
                session_id,
                json!({
                    "observationId": observation.observation_id,
                    "ssrc": observation.ssrc,
                    "mimeType": observation.mime_type,
                    "twccExtId": observation.twcc_ext_id,
                    "headerExtensions": observation.header_extensions,
                    "rtcpFeedback": observation.rtcp_feedback,
                    "observedAtMs": observation.observed_at_ms,
                }),
            );
        }
    }

    if let Some(observation) = stats.latest_remote_answer_observation.as_ref() {
        if observation_state.remote_answer_observation_id != Some(observation.observation_id) {
            let accepted_video_feedback = observation
                .accepted_video_rtcp_feedback
                .iter()
                .map(|value| value.to_ascii_lowercase())
                .collect::<Vec<_>>();
            let accepted_audio_feedback = observation
                .accepted_audio_rtcp_feedback
                .iter()
                .map(|value| value.to_ascii_lowercase())
                .collect::<Vec<_>>();
            let video_goog_remb_accepted = accepted_video_feedback
                .iter()
                .any(|value| value == "goog-remb" || value.starts_with("goog-remb:"));
            let video_transport_cc_accepted = accepted_video_feedback
                .iter()
                .any(|value| value == "transport-cc" || value.starts_with("transport-cc:"));
            let audio_goog_remb_accepted = accepted_audio_feedback
                .iter()
                .any(|value| value == "goog-remb" || value.starts_with("goog-remb:"));
            let audio_transport_cc_accepted = accepted_audio_feedback
                .iter()
                .any(|value| value == "transport-cc" || value.starts_with("transport-cc:"));

            observation_state.remote_answer_observation_id = Some(observation.observation_id);
            runtime_trace.record_event(
                "xbxengine",
                "remoteAnswerAccepted",
                session_id,
                json!({
                    "observationId": observation.observation_id,
                    "selectedVideoPayloadType": observation.selected_video_payload_type,
                    "selectedVideoMimeType": observation.selected_video_mime_type,
                    "selectedVideoProfileLevelId": observation.selected_video_profile_level_id,
                    "selectedVideoH264SpropParameterSets": observation.selected_video_h264_sprop_parameter_sets,
                    "videoPayloadOrder": observation.video_payload_order,
                    "acceptedVideoRtcpFeedback": observation.accepted_video_rtcp_feedback,
                    "acceptedAudioRtcpFeedback": observation.accepted_audio_rtcp_feedback,
                    "acceptedVideoHeaderExtensions": observation.accepted_video_header_extensions,
                    "acceptedAudioHeaderExtensions": observation.accepted_audio_header_extensions,
                    "videoFeedbackAcceptance": {
                        "googRembAccepted": video_goog_remb_accepted,
                        "transportCcAccepted": video_transport_cc_accepted,
                    },
                    "audioFeedbackAcceptance": {
                        "googRembAccepted": audio_goog_remb_accepted,
                        "transportCcAccepted": audio_transport_cc_accepted,
                    },
                    "observedAtMs": observation.observed_at_ms,
                }),
            );
        }
    }

    if let Some(observation) = stats.latest_twcc_extension_observation.as_ref() {
        if observation_state.twcc_extension_observation_id != Some(observation.observation_id) {
            observation_state.twcc_extension_observation_id = Some(observation.observation_id);
            runtime_trace.record_event(
                "xbxengine",
                if observation.state == "seen" {
                    "twccInboundExtensionSeen"
                } else {
                    "twccInboundExtensionMissing"
                },
                session_id,
                json!({
                    "observationId": observation.observation_id,
                    "state": observation.state,
                    "ssrc": observation.ssrc,
                    "sequenceNumber": observation.sequence_number,
                    "expectedExtId": observation.expected_ext_id,
                    "packetSeenCount": observation.packet_seen_count,
                    "missingCount": observation.missing_count,
                    "observedAtMs": observation.observed_at_ms,
                }),
            );
        }
    }

    if let Some(observation) = stats
        .latest_data_channel_message_catalog_observation
        .as_ref()
    {
        if observation_state.data_channel_catalog_observation_id != Some(observation.observation_id)
        {
            observation_state.data_channel_catalog_observation_id =
                Some(observation.observation_id);
            runtime_trace.record_event(
                "xbxengine",
                "channelMessageCatalog",
                session_id,
                json!({
                    "observationId": observation.observation_id,
                    "direction": observation.direction,
                    "channel": observation.channel,
                    "kindType": observation.kind_type,
                    "kindMessage": observation.kind_message,
                    "target": observation.target,
                    "keys": observation.keys,
                    "payloadLen": observation.payload_len,
                    "observedAtMs": observation.observed_at_ms,
                }),
            );
        }
    }

    if observation_state.keyframe_request_episode != stats.latest_keyframe_request_episode {
        observation_state.keyframe_request_episode = stats.latest_keyframe_request_episode.clone();
        let payload =
            keyframe_request_episode_payload(stats.latest_keyframe_request_episode.as_ref());
        runtime_trace.record_state(
            "xbxengine",
            "keyframeRequestEpisode",
            session_id,
            payload.clone(),
        );
        if let Some(episode) = stats.latest_keyframe_request_episode.as_ref() {
            if let Some(event_name) = keyframe_request_episode_event_name(&episode.status) {
                runtime_trace.record_event("xbxengine", event_name, session_id, payload);
            }
        }
    }

    if observation_state.h264_inspection_observation != stats.latest_h264_inspection_observation {
        observation_state.h264_inspection_observation =
            stats.latest_h264_inspection_observation.clone();
        if let Some(inspection) = stats.latest_h264_inspection_observation.as_ref() {
            let payload = h264_inspection_payload(Some(inspection));
            runtime_trace.record_state("xbxengine", "h264Inspection", session_id, payload.clone());
            let event_name = if inspection.admission_accepted {
                "h264InspectionObserved"
            } else {
                "h264InspectionRejected"
            };
            runtime_trace.record_event("xbxengine", event_name, session_id, payload);
        }
    }

    let current_video_owner_observed_at_bucket = sample_bucket_ms(
        stats.video_owner_observed_at_ms,
        DIRECT_GAMING_STATE_SAMPLE_INTERVAL_MS,
    );
    if observation_state.session_phase != stats.session_phase
        || observation_state.remote_profile_baseline != stats.remote_profile_baseline
        || observation_state.remote_profile_dynamic != stats.remote_profile_dynamic
        || observation_state.remote_profile_effective_label != stats.remote_profile_effective_label
        || observation_state.transport_strategy_profile != stats.transport_strategy_profile
        || observation_state.recovery_strategy_profile != stats.recovery_strategy_profile
        || observation_state.recovery_diagnosis != stats.recovery_diagnosis
        || observation_state.direct_gaming_bitrate_band != stats.direct_gaming_bitrate_band
        || observation_state.runtime_summary != stats.runtime_summary
        || observation_state.primary_issue_chain != stats.primary_issue_chain
        || observation_state.latest_decision_summary != stats.latest_decision_summary
        || observation_state.recovery_owner_state != stats.recovery_owner_state
        || observation_state.recovery_owner_reason != stats.recovery_owner_reason
        || observation_state.video_owner_source != stats.video_owner_source
        || observation_state.video_owner_observed_at_bucket
            != current_video_owner_observed_at_bucket
        || observation_state.unified_lifecycle.as_deref() != Some(resolve_unified_lifecycle(stats))
        || observation_state.video_health != stats.video_health
        || observation_state.stall_kind != stats.stall_kind
    {
        observation_state.session_phase = stats.session_phase.clone();
        observation_state.remote_profile_baseline = stats.remote_profile_baseline.clone();
        observation_state.remote_profile_dynamic = stats.remote_profile_dynamic.clone();
        observation_state.remote_profile_effective_label =
            stats.remote_profile_effective_label.clone();
        observation_state.transport_strategy_profile = stats.transport_strategy_profile.clone();
        observation_state.recovery_strategy_profile = stats.recovery_strategy_profile.clone();
        observation_state.recovery_diagnosis = stats.recovery_diagnosis.clone();
        observation_state.direct_gaming_bitrate_band = stats.direct_gaming_bitrate_band.clone();
        observation_state.runtime_summary = stats.runtime_summary.clone();
        observation_state.primary_issue_chain = stats.primary_issue_chain.clone();
        observation_state.latest_decision_summary = stats.latest_decision_summary.clone();
        observation_state.recovery_owner_state = stats.recovery_owner_state.clone();
        observation_state.recovery_owner_reason = stats.recovery_owner_reason.clone();
        observation_state.video_owner_source = stats.video_owner_source.clone();
        observation_state.video_owner_observed_at_ms = stats.video_owner_observed_at_ms;
        observation_state.video_owner_observed_at_bucket = current_video_owner_observed_at_bucket;
        observation_state.unified_lifecycle = Some(resolve_unified_lifecycle(stats).to_string());
        observation_state.video_health = stats.video_health.clone();
        observation_state.stall_kind = stats.stall_kind.clone();
        runtime_trace.record_state(
            "xbxengine",
            "directGamingState",
            session_id,
            json!({
                "lifecycle": resolve_unified_lifecycle(stats),
                "streamLifecyclePhase": resolve_unified_lifecycle(stats),
                "sessionPhase": stats.session_phase,
                "remoteProfileBaseline": stats.remote_profile_baseline,
                "remoteProfileDynamic": stats.remote_profile_dynamic,
                "remoteProfileEffectiveLabel": stats.remote_profile_effective_label,
                "transportStrategyProfile": stats.transport_strategy_profile,
                "recoveryStrategyProfile": stats.recovery_strategy_profile,
                "recoveryDiagnosis": stats.recovery_diagnosis,
                "directGamingBitrateBand": stats.direct_gaming_bitrate_band,
                "runtimeSummary": stats.runtime_summary,
                "primaryIssueChain": stats.primary_issue_chain,
                "latestDecisionSummary": stats.latest_decision_summary,
                "videoOwnerState": stats.recovery_owner_state,
                "videoOwnerReason": stats.recovery_owner_reason,
                "videoOwnerSource": stats.video_owner_source,
                "videoOwnerObservedAtMs": stats.video_owner_observed_at_ms,
                "videoHealth": stats.video_health,
                "stallKind": stats.stall_kind,
            }),
        );
    }

    let host_present_semantic_changed = observation_state.host_no_pending_pressure_level
        != stats.host_no_pending_pressure_level
        || observation_state.host_cadence_phase != stats.host_cadence_phase
        || observation_state.host_descriptor_upload_mode
            != stats.video_present_descriptor_upload_mode;
    let host_present_counter_regressed = observation_state
        .host_present_enqueue_count_total
        .zip(stats.video_present_submit_count_total)
        .is_some_and(|(previous, current)| current < previous)
        || observation_state
            .host_present_drop_count_total
            .zip(stats.video_present_drop_count_total)
            .is_some_and(|(previous, current)| current < previous)
        || observation_state
            .host_present_overwrite_count_total
            .zip(stats.video_present_overwrite_count_total)
            .is_some_and(|(previous, current)| current < previous)
        || observation_state
            .host_no_pending_take_count_total
            .zip(stats.host_no_pending_take_count_total)
            .is_some_and(|(previous, current)| current < previous);
    let host_present_sample_due = observation_state
        .host_display_tick_epoch
        .zip(stats.host_display_tick_epoch)
        .is_none_or(|(previous, current)| {
            current.saturating_sub(previous) >= HOST_PRESENT_STATE_SAMPLE_EPOCH_INTERVAL
        });
    if host_present_semantic_changed
        || host_present_counter_regressed
        || host_present_sample_due
        || observation_state.host_display_tick_epoch.is_none()
        || stats.host_display_tick_epoch.is_none()
    {
        observation_state.host_present_enqueue_count_total = stats.video_present_submit_count_total;
        observation_state.host_present_drop_count_total = stats.video_present_drop_count_total;
        observation_state.host_present_overwrite_count_total =
            stats.video_present_overwrite_count_total;
        observation_state.host_no_pending_take_count_total = stats.host_no_pending_take_count_total;
        observation_state.host_no_pending_streak = stats.host_no_pending_streak;
        observation_state.host_no_pending_max_streak = stats.host_no_pending_max_streak;
        observation_state.host_no_pending_pressure_level =
            stats.host_no_pending_pressure_level.clone();
        observation_state.host_display_tick_epoch = stats.host_display_tick_epoch;
        observation_state.host_present_epoch = stats.video_present_epoch;
        observation_state.host_cadence_phase = stats.host_cadence_phase.clone();
        observation_state.host_descriptor_upload_mode =
            stats.video_present_descriptor_upload_mode.clone();
        observation_state.host_descriptor_metal_import_count_total =
            stats.video_present_descriptor_metal_import_count_total;
        observation_state.host_descriptor_cpu_upload_count_total =
            stats.video_present_descriptor_cpu_upload_count_total;
        runtime_trace.record_state(
            "xbxengine",
            "hostPresentState",
            session_id,
            json!({
                "presentFps": stats.present_fps,
                "presentEnqueueCountTotal": stats.video_present_submit_count_total,
                "presentDropCountTotal": stats.video_present_drop_count_total,
                "presentOverwriteCountTotal": stats.video_present_overwrite_count_total,
                "noPendingTakeCountTotal": stats.host_no_pending_take_count_total,
                "noPendingStreak": stats.host_no_pending_streak,
                "noPendingMaxStreak": stats.host_no_pending_max_streak,
                "noPendingPressureLevel": stats.host_no_pending_pressure_level,
                "displayTickEpoch": stats.host_display_tick_epoch,
                "presentEpoch": stats.video_present_epoch,
                "cadencePhase": stats.host_cadence_phase,
                "presentAgeMs": stats.present_age_ms,
                "descriptorUploadMode": stats.video_present_descriptor_upload_mode,
                "descriptorMetalImportCountTotal": stats.video_present_descriptor_metal_import_count_total,
                "descriptorCpuUploadCountTotal": stats.video_present_descriptor_cpu_upload_count_total,
            }),
        );
    }

    if observation_state.recovery_keyframe_request_count != stats.recovery_keyframe_request_count {
        observation_state.recovery_keyframe_request_count = stats.recovery_keyframe_request_count;
        if let Some(count) = stats.recovery_keyframe_request_count {
            if count > 0 && stats.last_recovery_action.as_deref() == Some("keyframe") {
                runtime_trace.record_decision(
                    "xbxengine",
                    "keyframeRequested",
                    session_id,
                    json!({
                        "count": count,
                        "atMs": stats.last_recovery_action_at_ms,
                        "reason": stats.last_recovery_reason,
                    }),
                );
            }
        }
    }

    if observation_state.recovery_decoder_reset_count != stats.recovery_decoder_reset_count {
        observation_state.recovery_decoder_reset_count = stats.recovery_decoder_reset_count;
        if let Some(count) = stats.recovery_decoder_reset_count {
            if count > 0 && stats.last_recovery_action.as_deref() == Some("decoderReset") {
                runtime_trace.record_decision(
                    "xbxengine",
                    "decoderResetRequested",
                    session_id,
                    json!({
                        "count": count,
                        "atMs": stats.last_recovery_action_at_ms,
                        "reason": stats.last_recovery_reason,
                    }),
                );
            }
        }
    }

    if observation_state.recovery_reconnect_count != stats.recovery_reconnect_count {
        observation_state.recovery_reconnect_count = stats.recovery_reconnect_count;
        if let Some(count) = stats.recovery_reconnect_count {
            if count > 0 && stats.last_recovery_action.as_deref() == Some("reconnect") {
                runtime_trace.record_decision(
                    "xbxengine",
                    "mediaTransportReconnect",
                    session_id,
                    json!({
                        "count": count,
                        "atMs": stats.last_recovery_action_at_ms,
                        "reason": stats.last_recovery_reason,
                        "reconnectTriggerSource": stats.reconnect_trigger_source,
                    }),
                );
            }
        }
    }
    if observation_state.recovery_hard_fallback_timer_ms != stats.recovery_hard_fallback_timer_ms
        || observation_state.recovery_hard_fallback_trigger_reason
            != stats.recovery_hard_fallback_trigger_reason
        || observation_state.recovery_hard_fallback_timer_reset_reason
            != stats.recovery_hard_fallback_timer_reset_reason
    {
        observation_state.recovery_hard_fallback_timer_ms = stats.recovery_hard_fallback_timer_ms;
        observation_state.recovery_hard_fallback_trigger_reason =
            stats.recovery_hard_fallback_trigger_reason.clone();
        observation_state.recovery_hard_fallback_timer_reset_reason =
            stats.recovery_hard_fallback_timer_reset_reason.clone();
        runtime_trace.record_state(
            "xbxengine",
            "recoveryHardFallbackState",
            session_id,
            json!({
                "timerMs": stats.recovery_hard_fallback_timer_ms,
                "triggerReason": stats.recovery_hard_fallback_trigger_reason,
                "resetReason": stats.recovery_hard_fallback_timer_reset_reason,
            }),
        );
    }

    if observation_state.transport_state != stats.transport_state
        || observation_state.transport_path != stats.transport_path
        || observation_state.transport_candidate_pair != stats.transport_candidate_pair
        || observation_state.transport_protocol != stats.transport_protocol
        || observation_state.transport_address_family != stats.transport_address_family
    {
        observation_state.transport_state = stats.transport_state.clone();
        observation_state.transport_path = stats.transport_path.clone();
        observation_state.transport_candidate_pair = stats.transport_candidate_pair.clone();
        observation_state.transport_protocol = stats.transport_protocol.clone();
        observation_state.transport_address_family = stats.transport_address_family.clone();
        runtime_trace.record_state(
            "xbxengine",
            "transportObservation",
            session_id,
            json!({
                "transportState": stats.transport_state,
                "transportPath": stats.transport_path,
                "transportCandidatePair": stats.transport_candidate_pair,
                "transportProtocol": stats.transport_protocol,
                "transportAddressFamily": stats.transport_address_family,
            }),
        );
    }

    if observation_state.actual_video_bitrate_source != stats.actual_video_bitrate_source {
        observation_state.actual_video_bitrate_source = stats.actual_video_bitrate_source.clone();
        runtime_trace.record_state(
            "xbxengine",
            "actualVideoBitrateSource",
            session_id,
            json!({
                "source": stats.actual_video_bitrate_source,
            }),
        );
    }

    if observation_state.twcc_observation_state != stats.twcc_observation_state {
        observation_state.twcc_observation_state = stats.twcc_observation_state.clone();
        runtime_trace.record_state(
            "xbxengine",
            "twccObservationState",
            session_id,
            json!({
                "state": stats.twcc_observation_state,
            }),
        );
    }

    if observation_state.latest_observation_label != stats.latest_observation_label
        || observation_state.latest_observation_summary != stats.latest_observation_summary
    {
        observation_state.latest_observation_label = stats.latest_observation_label.clone();
        observation_state.latest_observation_summary = stats.latest_observation_summary.clone();
    }

    if observation_state.latest_target_remb_action != stats.latest_target_remb_action
        || observation_state.latest_target_remb_summary != stats.latest_target_remb_summary
    {
        observation_state.latest_target_remb_action = stats.latest_target_remb_action.clone();
        observation_state.latest_target_remb_summary = stats.latest_target_remb_summary.clone();
        match stats.latest_target_remb_action.as_deref() {
            Some("requested") => {
                runtime_trace.record_event(
                    "xbxengine",
                    "rtcTargetRembRequested",
                    session_id,
                    json!({
                        "summary": stats.latest_target_remb_summary,
                    }),
                );
            }
            Some("queued") => {
                runtime_trace.record_event(
                    "xbxengine",
                    "rtcTargetRembQueued",
                    session_id,
                    json!({
                        "summary": stats.latest_target_remb_summary,
                    }),
                );
            }
            _ => {}
        }
    }

    if let Some(status) = stats.latest_video_track_status.as_ref() {
        let signature = (
            status.state.clone(),
            status.video_width,
            status.video_height,
            status.mime_type.clone(),
            status.transport_state.clone(),
        );
        let bucket = sample_bucket_ms(
            Some(status.observed_at_ms),
            VIDEO_TRACK_STATE_SAMPLE_INTERVAL_MS,
        );
        let identity_changed =
            observation_state.video_track_state_signature.as_ref() != Some(&signature);
        let sampled_tick = observation_state.video_track_state_bucket != bucket;
        if identity_changed || sampled_tick {
            observation_state.video_track_state_signature = Some(signature);
            observation_state.video_track_state_bucket = bucket;
            observation_state.latest_video_track_status = Some(status.clone());
            runtime_trace.record_state(
                "xbxengine",
                "videoTrackState",
                session_id,
                json!({
                    "state": status.state,
                    "videoWidth": status.video_width,
                    "videoHeight": status.video_height,
                    "mimeType": status.mime_type,
                    "transportState": status.transport_state,
                    "videoBytesTotal": status.video_bytes_total,
                    "videoPacketCountTotal": status.video_packet_count_total,
                    "audioBytesTotal": status.audio_bytes_total,
                    "observedAtMs": status.observed_at_ms,
                }),
            );
        }
    } else {
        observation_state.latest_video_track_status = None;
        observation_state.video_track_state_signature = None;
        observation_state.video_track_state_bucket = None;
    }

    if observation_state.video_decoder_stalled != stats.video_decoder_stalled {
        let previous_stalled = observation_state.video_decoder_stalled;
        observation_state.video_decoder_stalled = stats.video_decoder_stalled;
        runtime_trace.record_event(
            "xbxengine",
            "videoDecoderStallTransition",
            session_id,
            json!({
                "stalled": stats.video_decoder_stalled.unwrap_or(false),
                "previousStalled": previous_stalled,
                "packetAgeMs": stats.packet_age_ms,
                "decodeAgeMs": stats.decode_age_ms,
                "presentAgeMs": stats.present_age_ms,
                "observedAtMs": 0.0_f64,
            }),
        );
    }

    if observation_state.video_renderer_stalled != stats.video_renderer_stalled {
        let previous_stalled = observation_state.video_renderer_stalled;
        observation_state.video_renderer_stalled = stats.video_renderer_stalled;
        runtime_trace.record_event(
            "xbxengine",
            "videoRendererStallTransition",
            session_id,
            json!({
                "stalled": stats.video_renderer_stalled.unwrap_or(false),
                "previousStalled": previous_stalled,
                "packetAgeMs": stats.packet_age_ms,
                "decodeAgeMs": stats.decode_age_ms,
                "presentAgeMs": stats.present_age_ms,
                "observedAtMs": 0.0_f64,
            }),
        );
    }

    if observation_state.video_remb_bps != stats.video_remb_bps {
        observation_state.video_remb_bps = stats.video_remb_bps;
        if let Some(video_remb_bps) = stats.video_remb_bps {
            runtime_trace.record_state(
                "xbxengine",
                "rembUpdated",
                session_id,
                json!({
                    "videoRembBps": video_remb_bps,
                }),
            );
        }
    }
}

fn keyframe_request_episode_event_name(status: &str) -> Option<&'static str> {
    match status {
        "requested" => Some("keyframeRequestEpisodeRequested"),
        "sent" => Some("keyframeRequestEpisodeSent"),
        "packet-seen" => Some("keyframeRequestEpisodePacketSeen"),
        "decoded" => Some("keyframeRequestEpisodeDecoded"),
        "missed" => Some("keyframeRequestEpisodeMissed"),
        _ => None,
    }
}

fn keyframe_request_episode_payload(
    episode: Option<&xbxengine_protocol::XbxEngineKeyframeRequestEpisodeObservationDto>,
) -> serde_json::Value {
    match episode {
        Some(episode) => json!({
            "episodeId": episode.episode_id,
            "requestReason": episode.request_reason.clone(),
            "requestKind": episode.request_kind.clone(),
            "status": episode.status.clone(),
            "requestedAtMs": episode.requested_at_ms,
            "sentAtMs": episode.sent_at_ms,
            "deadlineAtMs": episode.deadline_at_ms,
            "firstKeyframePacketAtMs": episode.first_keyframe_packet_at_ms,
            "firstKeyframeDecodedAtMs": episode.first_keyframe_decoded_at_ms,
            "responseRtpTimestamp": episode.response_rtp_timestamp,
            "responseFrameSeq": episode.response_frame_seq,
            "responseVerdict": episode.response_verdict.clone(),
        }),
        None => json!(null),
    }
}

fn h264_inspection_payload(
    observation: Option<&xbxengine_protocol::XbxEngineH264InspectionObservationDto>,
) -> serde_json::Value {
    match observation {
        Some(observation) => json!({
            "observationId": observation.observation_id,
            "frameRtpTimestamp": observation.frame_rtp_timestamp,
            "nalTypes": observation.nal_types.clone(),
            "hasInbandSps": observation.has_inband_sps,
            "hasInbandPps": observation.has_inband_pps,
            "committedSpsPresent": observation.committed_sps_present,
            "committedPpsPresent": observation.committed_pps_present,
            "sliceHeadersValid": observation.slice_headers_valid,
            "deltaContinuationReady": observation.delta_continuation_ready,
            "parameterSetsChanged": observation.parameter_sets_changed,
            "configChanged": observation.config_changed,
            "isIdr": observation.is_idr,
            "bootstrapReady": observation.bootstrap_ready,
            "bootstrapRejectReason": observation.bootstrap_reject_reason.clone(),
            "admissionAccepted": observation.admission_accepted,
            "observedAtMs": observation.observed_at_ms,
        }),
        None => json!(null),
    }
}

fn is_timeout_source_event(source_event: &str) -> bool {
    source_event.starts_with("timeout-")
}

fn is_chain_transition_source_event(source_event: &str) -> bool {
    matches!(
        source_event,
        "chain-broken" | "chain-recovery-keyframe-requested" | "chain-clean-keyframe-submitted"
    )
}

fn is_chain_flush_source_event(source_event: &str) -> bool {
    source_event == "gap-expired-chain-flush"
}

fn sample_bucket_ms(value: Option<f64>, interval_ms: f64) -> Option<u64> {
    let observed_at_ms = value?;
    if !observed_at_ms.is_finite() || observed_at_ms < 0.0 || interval_ms <= 0.0 {
        return None;
    }
    Some((observed_at_ms / interval_ms).floor() as u64)
}

#[cfg(test)]
#[path = "trace_projection.test.rs"]
mod tests;
