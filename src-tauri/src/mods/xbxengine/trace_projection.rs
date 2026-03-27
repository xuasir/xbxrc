use serde_json::json;
use xbxengine_protocol::XbxEngineStatsDto;

use crate::mods::runtime_trace::RuntimeTraceRecorderRef;

#[derive(Default)]
pub(super) struct RuntimeTraceObservationState {
    packet_gap_observation_id: Option<u64>,
    frame_drop_observation_id: Option<u64>,
    frame_recovery_observation_id: Option<u64>,
    nack_observation_id: Option<u64>,
    escalation_observation_id: Option<u64>,
    bwe_observation_id: Option<u64>,
    twcc_observation_id: Option<u64>,
    rtc_builder_observation_id: Option<u64>,
    twcc_remote_stream_observation_id: Option<u64>,
    remote_answer_observation_id: Option<u64>,
    twcc_extension_observation_id: Option<u64>,
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
    actual_video_bitrate_source: Option<String>,
    twcc_observation_state: Option<String>,
    latest_observation_label: Option<String>,
    latest_observation_summary: Option<String>,
    latest_target_remb_action: Option<String>,
    latest_target_remb_summary: Option<String>,
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
            "videoDisplay": stats.br,
            "totalKbps": stats.inbound_bitrate_kbps,
            "inboundKbps": stats.inbound_bitrate_kbps,
            "videoKbps": stats.inbound_video_bitrate_kbps,
            "audioKbps": stats.inbound_audio_bitrate_kbps,
            "actualVideoKbps": stats.video_actual_bitrate_kbps,
            // legacy: 保留兼容镜像字段，值与 actualVideoBitrateSource 必须一致。
            "actualVideoSource": stats.actual_video_bitrate_source,
            "actualVideoBitrateSource": stats.actual_video_bitrate_source,
            "bytesTotal": stats.inbound_bytes_total,
            "videoBytesTotal": stats.inbound_video_bytes_total,
            "audioBytesTotal": stats.inbound_audio_bytes_total,
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
            "frameRecovery": stats.latest_video_frame_recovery_observation,
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
                    "observedAtMs": frame_recovery.observed_at_ms,
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
                    "estimatedRecoveryArrivalMs": nack.estimated_recovery_arrival_ms,
                    "nackDisposition": nack.nack_disposition,
                    "framePlayoutDeadlineAtMs": nack.frame_playout_deadline_at_ms,
                    "frameUnrecoverableReason": nack.frame_unrecoverable_reason,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mods::runtime_trace::RuntimeTraceRecorder;
    use serde_json::json;
    use std::fs;

    fn test_stats(payload: serde_json::Value) -> XbxEngineStatsDto {
        serde_json::from_value(payload).expect("valid stats dto")
    }

    #[test]
    fn build_observability_snapshot_includes_source_state_and_build_fingerprint() {
        let stats = test_stats(json!({
            "resolution": "2560x1440",
            "rtt": "81.0ms",
            "fps": 60.0,
            "pl": "0.00%",
            "fl": "",
            "jit": "1.0ms",
            "br": "8.5Mbps",
            "decode": "",
            "inbound_bitrate_kbps": 8600.0,
            "inbound_video_bitrate_kbps": 8400.0,
            "inbound_audio_bitrate_kbps": 200.0,
            "actual_video_bitrate_source": "transport-metrics",
            "video_actual_bitrate_kbps": 8400.0,
            "video_twcc_receive_bitrate_kbps": 22800.0,
            "video_twcc_loss_ratio": 0.01,
            "video_twcc_delivery_ratio": 0.99,
            "video_twcc_feedback_interval_ms": 80.0,
            "twcc_observation_state": "missing-local-feedback",
            "build_fingerprint": {
                "gitCommitShort": "abc1234",
                "workspaceDirty": true,
                "buildTimestampUnixMs": "1774405700000",
                "cargoProfile": "debug",
                "defaultFeedbackIntervalMs": 1000,
                "effectiveFeedbackIntervalMs": 80,
                "controlledTwccRegistry": true
            }
        }));

        let snapshot = build_observability_snapshot(&stats);
        assert_eq!(
            snapshot["bitrate"]["actualVideoSource"],
            "transport-metrics"
        );
        assert_eq!(snapshot["bitrate"]["actualVideoKbps"], 8400.0);
        assert_eq!(
            snapshot["bwe"]["actualVideoBitrateSource"],
            "transport-metrics"
        );
        assert_eq!(snapshot["bwe"]["actualVideoKbps"], 8400.0);
        assert_eq!(snapshot["twcc"]["state"], "missing-local-feedback");
        assert_eq!(snapshot["buildFingerprint"]["gitCommitShort"], "abc1234");
    }

    #[test]
    fn build_observability_snapshot_includes_latest_frame_recovery() {
        let stats = test_stats(json!({
            "resolution": "",
            "rtt": "",
            "fps": 0.0,
            "pl": "0.00%",
            "fl": "",
            "jit": "",
            "br": "",
            "decode": "",
            "actual_video_bitrate_source": "unavailable",
            "video_actual_bitrate_kbps": 1019.4,
            "latest_video_frame_recovery_observation": {
                "observation_id": 77,
                "action": "ledgerConsume",
                "frame_rtp_timestamp": 123456789,
                "frame_playout_deadline_at_ms": 4567.0,
                "frame_recovery_disposition": "unrecoverable-reference-chain",
                "frame_unrecoverable_reason": "referenceChainUnrecoverable",
                "observed_at_ms": 1234.0
            }
        }));

        let snapshot = build_observability_snapshot(&stats);
        assert_eq!(snapshot["bitrate"]["actualVideoKbps"], 1019.4);
        assert_eq!(snapshot["bitrate"]["actualVideoBitrateSource"], "unavailable");
        assert_eq!(snapshot["bitrate"]["actualVideoSource"], "unavailable");
        assert_eq!(
            snapshot["latest"]["frameRecovery"]["action"],
            "ledgerConsume"
        );
        assert_eq!(
            snapshot["latest"]["frameRecovery"]["frame_recovery_disposition"],
            "unrecoverable-reference-chain"
        );
        assert_eq!(
            snapshot["latest"]["frameRecovery"]["frame_unrecoverable_reason"],
            "referenceChainUnrecoverable"
        );
    }

    #[test]
    fn build_observability_snapshot_projects_latest_twcc_sample_gate_fields() {
        let stats = test_stats(json!({
            "resolution": "",
            "rtt": "",
            "fps": 0.0,
            "pl": "0.00%",
            "fl": "",
            "jit": "",
            "br": "",
            "decode": "",
            "twcc_observation_state": "unavailable",
            "latest_video_twcc_observation": {
                "observation_id": 19,
                "source": "local-feedback",
                "feedback_packet_count": 1,
                "covered_sequence_start": 1,
                "covered_sequence_end": 8,
                "covered_sequence_span": 8,
                "observed_packet_count": 8,
                "observed_byte_count": 0,
                "coverage_ratio": 1.0,
                "ledger_hit_ratio": 0.0,
                "feedback_interval_ms": 600.0,
                "arrival_span_ms": 100.0,
                "receive_bitrate_kbps": null,
                "twcc_sample_valid": false,
                "twcc_invalid_reason": "missing-byte-ledger|interval-too-long:600.0",
                "quality": "delayed",
                "delivery_ratio": 1.0,
                "packet_loss_ratio": 0.0,
                "observed_at_ms": 1234.0
            }
        }));

        let snapshot = build_observability_snapshot(&stats);
        assert_eq!(snapshot["twcc"]["sampleValid"], false);
        assert_eq!(
            snapshot["twcc"]["invalidReason"],
            "missing-byte-ledger|interval-too-long:600.0"
        );
        assert_eq!(snapshot["twcc"]["coverageRatio"], 1.0);
        assert_eq!(snapshot["twcc"]["ledgerHitRatio"], 0.0);
    }

    #[test]
    fn record_runtime_trace_observations_uses_twcc_event_name_by_source() {
        let recorder = std::sync::Arc::new(RuntimeTraceRecorder::new().expect("trace recorder"));
        let mut local_state = RuntimeTraceObservationState::default();
        let local_stats = test_stats(json!({
            "resolution": "",
            "rtt": "",
            "fps": 0.0,
            "pl": "0.00%",
            "fl": "",
            "jit": "",
            "br": "",
            "decode": "",
            "twcc_observation_state": "local-feedback",
            "latest_video_twcc_observation": {
                "observation_id": 7,
                "source": "local-feedback",
                "feedback_packet_count": 3,
                "covered_sequence_start": 100,
                "covered_sequence_end": 220,
                "covered_sequence_span": 120,
                "observed_packet_count": 120,
                "observed_byte_count": 340000,
                "coverage_ratio": 1.0,
                "ledger_hit_ratio": 0.95,
                "feedback_interval_ms": 80.0,
                "arrival_span_ms": 70.0,
                "receive_bitrate_kbps": 22800.0,
                "delivery_ratio": 0.99,
                "packet_loss_ratio": 0.01,
                "observed_at_ms": 2.0
            }
        }));

        record_runtime_trace_observations(
            &recorder,
            &mut local_state,
            Some("session-1"),
            &local_stats,
        );
        let mut remote_state = RuntimeTraceObservationState::default();
        record_runtime_trace_observations(
            &recorder,
            &mut remote_state,
            Some("session-1"),
            &test_stats(json!({
                "resolution": "",
                "rtt": "",
                "fps": 0.0,
                "pl": "0.00%",
                "fl": "",
                "jit": "",
                "br": "",
                "decode": "",
                "twcc_observation_state": "remote-observed",
                "latest_video_twcc_observation": {
                    "observation_id": 8,
                    "source": "remote-rtcp",
                    "feedback_packet_count": 3,
                    "covered_sequence_start": 100,
                    "covered_sequence_end": 220,
                    "covered_sequence_span": 120,
                    "observed_packet_count": 120,
                    "observed_byte_count": 340000,
                    "coverage_ratio": 1.0,
                    "ledger_hit_ratio": null,
                    "feedback_interval_ms": 80.0,
                    "arrival_span_ms": 70.0,
                    "receive_bitrate_kbps": 22800.0,
                    "delivery_ratio": 0.99,
                    "packet_loss_ratio": 0.01,
                    "observed_at_ms": 2.0
                }
            })),
        );

        let contents = fs::read_to_string(recorder.path()).expect("trace contents");
        assert!(contents.contains("\"event\":\"twccFeedbackSent\""));
        assert!(contents.contains("\"event\":\"twccFeedbackObserved\""));
    }

    #[test]
    fn record_runtime_trace_observations_projects_remote_answer_acceptance() {
        let recorder = std::sync::Arc::new(RuntimeTraceRecorder::new().expect("trace recorder"));
        let mut state = RuntimeTraceObservationState::default();
        let stats = test_stats(json!({
            "resolution": "",
            "rtt": "",
            "fps": 0.0,
            "pl": "0.00%",
            "fl": "",
            "jit": "",
            "br": "",
            "decode": "",
            "latest_remote_answer_observation": {
                "observation_id": 11,
                "video_payload_order": [124, 97, 125],
                "selected_video_payload_type": 124,
                "selected_video_mime_type": "video/h264",
                "selected_video_profile_level_id": "4d002a",
                "accepted_video_rtcp_feedback": ["goog-remb", "transport-cc", "nack:pli"],
                "accepted_audio_rtcp_feedback": ["transport-cc"],
                "accepted_video_header_extensions": ["http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01#3"],
                "accepted_audio_header_extensions": ["urn:ietf:params:rtp-hdrext:ssrc-audio-level#2"],
                "observed_at_ms": 1234.0
            }
        }));
        record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &stats);

        let contents = fs::read_to_string(recorder.path()).expect("trace contents");
        assert!(contents.contains("\"event\":\"remoteAnswerAccepted\""));
        assert!(contents.contains("\"selectedVideoProfileLevelId\":\"4d002a\""));
        assert!(contents.contains("\"googRembAccepted\":true"));
        assert!(contents.contains("\"transportCcAccepted\":true"));
    }

    #[test]
    fn bwe_updated_event_uses_top_level_actual_video_bitrate() {
        let recorder = std::sync::Arc::new(RuntimeTraceRecorder::new().expect("trace recorder"));
        let mut state = RuntimeTraceObservationState::default();
        let stats = test_stats(json!({
            "resolution": "",
            "rtt": "199.7ms",
            "fps": 0.0,
            "pl": "0.00%",
            "fl": "",
            "jit": "",
            "br": "0.7Mbps",
            "decode": "",
            "actual_video_bitrate_source": "transport-metrics",
            "video_actual_bitrate_kbps": 1019.4,
            "latest_video_bwe_observation": {
                "observation_id": 42,
                "mode": "twcc-gcc",
                "decision_reason": "twcc-gcc-cloud-ramp-up",
                "target_remb_kbps": 28500,
                "observed_remb_kbps": 28500,
                "actual_video_bitrate_kbps": 0.0,
                "loss_ratio": 0.0,
                "rtt_ms": 199.7,
                "transport_path": "Direct",
                "twcc_feedback_interval_ms": 113.0,
                "twcc_observed_packet_count": 12,
                "twcc_covered_sequence_span": 12,
                "twcc_receive_bitrate_kbps": 1019.4,
                "twcc_delivery_ratio": 1.0,
                "twcc_loss_ratio": 0.0,
                "observed_at_ms": 1000.0
            }
        }));

        record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &stats);

        let contents = fs::read_to_string(recorder.path()).expect("trace contents");
        assert!(contents.contains("\"event\":\"bweUpdated\""));
        assert!(contents.contains("\"actualVideoBitrateKbps\":1019.4"));
        assert!(!contents.contains("\"actualVideoBitrateKbps\":0.0"));
    }

    #[test]
    fn frame_recovery_observation_projects_ledger_events() {
        let recorder = std::sync::Arc::new(RuntimeTraceRecorder::new().expect("trace recorder"));
        let mut state = RuntimeTraceObservationState::default();
        let stats = test_stats(json!({
            "resolution": "",
            "rtt": "",
            "fps": 0.0,
            "pl": "0.00%",
            "fl": "",
            "jit": "",
            "br": "",
            "decode": "",
            "latest_video_frame_recovery_observation": {
                "observation_id": 77,
                "action": "ledgerWrite",
                "frame_rtp_timestamp": 123456789,
                "frame_playout_deadline_at_ms": 4567.0,
                "frame_recovery_disposition": "unrecoverable-reference-chain",
                "frame_unrecoverable_reason": "referenceChainUnrecoverable",
                "observed_at_ms": 1234.0
            }
        }));

        record_runtime_trace_observations(&recorder, &mut state, Some("session-1"), &stats);

        let contents = fs::read_to_string(recorder.path()).expect("trace contents");
        assert!(contents.contains("\"event\":\"frameRecoveryObserved\""));
        assert!(contents.contains("\"action\":\"ledgerWrite\""));
        assert!(contents.contains("\"frameRtpTimestamp\":123456789"));
        assert!(contents.contains("\"frameRecoveryDisposition\":\"unrecoverable-reference-chain\""));
        assert!(contents.contains("\"frameUnrecoverableReason\":\"referenceChainUnrecoverable\""));
    }
}
