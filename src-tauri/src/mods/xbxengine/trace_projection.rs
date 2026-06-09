use serde_json::{json, Value};
use xbxengine_protocol::XbxEngineStatsDto;

use crate::mods::runtime_trace::RuntimeTraceRecorderRef;

const DIRECT_GAMING_STATE_SAMPLE_INTERVAL_MS: f64 = 1_000.0;
const MEDIA_DIAGNOSTIC_TRACE_BUCKET_MS: f64 = 2_000.0;
const HOST_PRESENT_STATE_SAMPLE_EPOCH_INTERVAL: u64 = 60;
const VIDEO_TRACK_STATE_SAMPLE_INTERVAL_MS: f64 = 1_000.0;
const DISPLAYED_FRAME_STALE_THRESHOLD_MS: f64 = 300.0;

fn displayed_frame_stale(
    present_age_ms: Option<f64>,
    last_displayed_frame_seq: Option<u64>,
) -> bool {
    last_displayed_frame_seq.is_some()
        && present_age_ms.is_some_and(|age_ms| age_ms >= DISPLAYED_FRAME_STALE_THRESHOLD_MS)
}

fn retained_old_frame_risk(
    present_age_ms: Option<f64>,
    last_displayed_frame_seq: Option<u64>,
    no_pending_streak: Option<u32>,
    host_cadence_phase: Option<&str>,
) -> bool {
    if matches!(host_cadence_phase, Some("steady" | "priming")) {
        return false;
    }
    displayed_frame_stale(present_age_ms, last_displayed_frame_seq)
        && no_pending_streak.unwrap_or(0) > 0
}

/// 当 presentation 进入 `displaySupplyStarved` 时，把上游卡点收成可读 blocker（仅 trace/UI）。
/// 优先级：decode→clean anchor→display stable 闭环缺口，最后才是 generic wait-idr。
fn derive_display_supply_starved_blocker(stats: &XbxEngineStatsDto) -> Option<String> {
    let supply_starved = stats.presentation_health.as_deref() == Some("displaySupplyStarved")
        || stats.video_health.as_deref() == Some("displaySupplyStarved");
    if !supply_starved {
        return None;
    }
    if stats
        .latest_receive_picture_recovery_terminal_reason
        .as_deref()
        == Some("no-clean-anchor-after-decode")
    {
        return Some("decoded-no-clean-anchor".to_string());
    }
    if stats
        .latest_receive_picture_recovery_terminal_reason
        .as_deref()
        == Some("no-display-stable-after-anchor")
    {
        return Some("clean-anchor-no-host-submit".to_string());
    }
    if stats.receive_keyframe_response_state.as_deref() == Some("usable-idr")
        && stats.receive_display_state.as_deref() != Some("display-stable")
    {
        return Some("decoded-no-clean-anchor".to_string());
    }
    let present_fresh = stats.present_age_ms.is_some_and(|age| age < 600.0);
    let submit_stale = stats.submit_age_ms.is_some_and(|age| age >= 200.0);
    if present_fresh && submit_stale {
        return Some("host-retained-old-frame".to_string());
    }
    if matches!(
        stats
            .latest_receive_picture_recovery_terminal_reason
            .as_deref(),
        Some(
            "remote-no-usable-idr"
                | "remote-continuation-only"
                | "remote-no-response"
                | "decoder-rejected-idr"
        )
    ) {
        return Some("waiting-usable-idr".to_string());
    }
    if matches!(
        stats.receive_keyframe_response_state.as_deref(),
        Some("no-packet" | "non-idr-only" | "idr-unusable")
    ) {
        return Some("waiting-usable-idr".to_string());
    }
    if stats.receive_keyframe_required == Some(true) {
        return Some("waiting-usable-idr".to_string());
    }
    if stats.receive_picture_recovery_terminal_candidate == Some(true) {
        return Some("waiting-usable-idr".to_string());
    }
    if let Some(cause) = stats.receive_keyframe_required_cause.as_deref() {
        if cause.contains("decode") || cause.contains("no-output") {
            return Some("decoded-no-clean-anchor".to_string());
        }
    }
    Some("presentation-supply-starved".to_string())
}

/// episode 仅作 effectiveness 投影时的对齐状态（trace 排障）。
fn derive_episode_projection_state(stats: &XbxEngineStatsDto) -> &'static str {
    let Some(episode) = stats.latest_keyframe_request_episode.as_ref() else {
        return "absent";
    };
    let ledger_has_closure = stats.receive_display_state.as_deref() == Some("display-stable")
        || stats
            .latest_receive_picture_recovery_terminal_reason
            .is_some();
    if ledger_has_closure && stats.receive_keyframe_required != Some(true) {
        return "matched";
    }
    if episode.status == "waiting"
        && stats.receive_picture_recovery_terminal_candidate == Some(true)
    {
        return "mismatch";
    }
    if episode.status == "waiting" && !ledger_has_closure {
        return "legacy-only";
    }
    "matched"
}

fn display_stable_ledger_closure_ok(stats: &XbxEngineStatsDto) -> bool {
    stats.receive_keyframe_required != Some(true)
        && stats.receive_keyframe_response_state.as_deref() == Some("usable-idr")
        && stats.receive_display_state.as_deref() == Some("display-stable")
}

fn stale_insert_projection_under_must_idr(stats: &XbxEngineStatsDto) -> bool {
    let decision = stats.latest_insert_decision.as_deref();
    let reason = stats
        .latest_insert_decision_reason
        .as_deref()
        .unwrap_or_default();
    let must_idr_control = stats.receive_keyframe_required == Some(true)
        || stats.reference_chain_state.as_deref() == Some("need-keyframe");

    decision == Some("emit") && must_idr_control && !reason.to_ascii_lowercase().contains("idr")
}

#[derive(Default)]
pub(super) struct RuntimeTraceObservationState {
    packet_gap_observation_id: Option<u64>,
    frame_drop_observation_id: Option<u64>,
    frame_recovery_observation_id: Option<u64>,
    nack_observation_id: Option<u64>,
    escalation_observation_id: Option<u64>,
    recovery_decision_ledger_signature: Option<(
        String,
        String,
        String,
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
    )>,
    bwe_observation_id: Option<u64>,
    twcc_observation_id: Option<u64>,
    rtc_builder_observation_id: Option<u64>,
    twcc_remote_stream_observation_id: Option<u64>,
    remote_answer_observation_id: Option<u64>,
    twcc_extension_observation_id: Option<u64>,
    data_channel_catalog_observation_id: Option<u64>,
    timeline_observation_id: Option<u64>,
    anchor_candidate_observation: Option<(u64, Option<u32>, String, Option<String>, f64)>,
    h264_inspection_trace_signature: Option<(bool, Option<String>, u64)>,
    h264_idr_trace_signature: Option<(Option<u32>, bool, bool)>,
    picture_recovery_transition_observation_id: Option<u64>,
    picture_recovery_blocker_trace_signature: Option<(String, String, String, u64)>,
    video_ingress_termination_observation_id: Option<u64>,
    first_frame_latency_observation_id: Option<u64>,
    decoder_probe_observation_id: Option<u64>,
    decoder_bootstrap_gate_observation_id: Option<u64>,
    decode_output_path_observation_id: Option<u64>,
    remote_frame_capture_observation_id: Option<u64>,
    render_mailbox_decision_id: Option<u64>,
    recovery_keyframe_request_count: Option<u64>,
    recovery_decoder_reset_count: Option<u64>,
    recovery_reconnect_count: Option<u64>,
    keyframe_request_episode:
        Option<xbxengine_protocol::XbxEngineKeyframeRequestEpisodeObservationDto>,
    latest_video_rtcp_send_failure_signature: Option<(String, String)>,
    feedback_target_availability_signature:
        Option<(Option<String>, Option<String>, Option<String>)>,
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
    recovery_rfc_fault_domain: Option<String>,
    recovery_rfc_stage: Option<String>,
    recovery_rfc_ceiling: Option<String>,
    recovery_effective_rtt_ms: Option<String>,
    recovery_timing_signature: Option<(
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    )>,
    recovery_salvage_signature: Option<(Option<bool>, Option<String>)>,
    remote_profile_bitrate_band: Option<String>,
    primary_issue_chain: Option<String>,
    recovery_owner_state: Option<String>,
    recovery_owner_contract_state: Option<String>,
    recovery_owner_reason: Option<String>,
    video_owner_source: Option<String>,
    video_owner_observed_at_ms: Option<f64>,
    unified_lifecycle: Option<String>,
    video_health: Option<String>,
    chain_health: Option<String>,
    presentation_health: Option<String>,
    stall_kind: Option<String>,
    host_mailbox_enqueue_count_total: Option<u64>,
    host_mailbox_drop_count_total: Option<u64>,
    host_mailbox_overwrite_count_total: Option<u64>,
    host_no_pending_take_count_total: Option<u64>,
    host_no_pending_streak: Option<u32>,
    host_no_pending_max_streak: Option<u32>,
    host_no_pending_pressure_level: Option<String>,
    host_mailbox_submit_epoch: Option<u64>,
    host_display_tick_epoch: Option<u64>,
    host_frame_present_epoch: Option<u64>,
    host_cadence_phase: Option<String>,
    latest_host_submit_rtp_timestamp: Option<u32>,
    host_view_generation: Option<u64>,
    latest_host_view_created_at_bucket: Option<u64>,
    host_frame_present_resumed_signature: Option<(u64, u64, Option<u64>)>,
    last_displayed_frame_seq: Option<u64>,
    last_displayed_frame_rtp_timestamp: Option<u32>,
    last_displayed_at_bucket: Option<u64>,
    host_descriptor_upload_mode: Option<String>,
    host_descriptor_metal_import_count_total: Option<u64>,
    host_descriptor_cpu_upload_count_total: Option<u64>,
    actual_video_bitrate_source: Option<String>,
    twcc_observation_state: Option<String>,
    latest_turn_relay_observation_seq: Option<u64>,
    latest_observation_label: Option<String>,
    latest_observation_summary: Option<String>,
    keyframe_request_outcome_seq: u64,
    receive_feedback_decision_seq: u64,
    reference_chain_signature: Option<(String, String)>,
    picture_recovery_terminal_signature: Option<(String, u64)>,
    ingress_idr_not_admitted_total: u64,
    insert_gate_signature: Option<(String, String, Option<bool>, u64)>,
    picture_recovery_authority: Option<String>,
    picture_recovery_delegated_total: u64,
    session_keyframe_in_flight: Option<bool>,
    latest_target_remb_action: Option<String>,
    latest_target_remb_summary: Option<String>,
    timeline_chain_state: Option<String>,
    timeline_chain_reason: Option<String>,
    receiver_state_signature: Option<(String, Option<String>, f64)>,
    displayed_idr_signature: Option<(Option<u32>, Option<f64>)>,
    clean_anchor_committed_signature: Option<(Option<f64>, Option<u32>)>,
    display_stable_signature: Option<(Option<f64>, Option<u32>)>,
    playback_recovered_signature: Option<(Option<f64>, Option<String>)>,
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
/// `statsSnapshot + recoveryState + hostMailboxState`。
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
            "contractState": stats.recovery_owner_contract_state,
            "reason": stats.recovery_owner_reason,
            "source": stats.video_owner_source,
            "observedAtMs": stats.video_owner_observed_at_ms,
        },
        "remoteProfile": {
            "baseline": stats.remote_profile_baseline,
            "dynamic": stats.remote_profile_dynamic,
            "effectiveLabel": stats.remote_profile_effective_label,
            "bitrateBand": stats.direct_gaming_bitrate_band,
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
            "feedbackTargetAvailability": {
                "target": stats.latest_feedback_target_availability_target,
                "state": stats.latest_feedback_target_availability_state,
                "reason": stats.latest_feedback_target_availability_reason,
                "observedAtMs": stats.latest_feedback_target_availability_observed_at_ms,
            },
        },
        "recovery": {
            "lifecycle": unified_lifecycle,
            "streamLifecyclePhase": unified_lifecycle,
            "sessionPhase": stats.session_phase,
            "strategyProfile": stats.recovery_strategy_profile,
            "diagnosis": stats.recovery_diagnosis,
            "rfcFaultDomain": stats.recovery_rfc_fault_domain,
            "rfcStage": stats.recovery_rfc_stage,
            "rfcCeiling": stats.recovery_rfc_ceiling,
            "playbackRecoveredAtMs": stats.recovery_playback_recovered_at_ms,
            "playbackRecoveredPhase": stats.recovery_playback_recovered_phase,
            "freshAnchorRecoveredAtMs": stats.recovery_fresh_anchor_recovered_at_ms,
            "displayedIdrRtp": stats.recovery_displayed_idr_rtp,
            "displayedIdrAtMs": stats.recovery_displayed_idr_at_ms,
            "effectiveRttMs": stats.recovery_effective_rtt_ms,
            "timing": {
                "nackTimeoutMs": stats.recovery_dynamic_nack_timeout_ms,
                "nackRetryIntervalMs": stats.recovery_dynamic_nack_retry_interval_ms,
                "pliRefreshIntervalMs": stats.recovery_dynamic_pli_refresh_interval_ms,
                "firRetryIntervalMs": stats.recovery_dynamic_fir_retry_interval_ms,
                "decodedPendingCommitHoldMs": stats.recovery_dynamic_decoded_pending_commit_hold_ms,
                "continuationPatienceMs": stats.recovery_dynamic_continuation_patience_ms,
                "cleanAnchorCommitPatienceMs": stats.recovery_dynamic_clean_anchor_patience_ms,
                "firstAttemptSurvivalWindowMs": stats.recovery_nack_first_attempt_survival_window_ms,
                "firstAttemptDeadlineAtMs": stats.recovery_nack_first_attempt_deadline_at_ms,
                "firstAttemptStillEconomical": stats.recovery_nack_first_attempt_still_economical,
                "retryAllowedReason": stats.recovery_nack_retry_allowed_reason,
                "retrySuppressedReason": stats.recovery_nack_retry_suppressed_reason,
            },
            "codec": {
                "bootstrapSalvageApplied": stats.recovery_codec_bootstrap_salvage_applied,
                "bootstrapSalvageFailedReason": stats.recovery_codec_bootstrap_salvage_failed_reason,
                "codecBootstrapSalvageApplied": stats.recovery_codec_bootstrap_salvage_applied,
                "codecBootstrapSalvageFailedReason": stats.recovery_codec_bootstrap_salvage_failed_reason,
            },
            "videoHealth": stats.video_health,
            "chainHealth": stats.chain_health,
            "presentationHealth": stats.presentation_health,
            "videoOwnerState": stats.recovery_owner_state,
            "videoOwnerContractState": stats.recovery_owner_contract_state,
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
            "recoverySurface": stats.recovery_surface_phase,
            "mediaSupplyPhase": stats.media_supply_phase,
            "derivedDecoderHealth": stats.derived_decoder_health,
            "submitAgeMs": stats.submit_age_ms,
            "decodeAgeMs": stats.decode_age_ms,
            "decoderState": stats.video_decoder_recovery_state,
            "decoderEvent": stats.video_decoder_recovery_event,
            "decoderDetail": stats.video_decoder_recovery_detail,
            "decoderStatus": stats.video_decoder_recovery_status,
            "decoderStateChangedAtMs": stats.video_decoder_recovery_state_changed_at_ms,
            "decoderProbe": stats.latest_video_decoder_probe_observation,
            "decoderBootstrapGate": stats.latest_video_decoder_bootstrap_gate_observation,
            "decodeOutputPath": stats.latest_decode_output_path_observation,
            "remoteFrameCapture": stats.latest_remote_frame_capture_observation,
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
            "inboundFrameCountTotal": stats.inbound_video_frame_count_total,
            "inboundRtpMarkerCountTotal": stats.inbound_video_rtp_marker_count_total,
            "inboundAccessUnitCountTotal": stats.inbound_video_access_unit_count_total,
            "inboundDecodeGateEmitCountTotal": stats.inbound_video_decode_gate_emit_count_total,
            "inboundDecodeGateContinueCountTotal": stats.inbound_video_decode_gate_continue_count_total,
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
            "displayedAgeMs": stats.present_age_ms,
            "displayedFrameStale": displayed_frame_stale(
                stats.present_age_ms,
                stats.last_displayed_frame_seq,
            ),
            "retainedOldFrameRisk": retained_old_frame_risk(
                stats.present_age_ms,
                stats.last_displayed_frame_seq,
                stats.host_no_pending_streak,
                stats.host_cadence_phase.as_deref(),
            ),
            "lastDisplayedFrameSeq": stats.last_displayed_frame_seq,
            "lastDisplayedFrameRtpTimestamp": stats.last_displayed_frame_rtp_timestamp,
            "lastDisplayedAtMs": stats.last_displayed_at_ms,
            "packetToDecodeMs": stats.packet_to_decode_ms,
            "decodeToPresentMs": stats.decode_to_present_ms,
            "submitToPresentMs": stats.submit_to_present_ms,
            "inspectionPulseActive": stats.inspection_pulse_active,
            "packetToPresentMs": stats.packet_to_present_ms,
            "decoderStalled": stats.video_decoder_stalled,
            "rendererStalled": stats.video_renderer_stalled,
            "decodeInputDropCountTotal": stats.video_decode_input_drop_count_total,
            "decodeOutputDropCountTotal": stats.video_decode_output_drop_count_total,
            "pacerSubmitCountTotal": stats.video_pacer_submit_count_total,
            "pacerDropCountTotal": stats.video_pacer_drop_count_total,
            "rendererSubmitCountTotal": stats.video_renderer_submit_count_total,
            "rendererDropCountTotal": stats.video_renderer_drop_count_total,
            "hostMailboxEnqueueCountTotal": stats.host_mailbox_enqueue_count_total,
            "hostMailboxDropCountTotal": stats.host_mailbox_drop_count_total,
            "hostMailboxOverwriteCountTotal": stats.host_mailbox_overwrite_count_total,
            "noPendingTakeCountTotal": stats.host_no_pending_take_count_total,
            "noPendingStreak": stats.host_no_pending_streak,
            "noPendingMaxStreak": stats.host_no_pending_max_streak,
            "noPendingPressureLevel": stats.host_no_pending_pressure_level,
            "hostDisplayTickEpoch": stats.host_display_tick_epoch,
            "hostFramePresentEpoch": stats.host_frame_present_epoch,
            "hostPresentTakeEmptyStreak": stats.host_present_take_empty_streak,
            "hostMailboxLatestSubmitAtMs": stats.host_mailbox_latest_submit_at_ms,
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
            "rtcpSendFailure": latest_rtcp_send_failure_snapshot(stats),
            "keyframeRequestEpisode": stats.latest_keyframe_request_episode,
            "recentKeyframeRequestEpisodes": stats.recent_keyframe_request_episodes,
            "h264Inspection": h264_inspection_payload(
                stats.latest_h264_inspection_observation.as_ref(),
                stats,
            ),
            "timeline": stats.latest_video_timeline_observation,
            "decodeCandidate": stats.latest_decode_candidate_decision,
            "renderMailbox": stats.latest_render_mailbox_decision,
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
            "observing" => return "observing",
            "local-self-healing" => return "local-self-healing",
            "recovery-eligible" => return "recovery-eligible",
            "active-recovery" => return "active-recovery",
            "recovery-blocked" => return "recovery-blocked",
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
            "observing" => return "observing",
            "local-self-healing" => return "local-self-healing",
            "recovery-eligible" => return "recovery-eligible",
            "active-recovery" => return "active-recovery",
            "recovery-blocked" => return "recovery-blocked",
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
        let value_tier_v2 = derive_dynamic_value_tier_v2(
            budget.chain_value.as_str(),
            budget.recovery_stage.as_str(),
        );
        json!({
            "source": source,
            "observedAtMs": observed_at_ms,
            "recoveryStage": budget.recovery_stage,
            "chainValue": budget.chain_value,
            "valueTierV2": value_tier_v2,
            "rttSlack": budget.rtt_slack,
            "failureCost": budget.failure_cost,
            "windowSource": budget.window_source,
        })
    })
}

fn derive_dynamic_value_tier_v2(chain_value: &str, recovery_stage: &str) -> &'static str {
    match (chain_value, recovery_stage) {
        ("anchor", _) => "anchor",
        ("supply", "awaiting-keyframe" | "repairing") => "continuation",
        ("supply", _) => "supply",
        _ => "disposable",
    }
}

pub(super) fn record_runtime_trace_observations(
    runtime_trace: &RuntimeTraceRecorderRef,
    observation_state: &mut RuntimeTraceObservationState,
    session_id: Option<&str>,
    stats: &XbxEngineStatsDto,
) {
    let trace_mode = runtime_trace.trace_mode();
    let latest_rtcp_send_failure = latest_rtcp_send_failure_from_stats(stats);
    let latest_rtcp_send_failure_signature = latest_rtcp_send_failure
        .as_ref()
        .map(|(observed_at_ms, reason)| (format!("{observed_at_ms:.1}"), reason.clone()));
    if observation_state.latest_video_rtcp_send_failure_signature
        != latest_rtcp_send_failure_signature
    {
        observation_state.latest_video_rtcp_send_failure_signature =
            latest_rtcp_send_failure_signature;
        if let Some((observed_at_ms, reason)) = latest_rtcp_send_failure.as_ref() {
            runtime_trace.record_event(
                "xbxengine",
                "videoRtcpSendFailureObserved",
                session_id,
                json!({
                    "observedAtMs": observed_at_ms,
                    "reason": reason,
                    "source": "latestObservation",
                }),
            );
        }
    }

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
                    "replacementDecision": frame_drop.replacement_decision,
                    "observedAtMs": frame_drop.observed_at_ms,
                    "width": frame_drop.width,
                    "height": frame_drop.height,
                    "isKeyframe": frame_drop.is_keyframe,
                    "queueDepth": frame_drop.queue_depth,
                }),
            );
            let decision_event_name = match frame_drop.stage.as_deref() {
                Some("pacer") => Some("pacerCandidateDecision"),
                Some("render") => Some("renderMailboxDecision"),
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
    let has_decoder_recovery_signal = decoder_recovery_state.0.is_some()
        || decoder_recovery_state.1.is_some()
        || decoder_recovery_state.2.is_some()
        || decoder_recovery_state.3.is_some()
        || decoder_recovery_state.4.is_some();
    if has_decoder_recovery_signal
        && observation_state.decoder_recovery_state.as_ref() != Some(&decoder_recovery_state)
    {
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

    if let Some(decoder_probe) = stats.latest_video_decoder_probe_observation.as_ref() {
        if observation_state.decoder_probe_observation_id != Some(decoder_probe.observation_id) {
            observation_state.decoder_probe_observation_id = Some(decoder_probe.observation_id);
            runtime_trace.record_event(
                "xbxengine",
                "decoderBackendProbeObserved",
                session_id,
                json!({
                    "observationId": decoder_probe.observation_id,
                    "selectedBackendName": decoder_probe.selected_backend_name,
                    "selectedBackendKind": decoder_probe.selected_backend_kind,
                    "fallbackCount": decoder_probe.fallback_count,
                    "fallbackSummary": decoder_probe.fallback_summary,
                    "observedAtMs": decoder_probe.observed_at_ms,
                }),
            );
        }
    }

    if let Some(observation) = stats
        .latest_video_decoder_bootstrap_gate_observation
        .as_ref()
    {
        if observation_state.decoder_bootstrap_gate_observation_id
            != Some(observation.observation_id)
        {
            observation_state.decoder_bootstrap_gate_observation_id =
                Some(observation.observation_id);
            runtime_trace.record_event(
                "xbxengine",
                "decoderBootstrapGateRejected",
                session_id,
                json!({
                    "observationId": observation.observation_id,
                    "recoveryState": observation.recovery_state,
                    "frameRtpTimestamp": observation.frame_rtp_timestamp,
                    "isIdr": observation.is_idr,
                    "hasInbandSps": observation.has_inband_sps,
                    "hasInbandPps": observation.has_inband_pps,
                    "committedSpsPresent": observation.committed_sps_present,
                    "committedPpsPresent": observation.committed_pps_present,
                    "bootstrapReady": observation.bootstrap_ready,
                    "bootstrapRejectReason": observation.bootstrap_reject_reason,
                    "observedAtMs": observation.observed_at_ms,
                }),
            );
        }
    }

    if let Some(observation) = stats.latest_decode_output_path_observation.as_ref() {
        let material_in_minimal = trace_mode != "minimal" || observation.verdict != "decoded-frame";
        if material_in_minimal
            && observation_state.decode_output_path_observation_id
                != Some(observation.observation_id)
        {
            observation_state.decode_output_path_observation_id = Some(observation.observation_id);
            runtime_trace.record_event(
                "xbxengine",
                "decodeOutputPathObserved",
                session_id,
                json!({
                    "observationId": observation.observation_id,
                    "verdict": observation.verdict,
                    "detail": observation.detail,
                    "frameRtpTimestamp": observation.frame_rtp_timestamp,
                    "isKeyframe": observation.is_keyframe,
                    "status": observation.status,
                    "sendPacketStatus": observation.send_packet_status,
                    "receiveFrameStatus": observation.receive_frame_status,
                    "backendNoOutputStreak": observation.backend_no_output_streak,
                    "inputFramesSinceLastDecoded": observation.input_frames_since_last_decoded,
                    "bootstrapRejectReason": observation.bootstrap_reject_reason,
                    "observedAtMs": observation.observed_at_ms,
                }),
            );
        }
    }

    if let Some(observation) = stats.latest_remote_frame_capture_observation.as_ref() {
        if observation_state.remote_frame_capture_observation_id != Some(observation.observation_id)
        {
            observation_state.remote_frame_capture_observation_id =
                Some(observation.observation_id);
            runtime_trace.record_event(
                "xbxengine",
                "remoteFrameCaptured",
                session_id,
                json!({
                    "observationId": observation.observation_id,
                    "trigger": observation.trigger,
                    "backendName": observation.backend_name,
                    "frameRtpTimestamp": observation.frame_rtp_timestamp,
                    "isKeyframe": observation.is_keyframe,
                    "width": observation.width,
                    "height": observation.height,
                    "payloadBytes": observation.payload_bytes,
                    "payloadFingerprint": observation.payload_fingerprint,
                    "payloadPrefixHex": observation.payload_prefix_hex,
                    "nalTypes": observation.nal_types,
                    "nalCount": observation.nal_count,
                    "hasInbandSps": observation.has_inband_sps,
                    "hasInbandPps": observation.has_inband_pps,
                    "bootstrapReady": observation.bootstrap_ready,
                    "bootstrapRejectReason": observation.bootstrap_reject_reason,
                    "parameterSetsChanged": observation.parameter_sets_changed,
                    "configChanged": observation.config_changed,
                    "sliceHeadersValid": observation.slice_headers_valid,
                    "status": observation.status,
                    "sendPacketStatus": observation.send_packet_status,
                    "receiveFrameStatus": observation.receive_frame_status,
                    "backendNoOutputStreak": observation.backend_no_output_streak,
                    "inputFramesSinceLastDecoded": observation.input_frames_since_last_decoded,
                    "observedAtMs": observation.observed_at_ms,
                }),
            );
        }
    }

    if let Some(render_mailbox) = stats.latest_render_mailbox_decision.as_ref() {
        if observation_state.render_mailbox_decision_id != Some(render_mailbox.decision_id) {
            observation_state.render_mailbox_decision_id = Some(render_mailbox.decision_id);
            runtime_trace.record_event(
                "xbxengine",
                "renderMailboxStateTransition",
                session_id,
                json!({
                    "decisionId": render_mailbox.decision_id,
                    "state": render_mailbox.state,
                    "action": render_mailbox.action,
                    "detail": render_mailbox.detail,
                    "frameSeq": render_mailbox.frame_seq,
                    "replacementDecision": render_mailbox.replacement_decision,
                    "observedAtMs": render_mailbox.observed_at_ms,
                }),
            );
        }
    }

    if let Some(nack) = stats.latest_video_nack_observation.as_ref() {
        if observation_state.nack_observation_id != Some(nack.observation_id) {
            observation_state.nack_observation_id = Some(nack.observation_id);
            let event_name = match nack.action.as_str() {
                "expiredDeadline"
                | "expiredMaxAge"
                | "expiredRetryBudget"
                | "expiredSingleShotPollComplete"
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
            if let Some(event_name) = clean_anchor_funnel_event_name(&timeline.source_event) {
                runtime_trace.record_event(
                    "xbxengine",
                    event_name,
                    session_id,
                    clean_anchor_funnel_payload(stats, timeline),
                );
            }
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
            ledger.state_before.clone(),
            ledger.state_after.clone(),
            ledger.input_signal.clone(),
            ledger.gate_result.clone(),
            ledger.action_selected.clone(),
            ledger.command_result.clone(),
            ledger.command_detail.clone(),
            ledger.gap_severity.clone(),
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
                    "frameValue": ledger.frame_value,
                    "gapSeverity": ledger.gap_severity,
                    "repairability": ledger.repairability,
                    "triggerObservationLabel": ledger.trigger_observation_label,
                    "triggerObservationSummary": ledger.trigger_observation_summary,
                    "recoveryEpisodeStage": ledger.recovery_episode_stage,
                    "recoveryEpisodeProgressAtMs": ledger.recovery_episode_progress_at_ms,
                    "coalescingMode": ledger.coalescing_mode,
                    "unlockReason": ledger.unlock_reason,
                    "preemptReason": ledger.preempt_reason,
                    "recoveryPrimaryAction": ledger.recovery_primary_action,
                    "ownerSurfaceState": ledger.owner_surface_state,
                    "anchorEvidence": ledger.anchor_evidence,
                    "keyframeEpisodeHealth": ledger.keyframe_episode_health,
                    "escalationBasis": ledger.escalation_basis,
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
        let payload = keyframe_request_episode_payload(
            stats.latest_keyframe_request_episode.as_ref(),
            stats.latest_h264_inspection_observation.as_ref(),
            latest_rtcp_send_failure.as_ref(),
            stats,
        );
        runtime_trace.record_state(
            "xbxengine",
            "keyframeRequestEpisode",
            session_id,
            payload.clone(),
        );
        if let Some(episode) = stats.latest_keyframe_request_episode.as_ref() {
            if let Some(event_name) = keyframe_request_episode_event_name(&episode.status) {
                runtime_trace.record_event("xbxengine", event_name, session_id, payload.clone());
            }
            if let Some(suppression_reason) = diagnostic_keyframe_suppression_reason(episode) {
                let mut suppression_payload = payload.clone();
                if let Some(object) = suppression_payload.as_object_mut() {
                    object.insert(
                        "diagnosticSuppressionReason".to_string(),
                        json!(suppression_reason),
                    );
                }
                runtime_trace.record_event(
                    "xbxengine",
                    "keyframeRequestSuppressedObserved",
                    session_id,
                    suppression_payload,
                );
            }
        }
    }

    if let Some(receiver) = stats.latest_video_receiver_observation.as_ref() {
        let signature = (
            receiver.receiver_state.clone(),
            receiver.bootstrap_reject_reason.clone(),
            receiver.observed_at_ms,
        );
        if observation_state.receiver_state_signature.as_ref() != Some(&signature) {
            observation_state.receiver_state_signature = Some(signature);
            runtime_trace.record_event(
                "xbxengine",
                "receiverStateChanged",
                session_id,
                json!({
                    "receiverState": receiver.receiver_state,
                    "bootstrapRejectReason": receiver.bootstrap_reject_reason,
                    "keyframeRequestPending": receiver.keyframe_request_pending,
                    "observedAtMs": receiver.observed_at_ms,
                }),
            );
        }
    }

    let displayed_idr_signature = (
        stats.recovery_displayed_idr_rtp,
        stats.recovery_displayed_idr_at_ms,
    );
    if observation_state.displayed_idr_signature.as_ref() != Some(&displayed_idr_signature)
        && stats.recovery_displayed_idr_at_ms.is_some()
    {
        observation_state.displayed_idr_signature = Some(displayed_idr_signature);
        runtime_trace.record_event(
            "xbxengine",
            "displayedIdrObserved",
            session_id,
            json!({
                "rtpTimestamp": stats.recovery_displayed_idr_rtp,
                "observedAtMs": stats.recovery_displayed_idr_at_ms,
                "freshAnchorRecoveredAtMs": stats.recovery_fresh_anchor_recovered_at_ms,
            }),
        );
    }

    let clean_anchor_committed_signature = (
        stats.recovery_fresh_anchor_recovered_at_ms,
        stats
            .recovery_displayed_idr_rtp
            .or(stats.latest_video_decode_ok_rtp_timestamp),
    );
    if observation_state.clean_anchor_committed_signature.as_ref()
        != Some(&clean_anchor_committed_signature)
        && stats.recovery_fresh_anchor_recovered_at_ms.is_some()
    {
        observation_state.clean_anchor_committed_signature = Some(clean_anchor_committed_signature);
        runtime_trace.record_event(
            "xbxengine",
            "pictureRecoveryTransition",
            session_id,
            json!({
                "episodeId": stats
                    .latest_keyframe_request_episode
                    .as_ref()
                    .map(|episode| episode.episode_id),
                "phase": "CleanAnchorCommitted",
                "fromPhase": "Decoded",
                "toPhase": "CleanAnchorCommitted",
                "cause": "fresh-anchor-recovered",
                "detail": "mediaRecovered",
                "rtpTimestamp": stats
                    .recovery_displayed_idr_rtp
                    .or(stats.latest_video_decode_ok_rtp_timestamp),
                "frameSeq": stats.last_displayed_frame_seq,
                "ownerState": stats.recovery_owner_state,
                "transportState": stats.transport_state,
                "observedAtMs": stats.recovery_fresh_anchor_recovered_at_ms,
            }),
        );
        runtime_trace.record_event(
            "xbxengine",
            "cleanAnchorCommitted",
            session_id,
            json!({
                "rtpTimestamp": stats
                    .recovery_displayed_idr_rtp
                    .or(stats.latest_video_decode_ok_rtp_timestamp),
                "observedAtMs": stats.recovery_fresh_anchor_recovered_at_ms,
            }),
        );
    }

    let display_stable_signature = (
        stats
            .recovery_displayed_idr_at_ms
            .or(stats.last_displayed_at_ms),
        stats
            .recovery_displayed_idr_rtp
            .or(stats.last_displayed_frame_rtp_timestamp),
    );
    if observation_state.display_stable_signature.as_ref() != Some(&display_stable_signature)
        && stats.receive_display_state.as_deref() == Some("display-stable")
        && display_stable_ledger_closure_ok(stats)
    {
        observation_state.display_stable_signature = Some(display_stable_signature);
        runtime_trace.record_event(
            "xbxengine",
            "pictureRecoveryTransition",
            session_id,
            json!({
                "episodeId": stats
                    .latest_keyframe_request_episode
                    .as_ref()
                    .map(|episode| episode.episode_id),
                "phase": "DisplayStable",
                "fromPhase": "CleanAnchorCommitted",
                "toPhase": "DisplayStable",
                "cause": "stableServingSettled",
                "detail": "displayGate",
                "rtpTimestamp": stats
                    .recovery_displayed_idr_rtp
                    .or(stats.last_displayed_frame_rtp_timestamp),
                "frameSeq": stats.last_displayed_frame_seq,
                "ownerState": stats.recovery_owner_state,
                "transportState": stats.transport_state,
                "observedAtMs": stats.recovery_displayed_idr_at_ms.or(stats.last_displayed_at_ms),
                "keyframeRequired": stats.receive_keyframe_required,
                "responseState": stats.receive_keyframe_response_state,
                "receiveDisplayState": stats.receive_display_state,
                "ledgerGeneration": stats.receive_recovery_ledger_generation,
            }),
        );
        runtime_trace.record_event(
            "xbxengine",
            "stableServingSettled",
            session_id,
            json!({
                "rtpTimestamp": stats
                    .recovery_displayed_idr_rtp
                    .or(stats.last_displayed_frame_rtp_timestamp),
                "observedAtMs": stats.recovery_displayed_idr_at_ms.or(stats.last_displayed_at_ms),
                "keyframeRequired": stats.receive_keyframe_required,
                "responseState": stats.receive_keyframe_response_state,
                "receiveDisplayState": stats.receive_display_state,
                "ledgerGeneration": stats.receive_recovery_ledger_generation,
            }),
        );
    }

    let playback_recovered_signature = (
        stats.recovery_playback_recovered_at_ms,
        stats.recovery_playback_recovered_phase.clone(),
    );
    if observation_state.playback_recovered_signature.as_ref()
        != Some(&playback_recovered_signature)
        && stats.recovery_playback_recovered_at_ms.is_some()
    {
        observation_state.playback_recovered_signature = Some(playback_recovered_signature);
        runtime_trace.record_event(
            "xbxengine",
            "playbackRecovered",
            session_id,
            json!({
                "observedAtMs": stats.recovery_playback_recovered_at_ms,
                "phase": stats.recovery_playback_recovered_phase,
                "presentFps": stats.present_fps,
            }),
        );
    }

    if let Some(inspection) = stats.latest_h264_inspection_observation.as_ref() {
        if inspection.is_idr && inspection.admission_accepted {
            let idr_signature = (
                inspection.frame_rtp_timestamp,
                inspection.admission_accepted,
                inspection.bootstrap_ready,
            );
            if observation_state.h264_idr_trace_signature.as_ref() != Some(&idr_signature) {
                observation_state.h264_idr_trace_signature = Some(idr_signature);
                runtime_trace.record_event(
                    "xbxengine",
                    "h264IdrAccessUnitObserved",
                    session_id,
                    h264_inspection_payload(Some(inspection), stats),
                );
            }
        }
    }

    let ingress_idr_not_admitted = stats.ingress_idr_not_admitted_total.unwrap_or(0);
    if ingress_idr_not_admitted != observation_state.ingress_idr_not_admitted_total {
        observation_state.ingress_idr_not_admitted_total = ingress_idr_not_admitted;
        runtime_trace.record_event(
            "xbxengine",
            "h264IdrIngressNotAdmitted",
            session_id,
            json!({
                "total": ingress_idr_not_admitted,
                "insertReason": stats.latest_ingress_idr_not_admitted_reason,
                "ingressWaitingIdrInspectionTotal": stats.ingress_waiting_idr_inspection_total,
                "ingressWaitingRtpMarkerTotal": stats.ingress_waiting_rtp_marker_total,
                "mediaSupplyPhase": stats.media_supply_phase,
            }),
        );
    }

    if let Some(inspection) = stats.latest_h264_inspection_observation.as_ref() {
        let signature = (
            inspection.admission_accepted,
            inspection.reject_classification.clone(),
            sample_bucket_ms(
                Some(inspection.observed_at_ms),
                MEDIA_DIAGNOSTIC_TRACE_BUCKET_MS,
            )
            .unwrap_or(0),
        );
        if observation_state.h264_inspection_trace_signature.as_ref() != Some(&signature) {
            observation_state.h264_inspection_trace_signature = Some(signature);
            let material = inspection.bootstrap_reject_reason.is_some()
                || !inspection.admission_accepted
                || trace_mode != "minimal";
            if material {
                let payload = h264_inspection_payload(Some(inspection), stats);
                runtime_trace.record_state(
                    "xbxengine",
                    "h264Inspection",
                    session_id,
                    payload.clone(),
                );
                let event_name = if inspection.admission_accepted {
                    "h264InspectionObserved"
                } else {
                    "h264InspectionRejected"
                };
                runtime_trace.record_event("xbxengine", event_name, session_id, payload.clone());
                if inspection.bootstrap_reject_reason.is_some() && !inspection.admission_accepted {
                    runtime_trace.record_event(
                        "xbxengine",
                        "bootstrapRejectObserved",
                        session_id,
                        payload,
                    );
                }
            }
        }
    }

    if observation_state.picture_recovery_transition_observation_id
        != stats
            .latest_picture_recovery_transition_observation
            .as_ref()
            .map(|observation| observation.observation_id)
    {
        observation_state.picture_recovery_transition_observation_id = stats
            .latest_picture_recovery_transition_observation
            .as_ref()
            .map(|observation| observation.observation_id);
        if let Some(observation) = stats
            .latest_picture_recovery_transition_observation
            .as_ref()
        {
            let display_stable_without_ledger_closure =
                observation.to_phase == "DisplayStable" && !display_stable_ledger_closure_ok(stats);
            if !display_stable_without_ledger_closure {
                let mut payload = picture_recovery_transition_payload(observation);
                enrich_display_stable_transition_payload(&mut payload, stats);
                runtime_trace.record_event(
                    "xbxengine",
                    "pictureRecoveryTransition",
                    session_id,
                    payload.clone(),
                );
                if observation.phase == "FreshAnchorRecovered" {
                    runtime_trace.record_event(
                        "xbxengine",
                        "freshAnchorRecovered",
                        session_id,
                        payload,
                    );
                }
            }
        }
    }

    if let Some(observation) = stats.latest_picture_recovery_blocker_observation.as_ref() {
        let signature = (
            observation.gate.clone(),
            observation.blocker_kind.clone(),
            observation.severity.clone(),
            sample_bucket_ms(
                Some(observation.observed_at_ms),
                MEDIA_DIAGNOSTIC_TRACE_BUCKET_MS,
            )
            .unwrap_or(0),
        );
        if observation_state
            .picture_recovery_blocker_trace_signature
            .as_ref()
            != Some(&signature)
        {
            observation_state.picture_recovery_blocker_trace_signature = Some(signature);
            runtime_trace.record_event(
                "xbxengine",
                "pictureRecoveryBlockerObserved",
                session_id,
                picture_recovery_blocker_payload(observation),
            );
        }
    }

    if observation_state.video_ingress_termination_observation_id
        != stats
            .latest_video_ingress_termination_observation
            .as_ref()
            .map(|observation| observation.observation_id)
    {
        observation_state.video_ingress_termination_observation_id = stats
            .latest_video_ingress_termination_observation
            .as_ref()
            .map(|observation| observation.observation_id);
        if let Some(observation) = stats.latest_video_ingress_termination_observation.as_ref() {
            runtime_trace.record_event(
                "xbxengine",
                "videoIngressTermination",
                session_id,
                video_ingress_termination_payload(observation),
            );
        }
    }

    if observation_state.first_frame_latency_observation_id
        != stats
            .latest_first_frame_latency_observation
            .as_ref()
            .map(|observation| observation.observation_id)
    {
        observation_state.first_frame_latency_observation_id = stats
            .latest_first_frame_latency_observation
            .as_ref()
            .map(|observation| observation.observation_id);
        if let Some(observation) = stats.latest_first_frame_latency_observation.as_ref() {
            runtime_trace.record_event(
                "xbxengine",
                "firstFrameLatencyObserved",
                session_id,
                first_frame_latency_payload(observation),
            );
        }
    }

    let recovery_effective_rtt_ms = stats
        .recovery_effective_rtt_ms
        .map(|value| format!("{value:.1}"));
    let recovery_timing_signature = (
        stats
            .recovery_dynamic_nack_timeout_ms
            .map(|value| format!("{value:.1}")),
        stats
            .recovery_dynamic_nack_retry_interval_ms
            .map(|value| format!("{value:.1}")),
        stats
            .recovery_dynamic_pli_refresh_interval_ms
            .map(|value| format!("{value:.1}")),
        stats
            .recovery_dynamic_fir_retry_interval_ms
            .map(|value| format!("{value:.1}")),
        stats
            .recovery_dynamic_decoded_pending_commit_hold_ms
            .map(|value| format!("{value:.1}")),
        stats
            .recovery_dynamic_continuation_patience_ms
            .map(|value| format!("{value:.1}")),
        stats
            .recovery_dynamic_clean_anchor_patience_ms
            .map(|value| format!("{value:.1}")),
    );
    let recovery_salvage_signature = (
        stats.recovery_codec_bootstrap_salvage_applied,
        stats.recovery_codec_bootstrap_salvage_failed_reason.clone(),
    );
    if observation_state.session_phase != stats.session_phase
        || observation_state.remote_profile_baseline != stats.remote_profile_baseline
        || observation_state.remote_profile_dynamic != stats.remote_profile_dynamic
        || observation_state.remote_profile_effective_label != stats.remote_profile_effective_label
        || observation_state.transport_strategy_profile != stats.transport_strategy_profile
        || observation_state.recovery_strategy_profile != stats.recovery_strategy_profile
        || observation_state.recovery_diagnosis != stats.recovery_diagnosis
        || observation_state.recovery_rfc_fault_domain != stats.recovery_rfc_fault_domain
        || observation_state.recovery_rfc_stage != stats.recovery_rfc_stage
        || observation_state.recovery_rfc_ceiling != stats.recovery_rfc_ceiling
        || observation_state.recovery_effective_rtt_ms != recovery_effective_rtt_ms
        || observation_state.recovery_timing_signature.as_ref() != Some(&recovery_timing_signature)
        || observation_state.recovery_salvage_signature.as_ref()
            != Some(&recovery_salvage_signature)
        || observation_state.remote_profile_bitrate_band != stats.direct_gaming_bitrate_band
        || observation_state.primary_issue_chain != stats.primary_issue_chain
        || observation_state.recovery_owner_state != stats.recovery_owner_state
        || observation_state.recovery_owner_contract_state != stats.recovery_owner_contract_state
        || observation_state.recovery_owner_reason != stats.recovery_owner_reason
        || observation_state.video_owner_source != stats.video_owner_source
        || observation_state.unified_lifecycle.as_deref() != Some(resolve_unified_lifecycle(stats))
        || observation_state.video_health != stats.video_health
        || observation_state.chain_health != stats.chain_health
        || observation_state.presentation_health != stats.presentation_health
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
        observation_state.recovery_rfc_fault_domain = stats.recovery_rfc_fault_domain.clone();
        observation_state.recovery_rfc_stage = stats.recovery_rfc_stage.clone();
        observation_state.recovery_rfc_ceiling = stats.recovery_rfc_ceiling.clone();
        observation_state.recovery_effective_rtt_ms = recovery_effective_rtt_ms;
        observation_state.recovery_timing_signature = Some(recovery_timing_signature);
        observation_state.recovery_salvage_signature = Some(recovery_salvage_signature);
        observation_state.remote_profile_bitrate_band = stats.direct_gaming_bitrate_band.clone();
        observation_state.primary_issue_chain = stats.primary_issue_chain.clone();
        observation_state.recovery_owner_state = stats.recovery_owner_state.clone();
        observation_state.recovery_owner_contract_state =
            stats.recovery_owner_contract_state.clone();
        observation_state.recovery_owner_reason = stats.recovery_owner_reason.clone();
        observation_state.video_owner_source = stats.video_owner_source.clone();
        observation_state.video_owner_observed_at_ms = stats.video_owner_observed_at_ms;
        observation_state.unified_lifecycle = Some(resolve_unified_lifecycle(stats).to_string());
        observation_state.video_health = stats.video_health.clone();
        observation_state.chain_health = stats.chain_health.clone();
        observation_state.presentation_health = stats.presentation_health.clone();
        observation_state.stall_kind = stats.stall_kind.clone();
        runtime_trace.record_state(
            "xbxengine",
            "recoveryState",
            session_id,
            json!({
                "lifecycle": resolve_unified_lifecycle(stats),
                "streamLifecyclePhase": resolve_unified_lifecycle(stats),
                "sessionPhase": stats.session_phase,
                "remoteProfileBaseline": stats.remote_profile_baseline,
                "remoteProfileDynamic": stats.remote_profile_dynamic,
                "remoteProfileEffectiveLabel": stats.remote_profile_effective_label,
                "remoteProfileBitrateBand": stats.direct_gaming_bitrate_band,
                "transportStrategyProfile": stats.transport_strategy_profile,
                "recoveryStrategyProfile": stats.recovery_strategy_profile,
                "diagnosis": stats.recovery_diagnosis,
                "rfcFaultDomain": stats.recovery_rfc_fault_domain,
                "rfcStage": stats.recovery_rfc_stage,
                "rfcCeiling": stats.recovery_rfc_ceiling,
                "effectiveRttMs": stats.recovery_effective_rtt_ms,
                "timing": {
                    "nackTimeoutMs": stats.recovery_dynamic_nack_timeout_ms,
                    "nackRetryIntervalMs": stats.recovery_dynamic_nack_retry_interval_ms,
                    "pliRefreshIntervalMs": stats.recovery_dynamic_pli_refresh_interval_ms,
                    "firRetryIntervalMs": stats.recovery_dynamic_fir_retry_interval_ms,
                    "decodedPendingCommitHoldMs": stats.recovery_dynamic_decoded_pending_commit_hold_ms,
                    "continuationPatienceMs": stats.recovery_dynamic_continuation_patience_ms,
                    "cleanAnchorCommitPatienceMs": stats.recovery_dynamic_clean_anchor_patience_ms,
                    "firstAttemptSurvivalWindowMs": stats.recovery_nack_first_attempt_survival_window_ms,
                    "firstAttemptDeadlineAtMs": stats.recovery_nack_first_attempt_deadline_at_ms,
                    "firstAttemptStillEconomical": stats.recovery_nack_first_attempt_still_economical,
                    "retryAllowedReason": stats.recovery_nack_retry_allowed_reason,
                    "retrySuppressedReason": stats.recovery_nack_retry_suppressed_reason,
                },
                "codec": {
                    "bootstrapSalvageApplied": stats.recovery_codec_bootstrap_salvage_applied,
                    "bootstrapSalvageFailedReason": stats.recovery_codec_bootstrap_salvage_failed_reason,
                    "codecBootstrapSalvageApplied": stats.recovery_codec_bootstrap_salvage_applied,
                    "codecBootstrapSalvageFailedReason": stats.recovery_codec_bootstrap_salvage_failed_reason,
                },
                "runtimeSummary": stats.runtime_summary,
                "primaryIssueChain": stats.primary_issue_chain,
                "latestDecisionSummary": stats.latest_decision_summary,
                "videoOwnerState": stats.recovery_owner_state,
                "videoOwnerContractState": stats.recovery_owner_contract_state,
                "videoOwnerReason": stats.recovery_owner_reason,
                "videoOwnerSource": stats.video_owner_source,
                "videoOwnerObservedAtMs": stats.video_owner_observed_at_ms,
                "videoHealth": stats.video_health,
                "chainHealth": stats.chain_health,
                "presentationHealth": stats.presentation_health,
                "stallKind": stats.stall_kind,
            }),
        );
    }

    let current_last_displayed_at_bucket = sample_bucket_ms(
        stats.last_displayed_at_ms,
        DIRECT_GAMING_STATE_SAMPLE_INTERVAL_MS,
    );
    let current_host_view_created_at_bucket = sample_bucket_ms(
        stats.latest_host_view_created_at_ms,
        DIRECT_GAMING_STATE_SAMPLE_INTERVAL_MS,
    );
    let host_presentation_semantic_changed = observation_state.host_no_pending_pressure_level
        != stats.host_no_pending_pressure_level
        || observation_state.host_cadence_phase != stats.host_cadence_phase
        || observation_state.host_mailbox_submit_epoch != stats.host_mailbox_submit_epoch
        || observation_state.latest_host_submit_rtp_timestamp
            != stats.latest_video_host_submit_rtp_timestamp
        || observation_state.host_view_generation != stats.host_view_generation
        || observation_state.latest_host_view_created_at_bucket
            != current_host_view_created_at_bucket
        || observation_state.last_displayed_frame_seq != stats.last_displayed_frame_seq
        || observation_state.last_displayed_frame_rtp_timestamp
            != stats.last_displayed_frame_rtp_timestamp
        || observation_state.last_displayed_at_bucket != current_last_displayed_at_bucket
        || observation_state.host_descriptor_upload_mode
            != stats.video_present_descriptor_upload_mode;
    let host_mailbox_counter_regressed = observation_state
        .host_mailbox_enqueue_count_total
        .zip(stats.host_mailbox_enqueue_count_total)
        .is_some_and(|(previous, current)| current < previous)
        || observation_state
            .host_mailbox_drop_count_total
            .zip(stats.host_mailbox_drop_count_total)
            .is_some_and(|(previous, current)| current < previous)
        || observation_state
            .host_mailbox_overwrite_count_total
            .zip(stats.host_mailbox_overwrite_count_total)
            .is_some_and(|(previous, current)| current < previous)
        || observation_state
            .host_no_pending_take_count_total
            .zip(stats.host_no_pending_take_count_total)
            .is_some_and(|(previous, current)| current < previous);
    let host_presentation_sample_due = observation_state
        .host_display_tick_epoch
        .zip(stats.host_display_tick_epoch)
        .is_none_or(|(previous, current)| {
            current.saturating_sub(previous) >= HOST_PRESENT_STATE_SAMPLE_EPOCH_INTERVAL
        });
    if host_presentation_semantic_changed
        || host_mailbox_counter_regressed
        || host_presentation_sample_due
        || observation_state.host_display_tick_epoch.is_none()
        || stats.host_display_tick_epoch.is_none()
    {
        let previous_display_tick_epoch = observation_state.host_display_tick_epoch.unwrap_or(0);
        let previous_present_epoch = observation_state.host_frame_present_epoch.unwrap_or(0);
        let previous_last_displayed_frame_seq = observation_state.last_displayed_frame_seq;
        if stats.host_frame_present_epoch.unwrap_or(0) > 0
            && previous_present_epoch == 0
            && stats.host_display_tick_epoch.unwrap_or(0) >= previous_display_tick_epoch
        {
            let resume_signature = (
                stats.host_display_tick_epoch.unwrap_or(0),
                stats.host_frame_present_epoch.unwrap_or(0),
                stats.last_displayed_frame_seq,
            );
            if observation_state.host_frame_present_resumed_signature
                != Some(resume_signature.clone())
            {
                observation_state.host_frame_present_resumed_signature = Some(resume_signature);
                runtime_trace.record_event(
                    "xbxengine",
                    "hostFramePresentResumed",
                    session_id,
                    json!({
                        "hostDisplayTickEpoch": stats.host_display_tick_epoch,
                        "hostFramePresentEpoch": stats.host_frame_present_epoch,
                        "cadencePhase": stats.host_cadence_phase,
                        "lastDisplayedFrameSeq": stats.last_displayed_frame_seq,
                        "lastDisplayedFrameRtpTimestamp": stats.last_displayed_frame_rtp_timestamp,
                        "lastDisplayedAtMs": stats.last_displayed_at_ms,
                        "previousHostDisplayTickEpoch": previous_display_tick_epoch,
                        "previousHostFramePresentEpoch": previous_present_epoch,
                        "previousLastDisplayedFrameSeq": previous_last_displayed_frame_seq,
                    }),
                );
            }
        }
        observation_state.host_mailbox_enqueue_count_total = stats.host_mailbox_enqueue_count_total;
        observation_state.host_mailbox_drop_count_total = stats.host_mailbox_drop_count_total;
        observation_state.host_mailbox_overwrite_count_total =
            stats.host_mailbox_overwrite_count_total;
        observation_state.host_no_pending_take_count_total = stats.host_no_pending_take_count_total;
        observation_state.host_no_pending_streak = stats.host_no_pending_streak;
        observation_state.host_no_pending_max_streak = stats.host_no_pending_max_streak;
        observation_state.host_no_pending_pressure_level =
            stats.host_no_pending_pressure_level.clone();
        observation_state.host_mailbox_submit_epoch = stats.host_mailbox_submit_epoch;
        observation_state.host_display_tick_epoch = stats.host_display_tick_epoch;
        observation_state.host_frame_present_epoch = stats.host_frame_present_epoch;
        observation_state.host_cadence_phase = stats.host_cadence_phase.clone();
        observation_state.latest_host_submit_rtp_timestamp =
            stats.latest_video_host_submit_rtp_timestamp;
        observation_state.host_view_generation = stats.host_view_generation;
        observation_state.latest_host_view_created_at_bucket = current_host_view_created_at_bucket;
        observation_state.last_displayed_frame_seq = stats.last_displayed_frame_seq;
        observation_state.last_displayed_frame_rtp_timestamp =
            stats.last_displayed_frame_rtp_timestamp;
        observation_state.last_displayed_at_bucket = current_last_displayed_at_bucket;
        observation_state.host_descriptor_upload_mode =
            stats.video_present_descriptor_upload_mode.clone();
        observation_state.host_descriptor_metal_import_count_total =
            stats.video_present_descriptor_metal_import_count_total;
        observation_state.host_descriptor_cpu_upload_count_total =
            stats.video_present_descriptor_cpu_upload_count_total;
        runtime_trace.record_state(
            "xbxengine",
            "hostMailboxState",
            session_id,
            json!({
                "presentFps": stats.present_fps,
                "hostMailboxEnqueueCountTotal": stats.host_mailbox_enqueue_count_total,
                "hostMailboxDropCountTotal": stats.host_mailbox_drop_count_total,
                "hostMailboxOverwriteCountTotal": stats.host_mailbox_overwrite_count_total,
                "noPendingTakeCountTotal": stats.host_no_pending_take_count_total,
                "noPendingStreak": stats.host_no_pending_streak,
                "noPendingMaxStreak": stats.host_no_pending_max_streak,
                "noPendingPressureLevel": stats.host_no_pending_pressure_level,
                "hostMailboxSubmitEpoch": stats.host_mailbox_submit_epoch,
                "hostDisplayTickEpoch": stats.host_display_tick_epoch,
                "hostFramePresentEpoch": stats.host_frame_present_epoch,
                "cadencePhase": stats.host_cadence_phase,
                "latestHostMailboxSubmitTimeMs": stats.latest_host_mailbox_submit_time_ms,
                "latestHostSubmitRtpTimestamp": stats.latest_video_host_submit_rtp_timestamp,
                "submitAgeMs": stats.submit_age_ms,
                "presentAgeMs": stats.present_age_ms,
                "displayAgeMs": stats.display_age_ms,
                "displayedAgeMs": stats.present_age_ms,
                "latestHostPresentTimeMs": stats.latest_video_host_present_time_ms,
                "hostViewGeneration": stats.host_view_generation,
                "latestHostViewCreatedAtMs": stats.latest_host_view_created_at_ms,
                "displayedFrameStale": displayed_frame_stale(
                    stats.present_age_ms,
                    stats.last_displayed_frame_seq,
                ),
                "retainedOldFrameRisk": retained_old_frame_risk(
                    stats.present_age_ms,
                    stats.last_displayed_frame_seq,
                    stats.host_no_pending_streak,
                    stats.host_cadence_phase.as_deref(),
                ),
                "lastDisplayedFrameSeq": stats.last_displayed_frame_seq,
                "lastDisplayedFrameRtpTimestamp": stats.last_displayed_frame_rtp_timestamp,
                "lastDisplayedAtMs": stats.last_displayed_at_ms,
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

    let feedback_target_availability_signature = Some((
        stats.latest_feedback_target_availability_target.clone(),
        stats.latest_feedback_target_availability_state.clone(),
        stats.latest_feedback_target_availability_reason.clone(),
    ));
    if observation_state.feedback_target_availability_signature
        != feedback_target_availability_signature
        && feedback_target_availability_signature
            .as_ref()
            .is_some_and(|(target, state, reason)| {
                target.is_some() || state.is_some() || reason.is_some()
            })
    {
        observation_state.feedback_target_availability_signature =
            feedback_target_availability_signature;
        runtime_trace.record_event(
            "xbxengine",
            "feedbackTargetAvailabilityChanged",
            session_id,
            json!({
                "target": stats.latest_feedback_target_availability_target,
                "state": stats.latest_feedback_target_availability_state,
                "reason": stats.latest_feedback_target_availability_reason,
                "observedAtMs": stats.latest_feedback_target_availability_observed_at_ms,
                "summary": stats
                    .latest_feedback_target_availability_target
                    .as_deref()
                    .zip(stats.latest_feedback_target_availability_state.as_deref())
                    .zip(stats.latest_feedback_target_availability_reason.as_deref())
                    .map(|((target, state), reason)| format!(
                        "target={target} state={state} reason={reason}"
                    )),
            }),
        );
    }

    if stats.keyframe_request_outcome_seq != observation_state.keyframe_request_outcome_seq {
        observation_state.keyframe_request_outcome_seq = stats.keyframe_request_outcome_seq;
        runtime_trace.record_event(
            "xbxengine",
            "keyframeRequestOutcome",
            session_id,
            json!({
                "seq": stats.keyframe_request_outcome_seq,
                "source": stats.latest_keyframe_request_source,
                "outcome": stats.latest_keyframe_request_outcome,
                "ledgerGeneration": stats.receive_recovery_ledger_generation,
                "mediaSupplyPhase": stats.media_supply_phase,
                "ingressWaitingRtpMarkerTotal": stats.ingress_waiting_rtp_marker_total,
                "ingressWaitingIdrInspectionTotal": stats.ingress_waiting_idr_inspection_total,
            }),
        );
    }

    if stats.receive_feedback_decision_seq != observation_state.receive_feedback_decision_seq {
        observation_state.receive_feedback_decision_seq = stats.receive_feedback_decision_seq;
        let coalescing = stats.latest_receive_feedback_coalescing.clone();
        let action = stats.latest_receive_feedback_action.clone();
        let is_terminal = matches!(
            coalescing.as_deref(),
            Some("target-unavailable") | Some("rate-limited")
        );
        let is_sent = action.as_deref() == Some("requestPli")
            || action.as_deref() == Some("requestFir")
            || action.as_deref() == Some("sendNack");
        let is_mismatch = stats.receive_feedback_arbiter_mismatch_total.unwrap_or(0) > 0;
        if is_sent || is_terminal || is_mismatch || coalescing.as_deref() == Some("fresh-sent") {
            let gap_sequence = stats
                .latest_video_timeline_observation
                .as_ref()
                .and_then(|timeline| timeline.gap.as_ref())
                .and_then(|gap| gap.sequence);
            let nack_packet_count = stats
                .latest_video_nack_observation
                .as_ref()
                .map(|nack| nack.packet_count);
            let h264_verdict =
                stats
                    .latest_h264_inspection_observation
                    .as_ref()
                    .map(|inspection| {
                        if inspection.bootstrap_ready {
                            "bootstrap-ready".to_string()
                        } else if let Some(reason) = inspection.bootstrap_reject_reason.as_ref() {
                            reason.clone()
                        } else if inspection.is_idr {
                            "idr-observed".to_string()
                        } else {
                            inspection
                                .continuation_verdict
                                .clone()
                                .unwrap_or_else(|| "delta-observed".to_string())
                        }
                    });
            let last_keyframe_sent_age_ms = stats
                .receive_keyframe_last_sent_at_ms
                .and_then(|sent_at| {
                    stats
                        .latest_video_receiver_observation
                        .as_ref()
                        .map(|obs| (obs.observed_at_ms - sent_at).max(0.0))
                })
                .or_else(|| {
                    stats
                        .latest_h264_inspection_observation
                        .as_ref()
                        .and_then(|inspection| {
                            stats
                                .receive_keyframe_last_sent_at_ms
                                .map(|sent_at| (inspection.observed_at_ms - sent_at).max(0.0))
                        })
                });
            runtime_trace.record_event(
                "xbxengine",
                "receiveFeedbackDecision",
                session_id,
                json!({
                    "seq": stats.receive_feedback_decision_seq,
                    "action": action,
                    "reason": stats.latest_receive_feedback_reason,
                    "source": stats.latest_receive_feedback_source,
                    "coalescing": coalescing,
                    "outcome": stats.latest_receive_feedback_executor_outcome,
                    "lastKeyframeSentAgeMs": last_keyframe_sent_age_ms,
                    "feedbackTargetState": stats.latest_feedback_target_availability_state,
                    "referenceState": stats.reference_chain_state,
                    "sparseActive": stats
                        .latest_receive_feedback_sparse_active
                        .unwrap_or(false),
                    "sparsePliIntervalMs": stats.receive_sparse_idr_pli_interval_ms,
                    "gapSequence": gap_sequence,
                    "nackPacketCount": nack_packet_count,
                    "h264Verdict": h264_verdict,
                    "arbiterMismatchTotal": stats.receive_feedback_arbiter_mismatch_total,
                    "feedbackCoalescedTotal": stats.receive_feedback_coalesced_total,
                    "feedbackThrottledTotal": stats.receive_feedback_throttled_total,
                    "keyframeRequired": stats.receive_keyframe_required,
                    "keyframeRequiredCause": stats.receive_keyframe_required_cause,
                    "responseState": stats.receive_keyframe_response_state,
                    "receiveDisplayState": stats.receive_display_state,
                    "terminalCandidate": stats.receive_picture_recovery_terminal_candidate,
                    "ledgerGeneration": stats.receive_recovery_ledger_generation,
                    "episodeProjectionState": derive_episode_projection_state(&stats),
                    "displaySupplyStarvedBlocker": derive_display_supply_starved_blocker(&stats),
                }),
            );
        }
    }

    let reference_chain_signature = (
        stats.reference_chain_state.clone().unwrap_or_default(),
        stats
            .reference_chain_state_cause
            .clone()
            .unwrap_or_default(),
    );
    if observation_state.reference_chain_signature.as_ref() != Some(&reference_chain_signature) {
        observation_state.reference_chain_signature = Some(reference_chain_signature.clone());
        if stats.reference_chain_state.is_some() {
            let bootstrap_reject_reason = stats
                .latest_h264_inspection_observation
                .as_ref()
                .and_then(|inspection| inspection.bootstrap_reject_reason.clone());
            let displayed_idr_host_hint = stats.recovery_displayed_idr_at_ms.is_some();
            runtime_trace.record_event(
                "xbxengine",
                "referenceChainStateChanged",
                session_id,
                json!({
                    "state": stats.reference_chain_state,
                    "cause": stats.reference_chain_state_cause,
                    "decoderReferenceSynced": stats.reference_chain_decoder_reference_synced,
                    "bootstrapReady": stats.reference_chain_bootstrap_ready,
                    "bootstrapRejectReason": bootstrap_reject_reason,
                    "hasActiveGap": stats.reference_chain_has_active_gap,
                    "nackExhausted": stats.reference_chain_nack_exhausted,
                    "submitAgeMs": stats.reference_chain_submit_age_ms,
                    "displayedIdrHostHint": displayed_idr_host_hint,
                    "displayedIdrHostHintDiagnostic": true,
                    "sparseMustIdrMismatch": stats.latest_reference_chain_sparse_must_idr_mismatch,
                    "sparseMustIdrMismatchTotal": stats.receive_sparse_must_idr_mismatch_total,
                    "source": stats
                        .latest_reference_chain_observation_source
                        .as_deref()
                        .unwrap_or("ledger"),
                    "responseState": stats.receive_keyframe_response_state,
                    "receiveDisplayState": stats.receive_display_state,
                    "referenceStatsFallbackTotal": stats.reference_stats_fallback_total,
                    "keyframeRequired": stats.receive_keyframe_required,
                    "ledgerGeneration": stats.receive_recovery_ledger_generation,
                    "episodeProjectionState": derive_episode_projection_state(&stats),
                    "displaySupplyStarvedBlocker": derive_display_supply_starved_blocker(&stats),
                }),
            );
        }
    }

    if stats
        .latest_receive_picture_recovery_terminal_reason
        .is_some()
    {
        let terminal_signature = (
            stats
                .latest_receive_picture_recovery_terminal_reason
                .clone()
                .unwrap_or_default(),
            stats.receive_picture_recovery_terminal_total.unwrap_or(0),
        );
        if observation_state
            .picture_recovery_terminal_signature
            .as_ref()
            != Some(&terminal_signature)
        {
            observation_state.picture_recovery_terminal_signature = Some(terminal_signature);
            runtime_trace.record_event(
                "xbxengine",
                "receivePictureRecoveryTerminal",
                session_id,
                json!({
                    "reason": stats.latest_receive_picture_recovery_terminal_reason,
                    "terminalTotal": stats.receive_picture_recovery_terminal_total,
                    "referenceState": stats.reference_chain_state,
                    "sentCountUnresolved": stats.receive_keyframe_sent_count_unresolved,
                    "responseState": stats.receive_keyframe_response_state,
                    "receiveDisplayState": stats.receive_display_state,
                    "keyframeRequired": stats.receive_keyframe_required,
                    "elapsedRttCount": stats.receive_picture_recovery_terminal_elapsed_rtt_count,
                    "ledgerGeneration": stats.receive_recovery_ledger_generation,
                    "episodeProjectionState": derive_episode_projection_state(&stats),
                    "nextOwnerHint": "connectivity-reconnect-candidate",
                }),
            );
        }
    }

    let insert_gate_signature = (
        stats.latest_insert_decision.clone().unwrap_or_default(),
        stats
            .latest_insert_decision_reason
            .clone()
            .unwrap_or_default(),
        stats.insert_decode_bypass_aligned,
        stats.insert_hold_decode_bypass_mismatch_total.unwrap_or(0),
    );
    if observation_state.insert_gate_signature.as_ref() != Some(&insert_gate_signature) {
        observation_state.insert_gate_signature = Some(insert_gate_signature.clone());
        if stats.latest_insert_decision.is_some() && !stale_insert_projection_under_must_idr(stats)
        {
            runtime_trace.record_event(
                "xbxengine",
                "insertGateDecision",
                session_id,
                json!({
                    "decision": stats.latest_insert_decision,
                    "reason": stats.latest_insert_decision_reason,
                    "bypassAligned": stats.insert_decode_bypass_aligned,
                    "holdDecodeBypassMismatchTotal": stats.insert_hold_decode_bypass_mismatch_total,
                    "keyframeRequired": stats.receive_keyframe_required,
                    "responseState": stats.receive_keyframe_response_state,
                    "receiveDisplayState": stats.receive_display_state,
                    "ledgerGeneration": stats.receive_recovery_ledger_generation,
                    "packetRecoveryActionStage": stats.latest_packet_recovery_action_stage,
                    "referenceState": stats.reference_chain_state,
                    "mediaSupplyPhaseDiagnostic": stats.media_supply_phase,
                }),
            );
        }
    }

    if observation_state.picture_recovery_authority != stats.recovery_picture_recovery_authority {
        observation_state.picture_recovery_authority =
            stats.recovery_picture_recovery_authority.clone();
        if stats.recovery_picture_recovery_authority.is_some() {
            runtime_trace.record_event(
                "xbxengine",
                "pictureRecoveryAuthority",
                session_id,
                json!({
                    "authority": stats.recovery_picture_recovery_authority,
                    "delegatedTotal": stats.recovery_picture_recovery_delegated_total,
                    "sessionKeyframeInFlight": stats.recovery_session_keyframe_in_flight,
                    "sparseIdrPliIntervalMs": stats.receive_sparse_idr_pli_interval_ms,
                }),
            );
        }
    }

    let delegated_total = stats.recovery_picture_recovery_delegated_total.unwrap_or(0);
    if observation_state.picture_recovery_delegated_total != delegated_total {
        observation_state.picture_recovery_delegated_total = delegated_total;
        if delegated_total > 0 {
            runtime_trace.record_event(
                "xbxengine",
                "pictureRecoveryDelegated",
                session_id,
                json!({
                    "delegatedTotal": delegated_total,
                    "summary": stats.latest_observation_summary,
                    "authority": stats.recovery_picture_recovery_authority,
                    "sessionKeyframeInFlight": stats.recovery_session_keyframe_in_flight,
                }),
            );
        }
    }

    if observation_state.session_keyframe_in_flight != stats.recovery_session_keyframe_in_flight {
        observation_state.session_keyframe_in_flight = stats.recovery_session_keyframe_in_flight;
        if let Some(in_flight) = stats.recovery_session_keyframe_in_flight {
            runtime_trace.record_event(
                "xbxengine",
                "sessionKeyframeInFlight",
                session_id,
                json!({
                    "inFlight": in_flight,
                    "pictureRecoveryAuthority": stats.recovery_picture_recovery_authority,
                }),
            );
        }
    }

    if observation_state.latest_observation_label != stats.latest_observation_label
        || observation_state.latest_observation_summary != stats.latest_observation_summary
    {
        observation_state.latest_observation_label = stats.latest_observation_label.clone();
        observation_state.latest_observation_summary = stats.latest_observation_summary.clone();
        if stats.latest_observation_label.as_deref() == Some("videoDecoderLocalResetFailed") {
            runtime_trace.record_event(
                "xbxengine",
                "videoDecoderLocalResetFailed",
                session_id,
                json!({
                    "summary": stats.latest_observation_summary,
                }),
            );
        }
        match stats.latest_observation_label.as_deref() {
            Some("keyframeRequestSent") => {
                runtime_trace.record_event(
                    "xbxengine",
                    "keyframeRequestSent",
                    session_id,
                    json!({
                        "summary": stats.latest_observation_summary,
                    }),
                );
            }
            Some("twccReceiverMappingMissing") => {
                runtime_trace.record_event(
                    "xbxengine",
                    "twccReceiverMappingMissing",
                    session_id,
                    json!({
                        "summary": stats.latest_observation_summary,
                    }),
                );
            }
            Some("pictureRecoveryDelegated") => {
                runtime_trace.record_event(
                    "xbxengine",
                    "pictureRecoveryDelegated",
                    session_id,
                    json!({
                        "summary": stats.latest_observation_summary,
                        "authority": stats.recovery_picture_recovery_authority,
                        "sessionKeyframeInFlight": stats.recovery_session_keyframe_in_flight,
                    }),
                );
            }
            _ => {}
        }
    }

    if stats
        .latest_turn_relay_observation_seq
        .is_some_and(|seq| observation_state.latest_turn_relay_observation_seq != Some(seq))
    {
        observation_state.latest_turn_relay_observation_seq =
            stats.latest_turn_relay_observation_seq;
        if let Some(label) = stats.latest_turn_relay_observation_label.as_deref() {
            runtime_trace.record_event(
                "xbxengine",
                label,
                session_id,
                json!({
                    "summary": stats.latest_turn_relay_observation_summary,
                    "observationSeq": stats.latest_turn_relay_observation_seq,
                }),
            );
        }
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
                "submitAgeMs": stats.submit_age_ms,
                "displayAgeMs": stats.display_age_ms,
                "latestHostSubmitRtpTimestamp": stats.latest_video_host_submit_rtp_timestamp,
                "lastDisplayedFrameRtpTimestamp": stats.last_displayed_frame_rtp_timestamp,
                "hostViewGeneration": stats.host_view_generation,
                "latestHostViewCreatedAtMs": stats.latest_host_view_created_at_ms,
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
        "response-observed" => Some("keyframeRequestEpisodeResponseObserved"),
        "packet-seen" => Some("keyframeRequestEpisodePacketSeen"),
        "decoded" => Some("keyframeRequestEpisodeDecoded"),
        "missed" => Some("keyframeRequestEpisodeMissed"),
        "succeeded" => Some("keyframeRequestEpisodeSucceeded"),
        "deferred" => Some("keyframeRequestEpisodeDeferred"),
        "failed" => Some("keyframeRequestEpisodeFailed"),
        "expired-unsent" => Some("keyframeRequestEpisodeUnsentExpired"),
        _ => None,
    }
}

fn diagnostic_keyframe_suppression_reason(
    episode: &xbxengine_protocol::XbxEngineKeyframeRequestEpisodeObservationDto,
) -> Option<String> {
    if let Some(detail) = episode
        .transport_detail
        .as_deref()
        .filter(|detail| is_transport_suppression_detail(detail))
    {
        return Some(detail.to_string());
    }
    if let Some(detail) = episode
        .status_detail
        .as_deref()
        .filter(|detail| is_transport_suppression_detail(detail))
    {
        return Some(detail.to_string());
    }
    match episode.status.as_str() {
        "deferred" => episode
            .transport_detail
            .clone()
            .or_else(|| episode.status_detail.clone())
            .or_else(|| Some("deferred".to_string())),
        "expired-unsent" => episode
            .transport_detail
            .clone()
            .or_else(|| episode.status_detail.clone())
            .or_else(|| Some("expired-unsent".to_string())),
        _ => None,
    }
}

fn is_transport_suppression_detail(detail: &str) -> bool {
    detail.contains("coalesced:")
        || detail.contains("familyInFlight:")
        || detail.contains("videoRtcpFeedbackTransportNotReady")
        || detail.contains("videoRtcpFeedbackTargetPending")
        || detail.contains("videoFeedbackWarming")
        || detail.contains("controlPending")
        || detail.contains("transport-await")
        || detail.contains("transport-suppressed")
}

fn clean_anchor_funnel_event_name(source_event: &str) -> Option<&'static str> {
    match source_event {
        "frame-complete-candidate" => Some("cleanAnchorCompleteCandidateObserved"),
        "frame-complete-candidate-decode-feedback-blocked" => {
            Some("cleanAnchorCompleteCandidateBlocked")
        }
        _ => None,
    }
}

fn clean_anchor_funnel_payload(
    stats: &XbxEngineStatsDto,
    timeline: &xbxengine_protocol::XbxEngineVideoTimelineObservationDto,
) -> serde_json::Value {
    let anchor = stats.latest_anchor_candidate_ledger.as_ref();
    json!({
        "observationId": timeline.observation_id,
        "sourceEvent": timeline.source_event,
        "frameRtpTimestamp": timeline.frame.as_ref().and_then(|frame| frame.frame_rtp_timestamp),
        "frameState": timeline.frame.as_ref().map(|frame| frame.state.clone()),
        "frameIsKeyframe": timeline.frame.as_ref().and_then(|frame| frame.is_keyframe),
        "frameImportance": timeline.frame.as_ref().map(|frame| frame.frame_importance.clone()),
        "chainState": timeline.chain.state,
        "chainReason": timeline.chain.reason,
        "recoveryEpoch": anchor
            .map(|candidate| candidate.recovery_epoch)
            .or_else(|| diagnostic_recovery_epoch(stats)),
        "anchorState": anchor.map(|candidate| candidate.state.clone()),
        "anchorFailureReason": anchor.and_then(|candidate| candidate.failure_reason.clone()),
        "anchorSourceEvent": anchor.map(|candidate| candidate.source_event.clone()),
        "observedAtMs": timeline.observed_at_ms,
    })
}

fn keyframe_request_episode_payload(
    episode: Option<&xbxengine_protocol::XbxEngineKeyframeRequestEpisodeObservationDto>,
    h264_inspection: Option<&xbxengine_protocol::XbxEngineH264InspectionObservationDto>,
    latest_rtcp_send_failure: Option<&(f64, String)>,
    stats: &XbxEngineStatsDto,
) -> serde_json::Value {
    match episode {
        Some(episode) => json!({
            "episodeId": episode.episode_id,
            "requestReason": episode.request_reason.clone(),
            "requestKind": episode.request_kind.clone(),
            "status": episode.status.clone(),
            "statusDetail": episode.status_detail.clone(),
            "lifecyclePhase": episode.lifecycle_phase.clone(),
            "requestedAtMs": episode.requested_at_ms,
            "sentAtMs": episode.sent_at_ms,
            "deadlineAtMs": episode.deadline_at_ms,
            "transportDetail": episode.transport_detail.clone(),
            "firstVideoPacketAtMs": episode.first_video_packet_at_ms,
            "firstVideoPacketRtpTimestamp": episode.first_video_packet_rtp_timestamp,
            "firstVideoPacketIsKeyframe": episode.first_video_packet_is_keyframe,
            "firstKeyframePacketAtMs": episode.first_keyframe_packet_at_ms,
            "firstKeyframeDecodedAtMs": episode.first_keyframe_decoded_at_ms,
            "responseRtpTimestamp": episode.response_rtp_timestamp,
            "responseFrameSeq": episode.response_frame_seq,
            "responseVerdict": episode.response_verdict.clone(),
            "retiredAtMs": episode.retired_at_ms,
            "familyId": episode.family_id.clone(),
            "ownerEpisodeId": episode.owner_episode_id,
            "suppressDurationMs": episode.suppress_duration_ms,
            "releaseReason": episode.release_reason.clone(),
            "requestToFirstPacketMs": duration_ms(episode.requested_at_ms, episode.first_keyframe_packet_at_ms),
            "requestToFirstDecodeMs": duration_ms(episode.requested_at_ms, episode.first_keyframe_decoded_at_ms),
            "sentToFirstPacketMs": episode.sent_at_ms.and_then(|sent_at_ms| duration_ms(sent_at_ms, episode.first_keyframe_packet_at_ms)),
            "sentToFirstDecodeMs": episode.sent_at_ms.and_then(|sent_at_ms| duration_ms(sent_at_ms, episode.first_keyframe_decoded_at_ms)),
            "timedOut": keyframe_episode_timed_out(episode),
            "recentRtcpSendFailureObservedAtMs": latest_rtcp_send_failure.map(|(observed_at_ms, _)| *observed_at_ms),
            "recentRtcpSendFailureReason": latest_rtcp_send_failure.map(|(_, reason)| reason.clone()),
            "linkedH264BootstrapRejectReason": h264_inspection.and_then(|inspection| inspection.bootstrap_reject_reason.clone()),
            "linkedH264AdmissionAccepted": h264_inspection.map(|inspection| inspection.admission_accepted),
            "linkedH264ObservedAtMs": h264_inspection.map(|inspection| inspection.observed_at_ms),
            "diagnosticRecoveryEpoch": diagnostic_recovery_epoch(stats),
            "diagnosticTimelineSourceEvent": stats.latest_video_timeline_observation.as_ref().map(|timeline| timeline.source_event.clone()),
            "diagnosticTimelineChainState": stats.latest_video_timeline_observation.as_ref().map(|timeline| timeline.chain.state.clone()),
            "diagnosticTimelineChainReason": stats.latest_video_timeline_observation.as_ref().and_then(|timeline| timeline.chain.reason.clone()),
            "diagnosticAnchorState": stats.latest_anchor_candidate_ledger.as_ref().map(|anchor| anchor.state.clone()),
            "diagnosticAnchorFailureReason": stats.latest_anchor_candidate_ledger.as_ref().and_then(|anchor| anchor.failure_reason.clone()),
            "diagnosticAnchorSourceEvent": stats.latest_anchor_candidate_ledger.as_ref().map(|anchor| anchor.source_event.clone()),
            "diagnosticDecodeCandidateAction": stats.latest_decode_candidate_decision.as_ref().map(|candidate| candidate.action.clone()),
            "diagnosticDecodeCandidateDetail": stats.latest_decode_candidate_decision.as_ref().map(|candidate| candidate.detail.clone()),
            "diagnosticDecodeOutputDetail": stats.latest_decode_output_path_observation.as_ref().map(|output| output.detail.clone()),
            "diagnosticFrameDropDetail": stats.latest_video_frame_drop.as_ref().and_then(|drop| drop.detail.clone()),
            "diagnosticFrameDropQueueDepth": stats.latest_video_frame_drop.as_ref().map(|drop| drop.queue_depth),
            "diagnosticPendingReason": diagnostic_pending_reason(stats, h264_inspection),
        }),
        None => json!(null),
    }
}

fn picture_recovery_transition_payload(
    observation: &xbxengine_protocol::XbxEnginePictureRecoveryTransitionObservationDto,
) -> serde_json::Value {
    json!({
        "observationId": observation.observation_id,
        "episodeId": observation.episode_id,
        "recoveryEpoch": observation.recovery_epoch,
        "phase": observation.phase,
        "fromPhase": observation.from_phase,
        "toPhase": observation.to_phase,
        "cause": observation.cause,
        "detail": observation.detail,
        "rtpTimestamp": observation.rtp_timestamp,
        "frameSeq": observation.frame_seq,
        "ownerState": observation.owner_state,
        "transportState": observation.transport_state,
        "observedAtMs": observation.observed_at_ms,
    })
}

fn enrich_display_stable_transition_payload(payload: &mut Value, stats: &XbxEngineStatsDto) {
    if payload.get("toPhase").and_then(Value::as_str) != Some("DisplayStable") {
        return;
    }
    let Some(object) = payload.as_object_mut() else {
        return;
    };
    object.insert(
        "keyframeRequired".to_string(),
        json!(stats.receive_keyframe_required),
    );
    object.insert(
        "responseState".to_string(),
        json!(stats.receive_keyframe_response_state),
    );
    object.insert(
        "receiveDisplayState".to_string(),
        json!(stats.receive_display_state),
    );
    object.insert(
        "ledgerGeneration".to_string(),
        json!(stats.receive_recovery_ledger_generation),
    );
}

fn picture_recovery_blocker_payload(
    observation: &xbxengine_protocol::XbxEnginePictureRecoveryBlockerObservationDto,
) -> serde_json::Value {
    json!({
        "observationId": observation.observation_id,
        "episodeId": observation.episode_id,
        "recoveryEpoch": observation.recovery_epoch,
        "gate": observation.gate,
        "blockerKind": observation.blocker_kind,
        "severity": observation.severity,
        "firstObservedAtMs": observation.first_observed_at_ms,
        "observedAtMs": observation.observed_at_ms,
        "count": observation.count,
        "frameRtpTimestamp": observation.frame_rtp_timestamp,
        "frameSeq": observation.frame_seq,
        "ownerState": observation.owner_state,
        "transportState": observation.transport_state,
    })
}

fn video_ingress_termination_payload(
    observation: &xbxengine_protocol::XbxEngineVideoIngressTerminationObservationDto,
) -> serde_json::Value {
    json!({
        "observationId": observation.observation_id,
        "terminationId": observation.termination_id,
        "derivedFromTerminationId": observation.derived_from_termination_id,
        "kind": observation.kind,
        "cause": observation.cause,
        "upstreamCause": observation.upstream_cause,
        "sourceSubsystem": observation.source_subsystem,
        "linkedRecoveryEpoch": observation.linked_recovery_epoch,
        "linkedEpisodeId": observation.linked_episode_id,
        "transportState": observation.transport_state,
        "ownerState": observation.owner_state,
        "videoTrackState": observation.video_track_state,
        "recentCommand": observation.recent_command,
        "observedAtMs": observation.observed_at_ms,
    })
}

fn first_frame_latency_payload(
    observation: &xbxengine_protocol::XbxEngineFirstFrameLatencyObservationDto,
) -> serde_json::Value {
    json!({
        "observationId": observation.observation_id,
        "episodeId": observation.episode_id,
        "recoveryEpoch": observation.recovery_epoch,
        "controlReadyToPliSentMs": observation.control_ready_to_pli_sent_ms,
        "pliSentToFirstIdrPacketMs": observation.pli_sent_to_first_idr_packet_ms,
        "firstIdrPacketToFirstDecodeMs": observation.first_idr_packet_to_first_decode_ms,
        "firstDecodeToCleanAnchorCommittedMs": observation.first_decode_to_clean_anchor_committed_ms,
        "cleanAnchorCommittedToDisplayStableMs": observation.clean_anchor_committed_to_display_stable_ms,
        "terminalPhase": observation.terminal_phase,
        "incompleteReason": observation.incomplete_reason,
        "observedAtMs": observation.observed_at_ms,
    })
}

fn diagnostic_recovery_epoch(stats: &XbxEngineStatsDto) -> Option<u64> {
    stats
        .latest_recovery_decision_ledger
        .as_ref()
        .and_then(|ledger| {
            ledger
                .budget_after
                .as_ref()
                .or(ledger.budget_before.as_ref())
        })
        .map(|budget| budget.recovery_epoch)
}

fn diagnostic_pending_reason(
    stats: &XbxEngineStatsDto,
    h264_inspection: Option<&xbxengine_protocol::XbxEngineH264InspectionObservationDto>,
) -> Option<String> {
    let anchor_rejected = stats
        .latest_anchor_candidate_ledger
        .as_ref()
        .is_some_and(|anchor| matches!(anchor.state.as_str(), "rejected" | "awaiting-recovery"));
    let h264_reason =
        h264_inspection.and_then(|inspection| inspection.bootstrap_reject_reason.as_deref());
    if anchor_rejected {
        if let Some(reason) = h264_reason {
            return Some(format!("anchorRejected:h264Reject:{reason}"));
        }
        if let Some(reason) = stats
            .latest_anchor_candidate_ledger
            .as_ref()
            .and_then(|anchor| anchor.failure_reason.as_deref())
        {
            return Some(format!("anchorRejected:{reason}"));
        }
        return Some("anchorRejected".to_string());
    }
    if let Some(reason) = h264_reason {
        return Some(format!("h264Reject:{reason}"));
    }
    if let Some(candidate) = stats.latest_decode_candidate_decision.as_ref() {
        return Some(format!(
            "decodeCandidate:{}:{}",
            candidate.action, candidate.detail
        ));
    }
    if let Some(frame_drop) = stats.latest_video_frame_drop.as_ref() {
        if let Some(detail) = frame_drop.detail.as_deref() {
            return Some(format!("frameDrop:{detail}"));
        }
        return Some(format!("frameDrop:{}", frame_drop.reason));
    }
    if let Some(timeline) = stats.latest_video_timeline_observation.as_ref() {
        if let Some(reason) = timeline.chain.reason.as_deref() {
            return Some(format!("timeline:{reason}"));
        }
        return Some(format!("timeline:{}", timeline.source_event));
    }
    None
}

fn find_keyframe_episode_dto(
    stats: &XbxEngineStatsDto,
    episode_id: u64,
) -> Option<xbxengine_protocol::XbxEngineKeyframeRequestEpisodeObservationDto> {
    stats
        .latest_keyframe_request_episode
        .as_ref()
        .filter(|episode| episode.episode_id == episode_id)
        .cloned()
        .or_else(|| {
            stats
                .recent_keyframe_request_episodes
                .iter()
                .find(|episode| episode.episode_id == episode_id)
                .cloned()
        })
}

fn collect_keyframe_episode_dto_candidates(
    stats: &XbxEngineStatsDto,
) -> Vec<xbxengine_protocol::XbxEngineKeyframeRequestEpisodeObservationDto> {
    let mut out: Vec<xbxengine_protocol::XbxEngineKeyframeRequestEpisodeObservationDto> =
        Vec::new();
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

fn keyframe_episode_dto_observability_active(
    episode: &xbxengine_protocol::XbxEngineKeyframeRequestEpisodeObservationDto,
) -> bool {
    episode.retired_at_ms.is_none()
}

fn select_keyframe_episode_dto_for_h264(
    stats: &XbxEngineStatsDto,
    inspection: &xbxengine_protocol::XbxEngineH264InspectionObservationDto,
) -> Option<xbxengine_protocol::XbxEngineKeyframeRequestEpisodeObservationDto> {
    let candidates = collect_keyframe_episode_dto_candidates(stats);
    if let Some(rtp) = inspection.frame_rtp_timestamp {
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
    const WINDOW_MS: f64 = 10_000.0;
    let mut best: Option<xbxengine_protocol::XbxEngineKeyframeRequestEpisodeObservationDto> = None;
    let mut best_delta = f64::INFINITY;
    for episode in candidates.iter().filter(|episode| {
        keyframe_episode_dto_observability_active(episode)
            && episode.request_reason.as_deref() == Some("receiverWaitingKeyframe")
    }) {
        let anchor_ms = episode.sent_at_ms.unwrap_or(episode.requested_at_ms);
        let delta = (inspection.observed_at_ms - anchor_ms).abs();
        if delta < WINDOW_MS && delta < best_delta {
            best_delta = delta;
            best = Some(episode.clone());
        }
    }
    best
}

fn resolve_h264_linked_episode(
    stats: &XbxEngineStatsDto,
    observation: &xbxengine_protocol::XbxEngineH264InspectionObservationDto,
) -> Option<xbxengine_protocol::XbxEngineKeyframeRequestEpisodeObservationDto> {
    if let Some(id) = observation.bound_episode_id {
        if let Some(episode) = find_keyframe_episode_dto(stats, id) {
            return Some(episode);
        }
    }
    select_keyframe_episode_dto_for_h264(stats, observation)
}

fn resolve_is_recovery_keyframe_response_context(
    observation: &xbxengine_protocol::XbxEngineH264InspectionObservationDto,
    linked: Option<&xbxengine_protocol::XbxEngineKeyframeRequestEpisodeObservationDto>,
) -> bool {
    if let Some(flag) = observation.bound_as_recovery_response {
        return flag;
    }
    keyframe_episode_response_context(linked, Some(observation))
}

fn h264_inspection_payload(
    observation: Option<&xbxengine_protocol::XbxEngineH264InspectionObservationDto>,
    stats: &XbxEngineStatsDto,
) -> serde_json::Value {
    match observation {
        Some(observation) => {
            let linked_full = resolve_h264_linked_episode(stats, observation);
            let is_recovery =
                resolve_is_recovery_keyframe_response_context(observation, linked_full.as_ref());
            let (linked_id, linked_status, linked_reason, linked_verdict) = if is_recovery {
                if let Some(ref episode) = linked_full {
                    (
                        Some(episode.episode_id),
                        Some(episode.status.clone()),
                        episode.request_reason.clone(),
                        episode.response_verdict.clone(),
                    )
                } else {
                    (
                        observation.bound_episode_id,
                        observation.bound_episode_status.clone(),
                        None,
                        None,
                    )
                }
            } else {
                (None, None, None, None)
            };
            json!({
                "observationId": observation.observation_id,
                "frameRtpTimestamp": observation.frame_rtp_timestamp,
                "nalTypes": observation.nal_types.clone(),
                "nalCount": observation.nal_count,
                "vclNalCount": observation.vcl_nal_count,
                "hasInbandSps": observation.has_inband_sps,
                "hasInbandPps": observation.has_inband_pps,
                "committedSpsPresent": observation.committed_sps_present,
                "committedPpsPresent": observation.committed_pps_present,
                "sliceHeadersValid": observation.slice_headers_valid,
                "deltaContinuationReady": observation.delta_continuation_ready,
                "parameterSetsChanged": observation.parameter_sets_changed,
                "configChanged": observation.config_changed,
                "isIdr": observation.is_idr,
                "sampleWidth": observation.sample_width,
                "sampleHeight": observation.sample_height,
                "bootstrapReady": observation.bootstrap_ready,
                "bootstrapRejectReason": observation.bootstrap_reject_reason.clone(),
                "continuationVerdict": observation.continuation_verdict.clone(),
                "admissionAccepted": observation.admission_accepted,
                "observedAtMs": observation.observed_at_ms,
                "boundEpisodeId": observation.bound_episode_id,
                "boundEpisodeStatus": observation.bound_episode_status.clone(),
                "boundAsRecoveryResponse": observation.bound_as_recovery_response,
                "boundResponseRtpTimestamp": observation.bound_response_rtp_timestamp,
                "boundRecoveryEpoch": observation.bound_recovery_epoch,
                "episodePhaseAtObservation": observation.episode_phase_at_observation.clone(),
                "isPostRecoveryDegradation": observation.is_post_recovery_degradation,
                "rejectClassification": observation.reject_classification.clone(),
                "linkedEpisodeId": linked_id,
                "linkedEpisodeStatus": linked_status,
                "linkedEpisodeRequestReason": linked_reason,
                "linkedEpisodeResponseVerdict": linked_verdict,
                "isRecoveryKeyframeResponseContext": is_recovery,
                "usableIdrOutcome": resolve_usable_idr_outcome(linked_full.as_ref(), observation),
            })
        }
        None => json!(null),
    }
}

fn duration_ms(start_ms: f64, end_ms: Option<f64>) -> Option<f64> {
    end_ms.map(|end_ms| (end_ms - start_ms).max(0.0))
}

fn resolve_usable_idr_outcome(
    episode: Option<&xbxengine_protocol::XbxEngineKeyframeRequestEpisodeObservationDto>,
    observation: &xbxengine_protocol::XbxEngineH264InspectionObservationDto,
) -> Option<&'static str> {
    if !matches!(
        observation.bootstrap_reject_reason.as_deref(),
        Some("bootstrapMissingIdr" | "NonIdrVcl")
    ) {
        return None;
    }
    let episode = episode?;
    if episode
        .first_keyframe_decoded_at_ms
        .is_some_and(|decoded_at_ms| decoded_at_ms >= observation.observed_at_ms)
    {
        return Some("beforeUsableIdr");
    }
    if episode.first_keyframe_decoded_at_ms.is_none()
        && (episode.first_keyframe_packet_at_ms.is_some()
            || matches!(episode.status.as_str(), "packet-seen" | "response-observed"))
    {
        return Some("missingUsableIdr");
    }
    if matches!(
        episode.status.as_str(),
        "missed" | "failed" | "succeeded" | "decoded"
    ) || episode.retired_at_ms.is_some()
    {
        return Some("missingUsableIdr");
    }
    None
}

fn keyframe_episode_timed_out(
    episode: &xbxengine_protocol::XbxEngineKeyframeRequestEpisodeObservationDto,
) -> bool {
    episode.status == "missed" || matches!(episode.response_verdict.as_deref(), Some("missed"))
}

fn keyframe_episode_response_context(
    keyframe_episode: Option<&xbxengine_protocol::XbxEngineKeyframeRequestEpisodeObservationDto>,
    h264_inspection: Option<&xbxengine_protocol::XbxEngineH264InspectionObservationDto>,
) -> bool {
    let Some(keyframe_episode) = keyframe_episode else {
        return false;
    };
    if keyframe_episode.request_reason.as_deref() != Some("receiverWaitingKeyframe") {
        return false;
    }
    if !matches!(
        keyframe_episode.status.as_str(),
        "packet-seen" | "decoded" | "response-observed"
    ) {
        return false;
    }
    let Some(h264_inspection) = h264_inspection else {
        return false;
    };
    match (
        keyframe_episode.response_rtp_timestamp,
        h264_inspection.frame_rtp_timestamp,
    ) {
        (Some(episode_rtp_timestamp), Some(frame_rtp_timestamp)) => {
            episode_rtp_timestamp == frame_rtp_timestamp
        }
        _ => false,
    }
}

fn latest_rtcp_send_failure_from_stats(stats: &XbxEngineStatsDto) -> Option<(f64, String)> {
    if let (Some(observed_at_ms), Some(reason)) = (
        stats.latest_video_rtcp_send_failure_time_ms,
        stats.latest_video_rtcp_send_failure_reason.clone(),
    ) {
        return Some((observed_at_ms, reason));
    }
    if stats.latest_observation_label.as_deref() != Some("rtcVideoRtcpSendFailed") {
        return None;
    }
    let summary = stats.latest_observation_summary.as_deref()?;
    let prefix = "video rtcp send failed at ";
    let rest = summary.strip_prefix(prefix)?;
    let (observed_at_ms_text, reason_text) = rest.split_once(" reason=")?;
    let observed_at_ms = observed_at_ms_text.trim().parse::<f64>().ok()?;
    Some((observed_at_ms, reason_text.to_string()))
}

fn latest_rtcp_send_failure_snapshot(stats: &XbxEngineStatsDto) -> Option<serde_json::Value> {
    let (observed_at_ms, reason) = latest_rtcp_send_failure_from_stats(stats)?;
    Some(json!({
        "observedAtMs": observed_at_ms,
        "reason": reason,
    }))
}

fn is_timeout_source_event(source_event: &str) -> bool {
    source_event.starts_with("timeout-")
}

fn is_chain_transition_source_event(source_event: &str) -> bool {
    matches!(
        source_event,
        "chain-broken" | "chain-recovery-anchor-requested" | "chain-clean-anchor-submitted"
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
