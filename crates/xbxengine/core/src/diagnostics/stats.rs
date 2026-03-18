use xbxengine_protocol::XbxEngineStatsDto;

use crate::{XbxEngineMediaRuntimeStats, XbxEngineRuntimeSnapshot};

/**
 * 统计聚合先保持轻量：
 * - 不改当前事件/字段合同
 * - 只把 runtime 快照与媒体 runtime stats 的拼接逻辑独立出来
 * 后续补全 RTT/loss/jitter/bitrate 时，优先继续扩在这里。
 */
pub fn build_xbxengine_stats(
    snapshot: &XbxEngineRuntimeSnapshot,
    runtime_stats: Option<&XbxEngineMediaRuntimeStats>,
) -> XbxEngineStatsDto {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0);
    let transport_state = runtime_stats.map(|stats| format!("{:?}", stats.transport_state));
    let packet_age_ms = runtime_stats
        .and_then(|stats| stats.latest_video_packet_arrival_time_ms)
        .map(|at| (now_ms - at).max(0.0));
    let decode_age_ms = runtime_stats
        .and_then(|stats| stats.latest_video_decode_ok_time_ms)
        .map(|at| (now_ms - at).max(0.0));
    let present_age_ms = runtime_stats
        .and_then(|stats| stats.latest_video_present_time_ms)
        .map(|at| (now_ms - at).max(0.0));
    let packet_to_decode_ms = runtime_stats.and_then(|stats| {
        stats
            .latest_video_packet_arrival_time_ms
            .zip(stats.latest_video_decode_ok_time_ms)
            .map(|(packet_at_ms, decode_at_ms)| (decode_at_ms - packet_at_ms).max(0.0))
    });
    let decode_to_present_ms = runtime_stats.and_then(|stats| {
        stats
            .latest_video_decode_ok_time_ms
            .zip(stats.latest_video_present_time_ms)
            .map(|(decode_at_ms, present_at_ms)| (present_at_ms - decode_at_ms).max(0.0))
    });
    let packet_to_present_ms = runtime_stats.and_then(|stats| {
        stats
            .latest_video_packet_arrival_time_ms
            .zip(stats.latest_video_present_time_ms)
            .map(|(packet_at_ms, present_at_ms)| (present_at_ms - packet_at_ms).max(0.0))
    });
    let resolution = runtime_stats
        .and_then(|stats| stats.latest_video_frame.as_ref())
        .map(|frame| format!("{}x{}", frame.width, frame.height))
        .or_else(|| {
            runtime_stats.and_then(|stats| {
                match (
                    stats.latest_video_stream_width,
                    stats.latest_video_stream_height,
                ) {
                    (Some(width), Some(height)) if width > 0 && height > 0 => {
                        Some(format!("{width}x{height}"))
                    }
                    _ => None,
                }
            })
        })
        .unwrap_or_default();
    let fps = runtime_stats
        .and_then(|stats| stats.latest_video_frame.as_ref())
        .map(|frame| frame.fps)
        .unwrap_or_default();
    let packet_loss = runtime_stats
        .map(|stats| format!("{:.2}%", stats.inbound_video_loss_ratio_5s * 100.0))
        .unwrap_or_default();
    let rtt = runtime_stats
        .and_then(|stats| stats.video_rtt_ms)
        .map(|value| format!("{value:.1}ms"))
        .unwrap_or_default();
    let bitrate = runtime_stats
        .and_then(|stats| stats.inbound_video_bitrate_kbps)
        .map(|value| format!("{:.1}Mbps", value / 1_000.0))
        .or_else(|| {
            runtime_stats
                .and_then(|stats| stats.video_remb_bps)
                .map(|value| format!("{:.1}Mbps", value as f64 / 1_000_000.0))
        })
        .unwrap_or_default();
    let jitter = runtime_stats
        .and_then(|stats| stats.inbound_video_jitter_ms)
        .map(|value| format!("{value:.1}ms"))
        .unwrap_or_default();
    let video_health = classify_video_health(runtime_stats);
    let stall_kind = classify_stall_kind(runtime_stats);
    let runtime_summary = build_runtime_summary(runtime_stats, video_health.as_deref());
    let primary_issue_chain = build_primary_issue_chain(
        runtime_stats,
        video_health.as_deref(),
        stall_kind.as_deref(),
    );
    let latest_decision_summary =
        build_latest_decision_summary(snapshot, runtime_stats, video_health.as_deref(), now_ms);

    XbxEngineStatsDto {
        resolution,
        rtt,
        fps,
        runtime_summary,
        primary_issue_chain,
        latest_decision_summary,
        session_phase: runtime_stats.and_then(|stats| stats.session_phase.clone()),
        transport_policy_profile: runtime_stats
            .and_then(|stats| stats.transport_policy_profile.clone()),
        recovery_policy_profile: runtime_stats
            .and_then(|stats| stats.recovery_policy_profile.clone()),
        recovery_diagnosis: runtime_stats.and_then(|stats| stats.recovery_diagnosis.clone()),
        recovery_coupling_mode: runtime_stats
            .and_then(|stats| stats.recovery_coupling_mode.clone()),
        recovery_coupling_summary: runtime_stats
            .and_then(|stats| stats.recovery_coupling_summary.clone()),
        direct_gaming_bitrate_band: runtime_stats
            .and_then(|stats| stats.direct_gaming_bitrate_band.clone()),
        video_health,
        stall_kind,
        inbound_video_fps: runtime_stats.map(|stats| stats.inbound_video_frame_rate_fps),
        decode_fps: runtime_stats.map(|stats| stats.video_decode_fps),
        present_fps: runtime_stats.map(|stats| stats.video_present_fps.max(fps)),
        pl: packet_loss,
        fl: String::new(),
        jit: jitter,
        br: bitrate,
        decode: String::new(),
        transport_path: runtime_stats.and_then(|stats| stats.transport_path.clone()),
        transport_state,
        video_rtt_source: runtime_stats.and_then(|stats| stats.video_rtt_source.clone()),
        video_remb_bps: runtime_stats.and_then(|stats| stats.video_remb_bps),
        inbound_bitrate_kbps: runtime_stats.and_then(|stats| stats.inbound_bitrate_kbps),
        inbound_video_bitrate_kbps: runtime_stats
            .and_then(|stats| stats.inbound_video_bitrate_kbps),
        inbound_audio_bitrate_kbps: runtime_stats
            .and_then(|stats| stats.inbound_audio_bitrate_kbps),
        inbound_bytes_total: runtime_stats.map(|stats| stats.inbound_bytes_total),
        inbound_video_bytes_total: runtime_stats.map(|stats| stats.inbound_video_bytes_total),
        inbound_audio_bytes_total: runtime_stats.map(|stats| stats.inbound_audio_bytes_total),
        inbound_video_packet_count_total: runtime_stats
            .map(|stats| stats.inbound_video_packet_count_total),
        latest_video_track_status: runtime_stats.and_then(|stats| {
            stats.latest_video_track_status.as_ref().map(|status| {
                xbxengine_protocol::XbxEngineVideoTrackStatusDto {
                    state: status.state.clone(),
                    video_width: status.video_width,
                    video_height: status.video_height,
                    mime_type: status.mime_type.clone(),
                    transport_state: status.transport_state.clone(),
                    video_bytes_total: status.video_bytes_total,
                    video_packet_count_total: status.video_packet_count_total,
                    audio_bytes_total: status.audio_bytes_total,
                    observed_at_ms: status.observed_at_ms,
                }
            })
        }),
        video_decoder_reset_count: runtime_stats.map(|stats| stats.video_decoder_reset_count),
        video_decoder_stalled: runtime_stats.and_then(|stats| stats.video_decoder_stalled),
        video_decoder_hardware_failure_streak: runtime_stats
            .map(|stats| stats.video_decoder_hardware_failure_streak),
        latest_video_decoder_hardware_failure_time_ms: runtime_stats
            .and_then(|stats| stats.latest_video_decoder_hardware_failure_time_ms),
        latest_video_decoder_hardware_failure_status: runtime_stats
            .and_then(|stats| stats.latest_video_decoder_hardware_failure_status),
        video_renderer_stalled: runtime_stats.and_then(|stats| stats.video_renderer_stalled),
        packet_age_ms,
        decode_age_ms,
        present_age_ms,
        packet_to_decode_ms,
        decode_to_present_ms,
        packet_to_present_ms,
        video_decode_input_drop_count_total: runtime_stats
            .map(|stats| stats.video_decode_input_drop_count_total),
        video_decode_output_drop_count_total: runtime_stats
            .map(|stats| stats.video_decode_output_drop_count_total),
        video_pacer_submit_count_total: runtime_stats
            .map(|stats| stats.video_pacer_submit_count_total),
        video_pacer_drop_count_total: runtime_stats.map(|stats| stats.video_pacer_drop_count_total),
        video_renderer_submit_count_total: runtime_stats
            .map(|stats| stats.video_renderer_submit_count_total),
        video_renderer_drop_count_total: runtime_stats
            .map(|stats| stats.video_renderer_drop_count_total),
        video_present_drop_count_total: None,
        video_present_overwrite_count_total: runtime_stats
            .map(|stats| stats.video_present_overwrite_count_total),
        video_present_submit_count_total: runtime_stats
            .map(|stats| stats.video_present_submit_count_total),
        video_present_descriptor_upload_mode: None,
        video_present_descriptor_metal_import_count_total: None,
        video_present_descriptor_cpu_upload_count_total: None,
        recovery_keyframe_request_count: Some(snapshot.recovery_keyframe_request_count),
        recovery_decoder_reset_count: Some(snapshot.recovery_decoder_reset_count),
        recovery_reconnect_count: Some(snapshot.recovery_reconnect_count),
        last_recovery_action: snapshot.last_recovery_action.clone(),
        last_recovery_action_at_ms: snapshot.last_recovery_action_at_ms,
        last_recovery_reason: snapshot.last_recovery_reason.clone(),
        latest_video_packet_gap: runtime_stats.and_then(|stats| {
            stats.latest_video_packet_gap.as_ref().map(|gap| {
                xbxengine_protocol::XbxEnginePacketGapObservationDto {
                    observation_id: gap.observation_id,
                    expected_sequence: gap.expected_sequence,
                    received_sequence: gap.received_sequence,
                    missing_count: gap.missing_count,
                    source: gap.source.clone(),
                    frame_rtp_timestamp: gap.frame_rtp_timestamp,
                    frame_packet_count: gap.frame_packet_count,
                    frame_missing_count: gap.frame_missing_count,
                    frame_is_keyframe: gap.frame_is_keyframe,
                    frame_importance: gap.frame_importance.clone(),
                    observed_at_ms: gap.observed_at_ms,
                }
            })
        }),
        latest_video_frame_drop: runtime_stats.and_then(|stats| {
            stats.latest_video_frame_drop.as_ref().map(|drop| {
                xbxengine_protocol::XbxEngineFrameDropObservationDto {
                    observation_id: drop.observation_id,
                    reason: drop.reason.clone(),
                    observed_at_ms: drop.observed_at_ms,
                    width: drop.width,
                    height: drop.height,
                    is_keyframe: drop.is_keyframe,
                    queue_depth: drop.queue_depth,
                }
            })
        }),
        latest_video_nack_observation: runtime_stats.and_then(|stats| {
            stats.latest_video_nack_observation.as_ref().map(|nack| {
                xbxengine_protocol::XbxEngineNackObservationDto {
                    observation_id: nack.observation_id,
                    action: nack.action.clone(),
                    source: nack.source.clone(),
                    first_sequence: nack.first_sequence,
                    last_sequence: nack.last_sequence,
                    packet_count: nack.packet_count,
                    retry_count: nack.retry_count,
                    frame_rtp_timestamp: nack.frame_rtp_timestamp,
                    frame_is_keyframe: nack.frame_is_keyframe,
                    frame_importance: nack.frame_importance.clone(),
                    deadline_at_ms: nack.deadline_at_ms,
                    observed_at_ms: nack.observed_at_ms,
                }
            })
        }),
        latest_video_escalation_observation: runtime_stats.and_then(|stats| {
            stats
                .latest_video_escalation_observation
                .as_ref()
                .map(
                    |escalation| xbxengine_protocol::XbxEngineVideoEscalationObservationDto {
                        observation_id: escalation.observation_id,
                        reason: escalation.reason.clone(),
                        action: escalation.action.clone(),
                        observed_at_ms: escalation.observed_at_ms,
                    },
                )
        }),
        latest_video_bwe_observation: runtime_stats.and_then(|stats| {
            stats.latest_video_bwe_observation.as_ref().map(|bwe| {
                xbxengine_protocol::XbxEngineVideoBweObservationDto {
                    observation_id: bwe.observation_id,
                    mode: bwe.mode.clone(),
                    decision_reason: bwe.decision_reason.clone(),
                    target_remb_kbps: bwe.target_remb_kbps,
                    observed_remb_kbps: bwe.observed_remb_kbps,
                    actual_video_bitrate_kbps: bwe.actual_video_bitrate_kbps,
                    loss_ratio: bwe.loss_ratio,
                    rtt_ms: bwe.rtt_ms,
                    transport_path: bwe.transport_path.clone(),
                    twcc_feedback_interval_ms: bwe.twcc_feedback_interval_ms,
                    twcc_observed_packet_count: bwe.twcc_observed_packet_count,
                    twcc_covered_sequence_span: bwe.twcc_covered_sequence_span,
                    twcc_receive_bitrate_kbps: bwe.twcc_receive_bitrate_kbps,
                    twcc_delivery_ratio: bwe.twcc_delivery_ratio,
                    twcc_loss_ratio: bwe.twcc_loss_ratio,
                    observed_at_ms: bwe.observed_at_ms,
                }
            })
        }),
        latest_video_twcc_observation: runtime_stats.and_then(|stats| {
            stats.latest_video_twcc_observation.as_ref().map(|twcc| {
                xbxengine_protocol::XbxEngineVideoTwccObservationDto {
                    observation_id: twcc.observation_id,
                    feedback_packet_count: twcc.feedback_packet_count,
                    covered_sequence_start: twcc.covered_sequence_start,
                    covered_sequence_end: twcc.covered_sequence_end,
                    covered_sequence_span: twcc.covered_sequence_span,
                    observed_packet_count: twcc.observed_packet_count,
                    observed_byte_count: twcc.observed_byte_count,
                    feedback_interval_ms: twcc.feedback_interval_ms,
                    arrival_span_ms: twcc.arrival_span_ms,
                    receive_bitrate_kbps: twcc.receive_bitrate_kbps,
                    delivery_ratio: twcc.delivery_ratio,
                    packet_loss_ratio: twcc.packet_loss_ratio,
                    observed_at_ms: twcc.observed_at_ms,
                }
            })
        }),
        latest_data_channel_message_catalog_observation: runtime_stats.and_then(|stats| {
            stats
                .latest_data_channel_message_catalog_observation
                .as_ref()
                .map(|observation| {
                    xbxengine_protocol::XbxEngineDataChannelMessageCatalogObservationDto {
                        observation_id: observation.observation_id,
                        direction: observation.direction.clone(),
                        channel: observation.channel.clone(),
                        kind_type: observation.kind_type.clone(),
                        kind_message: observation.kind_message.clone(),
                        target: observation.target.clone(),
                        keys: observation.keys.clone(),
                        payload_len: observation.payload_len,
                        observed_at_ms: observation.observed_at_ms,
                    }
                })
        }),
    }
}

// 用统一摘要描述当前 runtime 所处状态，便于回归时快速判断是否落在预期档位。
fn build_runtime_summary(
    runtime_stats: Option<&XbxEngineMediaRuntimeStats>,
    video_health: Option<&str>,
) -> Option<String> {
    let stats = runtime_stats?;
    let profile = stats
        .transport_policy_profile
        .as_deref()
        .unwrap_or("unknown");
    let phase = stats.session_phase.as_deref().unwrap_or("unknown");
    let band = stats
        .direct_gaming_bitrate_band
        .as_deref()
        .unwrap_or("unknown");
    let health = video_health.unwrap_or("unknown");
    Some(format!("{profile}/{phase}/{band}/{health}"))
}

// 将当前主问题链显式归类，避免每次回归都手工拼 diagnosis/band/health。
fn build_primary_issue_chain(
    runtime_stats: Option<&XbxEngineMediaRuntimeStats>,
    video_health: Option<&str>,
    stall_kind: Option<&str>,
) -> Option<String> {
    let stats = runtime_stats?;
    match video_health {
        Some("connecting") => return Some("transport:connecting".to_string()),
        Some("startupLowQuality") => return Some("startup:lowQuality".to_string()),
        Some("stalled") => {
            if let Some(stall) = stall_kind.filter(|stall| *stall != "none") {
                return Some(format!("stall:{stall}"));
            }
        }
        Some("recovering") => {
            if let Some(diagnosis) = stats.recovery_diagnosis.as_deref() {
                return Some(format!("recovery:{diagnosis}"));
            }
        }
        _ => {}
    }
    if let Some(diagnosis) = stats.recovery_diagnosis.as_deref() {
        return Some(format!("recovery:{diagnosis}"));
    }
    if let Some(stall) = stall_kind.filter(|stall| *stall != "none") {
        return Some(format!("stall:{stall}"));
    }
    if stats.session_phase.as_deref() == Some("startup")
        && stats.direct_gaming_bitrate_band.as_deref() == Some("startupLow")
    {
        return Some("startup:lowQuality".to_string());
    }
    Some("steady:healthy".to_string())
}

// 把最近一次真正影响行为的决策压成摘要，便于对照 trace 和面板。
fn build_latest_decision_summary(
    snapshot: &XbxEngineRuntimeSnapshot,
    runtime_stats: Option<&XbxEngineMediaRuntimeStats>,
    video_health: Option<&str>,
    now_ms: f64,
) -> Option<String> {
    let stats = runtime_stats?;
    const DECISION_FRESH_WINDOW_MS: f64 = 1_500.0;

    if let Some(escalation) = stats.latest_video_escalation_observation.as_ref() {
        if now_ms - escalation.observed_at_ms <= DECISION_FRESH_WINDOW_MS
            || matches!(
                video_health,
                Some("stalled" | "recovering" | "startupLowQuality")
            )
        {
            return Some(format!(
                "recovery:{}->{}",
                escalation.reason, escalation.action
            ));
        }
    }
    if let Some(bwe) = stats.latest_video_bwe_observation.as_ref() {
        return Some(format!(
            "bwe:{}:{}kbps",
            bwe.decision_reason, bwe.target_remb_kbps
        ));
    }
    if let (Some(action), Some(reason)) = (
        snapshot.last_recovery_action.as_deref(),
        snapshot.last_recovery_reason.as_deref(),
    ) {
        if snapshot
            .last_recovery_action_at_ms
            .is_some_and(|at_ms| now_ms - at_ms <= DECISION_FRESH_WINDOW_MS)
        {
            return Some(format!("recovery:{reason}->{action}"));
        }
    }
    None
}

/// 统一把运行时事实压成 UI/trace 可直接消费的健康态，避免前端再拼条件猜状态。
fn classify_video_health(runtime_stats: Option<&XbxEngineMediaRuntimeStats>) -> Option<String> {
    let stats = runtime_stats?;
    let transport_state = format!("{:?}", stats.transport_state);
    if transport_state != "Connected" {
        return Some("connecting".to_string());
    }
    match stats.recovery_diagnosis.as_deref() {
        Some("ingressWaitKeyframe") | Some("transportAwaitRecoveryKeyframe") => {
            return Some("waitingKeyframe".to_string());
        }
        Some("transportSampleLoss") => {
            return Some("referenceDirty".to_string());
        }
        Some("adapterIdleTimeout" | "decoderBackendFailure") => {
            return Some("stalled".to_string());
        }
        _ => {}
    }
    if stats.video_decoder_stalled == Some(true) || stats.video_renderer_stalled == Some(true) {
        return Some("stalled".to_string());
    }
    if stats.session_phase.as_deref() == Some("startup")
        && stats.direct_gaming_bitrate_band.as_deref() == Some("startupLow")
    {
        return Some("startupLowQuality".to_string());
    }
    if stats.session_phase.as_deref() == Some("recovering") {
        return Some("recovering".to_string());
    }
    Some("healthy".to_string())
}

/// stall kind 用于界面/离线分析统一解释“这次卡住属于哪条链”。
fn classify_stall_kind(runtime_stats: Option<&XbxEngineMediaRuntimeStats>) -> Option<String> {
    let stats = runtime_stats?;
    match stats.recovery_diagnosis.as_deref() {
        Some("adapterIdleTimeout") => Some("idleTimeout".to_string()),
        Some("decoderBackendFailure") => Some("decoderBackendFailure".to_string()),
        Some("transportSampleLoss") => Some("sampleLoss".to_string()),
        Some("transportAwaitRecoveryKeyframe") | Some("ingressWaitKeyframe") => {
            Some("waitingKeyframe".to_string())
        }
        Some("reconfigure") => Some("reconfigure".to_string()),
        _ => {
            if stats.video_decoder_stalled == Some(true)
                || stats.video_renderer_stalled == Some(true)
            {
                Some("pipelineStall".to_string())
            } else if stats.direct_gaming_bitrate_band.as_deref() == Some("paused")
                && stats.inbound_video_bitrate_kbps.unwrap_or(0.0) <= 0.1
                && stats.video_present_fps <= 1.0
            {
                Some("videoPaused".to_string())
            } else if stats.session_phase.as_deref() == Some("startup")
                && stats.direct_gaming_bitrate_band.as_deref() == Some("startupLow")
            {
                Some("startupLowQuality".to_string())
            } else {
                Some("none".to_string())
            }
        }
    }
}
