use serde_json::json;
use xbxengine_protocol::XbxEngineStatsDto;

use crate::mods::runtime_trace::RuntimeTraceRecorderRef;

#[derive(Default)]
pub(super) struct RuntimeTraceObservationState {
    packet_gap_observation_id: Option<u64>,
    frame_drop_observation_id: Option<u64>,
    nack_observation_id: Option<u64>,
    escalation_observation_id: Option<u64>,
    bwe_observation_id: Option<u64>,
    twcc_observation_id: Option<u64>,
    data_channel_catalog_observation_id: Option<u64>,
    recovery_keyframe_request_count: Option<u64>,
    recovery_decoder_reset_count: Option<u64>,
    recovery_reconnect_count: Option<u64>,
    transport_state: Option<String>,
    transport_path: Option<String>,
    latest_video_track_status: Option<xbxengine_protocol::XbxEngineVideoTrackStatusDto>,
    video_remb_bps: Option<u32>,
    session_phase: Option<String>,
    transport_policy_profile: Option<String>,
    recovery_policy_profile: Option<String>,
    recovery_diagnosis: Option<String>,
    recovery_coupling_mode: Option<String>,
    recovery_coupling_summary: Option<String>,
    direct_gaming_bitrate_band: Option<String>,
    runtime_summary: Option<String>,
    primary_issue_chain: Option<String>,
    latest_decision_summary: Option<String>,
    video_health: Option<String>,
    stall_kind: Option<String>,
    host_present_submit_count_total: Option<u64>,
    host_present_drop_count_total: Option<u64>,
    host_present_overwrite_count_total: Option<u64>,
    host_descriptor_upload_mode: Option<String>,
    host_descriptor_metal_import_count_total: Option<u64>,
    host_descriptor_cpu_upload_count_total: Option<u64>,
}

pub(super) fn should_skip_trace_tick(session_id: Option<&str>, stats: &XbxEngineStatsDto) -> bool {
    session_id.is_none() && stats.transport_state.as_deref() == Some("Closed")
}

/// 统一观测快照：把 UI 与离线分析真正关心的状态压成单条 snapshot，避免继续手工拼
/// `statsSnapshot + directGamingState + hostPresentState`。
pub(super) fn build_observability_snapshot(stats: &XbxEngineStatsDto) -> serde_json::Value {
    json!({
        "resolution": stats.resolution,
        "fps": stats.fps,
        "rtt": stats.rtt,
        "runtimeSummary": stats.runtime_summary,
        "primaryIssueChain": stats.primary_issue_chain,
        "latestDecisionSummary": stats.latest_decision_summary,
        "transport": {
            "path": stats.transport_path,
            "state": stats.transport_state,
            "policyProfile": stats.transport_policy_profile,
            "videoRttSource": stats.video_rtt_source,
            "videoRembBps": stats.video_remb_bps,
        },
        "recovery": {
            "sessionPhase": stats.session_phase,
            "policyProfile": stats.recovery_policy_profile,
            "diagnosis": stats.recovery_diagnosis,
            "couplingMode": stats.recovery_coupling_mode,
            "couplingSummary": stats.recovery_coupling_summary,
            "videoHealth": stats.video_health,
            "stallKind": stats.stall_kind,
            "keyframeRequestCount": stats.recovery_keyframe_request_count,
            "decoderResetCount": stats.recovery_decoder_reset_count,
            "reconnectCount": stats.recovery_reconnect_count,
            "lastAction": stats.last_recovery_action,
            "lastActionAtMs": stats.last_recovery_action_at_ms,
            "lastReason": stats.last_recovery_reason,
        },
        "directGaming": {
            "bitrateBand": stats.direct_gaming_bitrate_band,
        },
        "bitrate": {
            "display": stats.br,
            "inboundKbps": stats.inbound_bitrate_kbps,
            "videoKbps": stats.inbound_video_bitrate_kbps,
            "audioKbps": stats.inbound_audio_bitrate_kbps,
            "bytesTotal": stats.inbound_bytes_total,
            "videoBytesTotal": stats.inbound_video_bytes_total,
            "audioBytesTotal": stats.inbound_audio_bytes_total,
        },
        "video": {
            "inboundFps": stats.inbound_video_fps,
            "decodeFps": stats.decode_fps,
            "presentFps": stats.present_fps,
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
            "presentSubmitCountTotal": stats.video_present_submit_count_total,
            "presentDropCountTotal": stats.video_present_drop_count_total,
            "presentOverwriteCountTotal": stats.video_present_overwrite_count_total,
            "descriptorUploadMode": stats.video_present_descriptor_upload_mode,
            "descriptorMetalImportCountTotal": stats.video_present_descriptor_metal_import_count_total,
            "descriptorCpuUploadCountTotal": stats.video_present_descriptor_cpu_upload_count_total,
        },
        "latest": {
            "packetGap": stats.latest_video_packet_gap,
            "frameDrop": stats.latest_video_frame_drop,
            "nack": stats.latest_video_nack_observation,
            "escalation": stats.latest_video_escalation_observation,
            "bwe": stats.latest_video_bwe_observation,
            "twcc": stats.latest_video_twcc_observation,
        },
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
                    "observedAtMs": frame_drop.observed_at_ms,
                    "width": frame_drop.width,
                    "height": frame_drop.height,
                    "isKeyframe": frame_drop.is_keyframe,
                    "queueDepth": frame_drop.queue_depth,
                }),
            );
        }
    }

    if let Some(nack) = stats.latest_video_nack_observation.as_ref() {
        if observation_state.nack_observation_id != Some(nack.observation_id) {
            observation_state.nack_observation_id = Some(nack.observation_id);
            let event_name = match nack.action.as_str() {
                "expiredDeadline" | "expiredMaxAge" => "nackExpired",
                "recovered" | "recoveredLate" => "nackRecovered",
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
                    "observedAtMs": nack.observed_at_ms,
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
                    "observedAtMs": escalation.observed_at_ms,
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
                    "actualVideoBitrateKbps": bwe.actual_video_bitrate_kbps,
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
            runtime_trace.record_event(
                "xbxengine",
                "twccFeedbackSent",
                session_id,
                json!({
                    "observationId": twcc.observation_id,
                    "feedbackPacketCount": twcc.feedback_packet_count,
                    "coveredSequenceStart": twcc.covered_sequence_start,
                    "coveredSequenceEnd": twcc.covered_sequence_end,
                    "coveredSequenceSpan": twcc.covered_sequence_span,
                    "observedPacketCount": twcc.observed_packet_count,
                    "observedByteCount": twcc.observed_byte_count,
                    "feedbackIntervalMs": twcc.feedback_interval_ms,
                    "arrivalSpanMs": twcc.arrival_span_ms,
                    "receiveBitrateKbps": twcc.receive_bitrate_kbps,
                    "deliveryRatio": twcc.delivery_ratio,
                    "packetLossRatio": twcc.packet_loss_ratio,
                    "observedAtMs": twcc.observed_at_ms,
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

    if observation_state.session_phase != stats.session_phase
        || observation_state.transport_policy_profile != stats.transport_policy_profile
        || observation_state.recovery_policy_profile != stats.recovery_policy_profile
        || observation_state.recovery_diagnosis != stats.recovery_diagnosis
        || observation_state.recovery_coupling_mode != stats.recovery_coupling_mode
        || observation_state.recovery_coupling_summary != stats.recovery_coupling_summary
        || observation_state.direct_gaming_bitrate_band != stats.direct_gaming_bitrate_band
        || observation_state.runtime_summary != stats.runtime_summary
        || observation_state.primary_issue_chain != stats.primary_issue_chain
        || observation_state.latest_decision_summary != stats.latest_decision_summary
        || observation_state.video_health != stats.video_health
        || observation_state.stall_kind != stats.stall_kind
    {
        observation_state.session_phase = stats.session_phase.clone();
        observation_state.transport_policy_profile = stats.transport_policy_profile.clone();
        observation_state.recovery_policy_profile = stats.recovery_policy_profile.clone();
        observation_state.recovery_diagnosis = stats.recovery_diagnosis.clone();
        observation_state.recovery_coupling_mode = stats.recovery_coupling_mode.clone();
        observation_state.recovery_coupling_summary = stats.recovery_coupling_summary.clone();
        observation_state.direct_gaming_bitrate_band = stats.direct_gaming_bitrate_band.clone();
        observation_state.runtime_summary = stats.runtime_summary.clone();
        observation_state.primary_issue_chain = stats.primary_issue_chain.clone();
        observation_state.latest_decision_summary = stats.latest_decision_summary.clone();
        observation_state.video_health = stats.video_health.clone();
        observation_state.stall_kind = stats.stall_kind.clone();
        runtime_trace.record_state(
            "xbxengine",
            "directGamingState",
            session_id,
            json!({
                "sessionPhase": stats.session_phase,
                "transportPolicyProfile": stats.transport_policy_profile,
                "recoveryPolicyProfile": stats.recovery_policy_profile,
                "recoveryDiagnosis": stats.recovery_diagnosis,
                "recoveryCouplingMode": stats.recovery_coupling_mode,
                "recoveryCouplingSummary": stats.recovery_coupling_summary,
                "directGamingBitrateBand": stats.direct_gaming_bitrate_band,
                "runtimeSummary": stats.runtime_summary,
                "primaryIssueChain": stats.primary_issue_chain,
                "latestDecisionSummary": stats.latest_decision_summary,
                "videoHealth": stats.video_health,
                "stallKind": stats.stall_kind,
            }),
        );
    }

    if observation_state.host_present_submit_count_total != stats.video_present_submit_count_total
        || observation_state.host_present_drop_count_total != stats.video_present_drop_count_total
        || observation_state.host_present_overwrite_count_total
            != stats.video_present_overwrite_count_total
        || observation_state.host_descriptor_upload_mode
            != stats.video_present_descriptor_upload_mode
        || observation_state.host_descriptor_metal_import_count_total
            != stats.video_present_descriptor_metal_import_count_total
        || observation_state.host_descriptor_cpu_upload_count_total
            != stats.video_present_descriptor_cpu_upload_count_total
    {
        observation_state.host_present_submit_count_total = stats.video_present_submit_count_total;
        observation_state.host_present_drop_count_total = stats.video_present_drop_count_total;
        observation_state.host_present_overwrite_count_total =
            stats.video_present_overwrite_count_total;
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
                "presentSubmitCountTotal": stats.video_present_submit_count_total,
                "presentDropCountTotal": stats.video_present_drop_count_total,
                "presentOverwriteCountTotal": stats.video_present_overwrite_count_total,
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
    }

    if observation_state.transport_state != stats.transport_state
        || observation_state.transport_path != stats.transport_path
    {
        observation_state.transport_state = stats.transport_state.clone();
        observation_state.transport_path = stats.transport_path.clone();
        runtime_trace.record_state(
            "xbxengine",
            "transportObservation",
            session_id,
            json!({
                "transportState": stats.transport_state,
                "transportPath": stats.transport_path,
            }),
        );
    }

    if observation_state.latest_video_track_status != stats.latest_video_track_status {
        observation_state.latest_video_track_status = stats.latest_video_track_status.clone();
        if let Some(status) = stats.latest_video_track_status.as_ref() {
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
