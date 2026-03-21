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
    let observation_note = build_observation_note(runtime_stats);
    let transport_recovery_note = build_transport_recovery_note(runtime_stats);
    let repair_probe_note = build_repair_probe_note(runtime_stats);
    let reinject_note = build_rtx_reinject_note(runtime_stats);
    let runtime_summary = build_runtime_summary(
        runtime_stats,
        video_health.as_deref(),
        observation_note.as_deref(),
        transport_recovery_note.as_deref(),
        repair_probe_note.as_deref(),
        reinject_note.as_deref(),
    );
    let primary_issue_chain = build_primary_issue_chain(
        runtime_stats,
        video_health.as_deref(),
        stall_kind.as_deref(),
    );
    let latest_decision_summary = build_latest_decision_summary(
        snapshot,
        runtime_stats,
        video_health.as_deref(),
        observation_note.as_deref(),
        transport_recovery_note.as_deref(),
        repair_probe_note.as_deref(),
        reinject_note.as_deref(),
        now_ms,
    );

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
        inbound_bitrate_kbps: runtime_stats.and_then(|stats| {
            stats
                .inbound_bitrate_kbps
                .or_else(|| estimate_total_inbound_bitrate_kbps(stats, now_ms))
        }),
        inbound_video_bitrate_kbps: runtime_stats
            .and_then(|stats| stats.inbound_video_bitrate_kbps),
        inbound_audio_bitrate_kbps: runtime_stats
            .and_then(|stats| {
                stats
                    .inbound_audio_bitrate_kbps
                    .or_else(|| estimate_audio_inbound_bitrate_kbps(stats, now_ms))
            }),
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
    observation_note: Option<&str>,
    transport_recovery_note: Option<&str>,
    repair_probe_note: Option<&str>,
    reinject_note: Option<&str>,
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
    let base = format!("{profile}/{phase}/{band}/{health}");
    Some(append_runtime_notes(
        base,
        observation_note,
        transport_recovery_note,
        repair_probe_note,
        reinject_note,
    ))
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
    observation_note: Option<&str>,
    transport_recovery_note: Option<&str>,
    repair_probe_note: Option<&str>,
    reinject_note: Option<&str>,
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
            return Some(append_runtime_notes(
                format!("recovery:{}->{}", escalation.reason, escalation.action),
                observation_note,
                transport_recovery_note,
                repair_probe_note,
                reinject_note,
            ));
        }
    }
    if let Some(bwe) = stats.latest_video_bwe_observation.as_ref() {
        return Some(append_runtime_notes(
            format!("bwe:{}:{}kbps", bwe.decision_reason, bwe.target_remb_kbps),
            observation_note,
            transport_recovery_note,
            repair_probe_note,
            reinject_note,
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
            return Some(append_runtime_notes(
                format!("recovery:{reason}->{action}"),
                observation_note,
                transport_recovery_note,
                repair_probe_note,
                reinject_note,
            ));
        }
    }
    match (
        observation_note,
        transport_recovery_note,
        repair_probe_note,
        reinject_note,
    ) {
        (Some(note), Some(epoch), Some(repair), Some(reinject)) => {
            Some(format!("obs:{note} | {epoch} | {repair} | {reinject}"))
        }
        (Some(note), Some(epoch), Some(repair), None) => {
            Some(format!("obs:{note} | {epoch} | {repair}"))
        }
        (Some(note), Some(epoch), None, Some(reinject)) => {
            Some(format!("obs:{note} | {epoch} | {reinject}"))
        }
        (Some(note), Some(epoch), None, None) => Some(format!("obs:{note} | {epoch}")),
        (Some(note), None, Some(repair), Some(reinject)) => {
            Some(format!("obs:{note} | {repair} | {reinject}"))
        }
        (Some(note), None, Some(repair), None) => Some(format!("obs:{note} | {repair}")),
        (Some(note), None, None, Some(reinject)) => Some(format!("obs:{note} | {reinject}")),
        (Some(note), None, None, None) => Some(format!("obs:{note}")),
        (None, Some(epoch), Some(repair), Some(reinject)) => {
            Some(format!("{epoch} | {repair} | {reinject}"))
        }
        (None, Some(epoch), Some(repair), None) => Some(format!("{epoch} | {repair}")),
        (None, Some(epoch), None, Some(reinject)) => Some(format!("{epoch} | {reinject}")),
        (None, Some(epoch), None, None) => Some(epoch.to_string()),
        (None, None, Some(repair), Some(reinject)) => Some(format!("{repair} | {reinject}")),
        (None, None, Some(repair), None) => Some(repair.to_string()),
        (None, None, None, Some(reinject)) => Some(reinject.to_string()),
        (None, None, None, None) => None,
    }
}

fn build_observation_note(runtime_stats: Option<&XbxEngineMediaRuntimeStats>) -> Option<String> {
    let stats = runtime_stats?;
    if format!("{:?}", stats.transport_state) == "Closed" {
        return None;
    }
    let label = stats.latest_observation_label.as_deref()?;
    let summary = stats.latest_observation_summary.as_deref()?;
    Some(format!("{label}:{summary}"))
}

fn build_transport_recovery_note(
    runtime_stats: Option<&XbxEngineMediaRuntimeStats>,
) -> Option<String> {
    let stats = runtime_stats?;
    if stats.transport_recovery_epoch == 0 {
        return None;
    }
    if stats.transport_recovery_epoch > stats.transport_recovery_epoch_at_last_escalation {
        Some(format!("repoch:{}:active", stats.transport_recovery_epoch))
    } else {
        Some(format!("repoch:{}", stats.transport_recovery_epoch))
    }
}

fn append_runtime_notes(
    base: String,
    observation_note: Option<&str>,
    transport_recovery_note: Option<&str>,
    repair_probe_note: Option<&str>,
    reinject_note: Option<&str>,
) -> String {
    let mut result = base;
    if let Some(note) = observation_note {
        result.push_str(" | obs:");
        result.push_str(note);
    }
    if let Some(note) = transport_recovery_note {
        result.push_str(" | ");
        result.push_str(note);
    }
    if let Some(note) = repair_probe_note {
        result.push_str(" | ");
        result.push_str(note);
    }
    if let Some(note) = reinject_note {
        result.push_str(" | ");
        result.push_str(note);
    }
    result
}

fn build_repair_probe_note(runtime_stats: Option<&XbxEngineMediaRuntimeStats>) -> Option<String> {
    let stats = runtime_stats?;
    let observation = stats.latest_video_repair_probe_observation.as_ref()?;
    let active_since_ms = stats.video_repair_probe_active_since_ms?;
    let recovery_hit_rate = stats
        .video_repair_probe_recovery_hit_rate_since_active
        .map(|value| format!("{value:.3}"))
        .unwrap_or_else(|| "-".to_string());
    Some(format!(
        "repair:{}:{}:{} id={} ssrc={} pt={} clock={} pkts={} since={active_since_ms:.0} rec={} late={} exp={} gaps={} hit={}",
        observation.classification,
        observation.mime_type,
        observation.phase,
        observation.stream_id,
        observation.stream_ssrc,
        observation.payload_type,
        observation.clock_rate,
        stats.video_repair_probe_packet_count_total,
        stats.video_repair_probe_recovered_count_since_active,
        stats.video_repair_probe_late_recovered_count_since_active,
        stats.video_repair_probe_expired_count_since_active,
        stats.video_repair_probe_packet_gap_count_since_active,
        recovery_hit_rate
    ))
}

fn build_rtx_reinject_note(runtime_stats: Option<&XbxEngineMediaRuntimeStats>) -> Option<String> {
    let stats = runtime_stats?;
    let observation = stats.latest_video_rtx_reinject_observation.as_ref()?;
    let total = stats.video_rtx_reinject_head_match_count_total
        + stats.video_rtx_reinject_range_match_count_total
        + stats.video_rtx_reinject_miss_count_total;
    let hit_rate = if total > 0 {
        format!(
            "{:.3}",
            stats.video_rtx_reinject_head_match_count_total as f64 / total as f64
        )
    } else {
        "-".to_string()
    };
    Some(format!(
        "reinject:stage={} seq={} native={:?} pending={} headMatch={} rangeMatch={} gap={:?} nack={:?}..{:?} headHits={} rangeHits={} miss={} headHitRate={}",
        observation.stage,
        observation.sequence_number,
        observation.native_sequence_number,
        observation.pending_queue_len,
        observation.matched_head_gap,
        observation.matched_nack_range,
        observation.matched_gap_sequence,
        observation.matched_nack_first_sequence,
        observation.matched_nack_last_sequence,
        stats.video_rtx_reinject_head_match_count_total,
        stats.video_rtx_reinject_range_match_count_total,
        stats.video_rtx_reinject_miss_count_total,
        hit_rate
    ))
}

/// 统一把运行时事实压成 UI/trace 可直接消费的健康态，避免前端再拼条件猜状态。
fn classify_video_health(runtime_stats: Option<&XbxEngineMediaRuntimeStats>) -> Option<String> {
    let stats = runtime_stats?;
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0);
    let fresh_output = has_recent_video_output(stats, now_ms);
    let transport_state = format!("{:?}", stats.transport_state);
    if transport_state != "Connected" {
        return Some("connecting".to_string());
    }
    match stats.recovery_diagnosis.as_deref() {
        Some("ingressWaitKeyframe") | Some("transportAwaitRecoveryKeyframe")
            if !fresh_output =>
        {
            return Some("waitingKeyframe".to_string());
        }
        Some("transportSampleLoss") if !fresh_output => {
            return Some("referenceDirty".to_string());
        }
        Some("adapterIdleTimeout" | "decoderBackendFailure") if !fresh_output => {
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
    if stats.session_phase.as_deref() == Some("recovering") && !fresh_output {
        return Some("recovering".to_string());
    }
    Some("healthy".to_string())
}

/// stall kind 用于界面/离线分析统一解释“这次卡住属于哪条链”。
fn classify_stall_kind(runtime_stats: Option<&XbxEngineMediaRuntimeStats>) -> Option<String> {
    let stats = runtime_stats?;
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0);
    let fresh_output = has_recent_video_output(stats, now_ms);
    match stats.recovery_diagnosis.as_deref() {
        Some("adapterIdleTimeout") if !fresh_output => Some("idleTimeout".to_string()),
        Some("decoderBackendFailure") if !fresh_output => {
            Some("decoderBackendFailure".to_string())
        }
        Some("transportSampleLoss") if !fresh_output => Some("sampleLoss".to_string()),
        Some("transportAwaitRecoveryKeyframe") | Some("ingressWaitKeyframe")
            if !fresh_output =>
        {
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
            } else if stats.session_phase.as_deref() == Some("recovering") && !fresh_output {
                Some("recovering".to_string())
            } else {
                Some("none".to_string())
            }
        }
    }
}

fn has_recent_video_output(stats: &XbxEngineMediaRuntimeStats, now_ms: f64) -> bool {
    const RECENT_VIDEO_OUTPUT_WINDOW_MS: f64 = 500.0;
    let present_fresh = stats
        .latest_video_present_time_ms
        .map(|at_ms| now_ms - at_ms < RECENT_VIDEO_OUTPUT_WINDOW_MS)
        .unwrap_or(false);
    let decode_fresh = stats
        .latest_video_decode_ok_time_ms
        .map(|at_ms| now_ms - at_ms < RECENT_VIDEO_OUTPUT_WINDOW_MS)
        .unwrap_or(false);
    present_fresh || decode_fresh || stats.video_present_fps >= 10.0
}

fn estimate_audio_inbound_bitrate_kbps(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> Option<f64> {
    let first_audio_packet_at_ms = stats.first_audio_packet_arrival_time_ms?;
    let elapsed_ms = (now_ms - first_audio_packet_at_ms).max(0.0);
    if elapsed_ms <= 0.0 {
        return None;
    }
    let bytes_total = stats.inbound_audio_bytes_total;
    if bytes_total == 0 {
        return None;
    }
    Some((bytes_total as f64 * 8.0 / elapsed_ms).max(0.0))
}

fn estimate_total_inbound_bitrate_kbps(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> Option<f64> {
    let video_kbps = stats.inbound_video_bitrate_kbps.unwrap_or(0.0);
    let audio_kbps = stats
        .inbound_audio_bitrate_kbps
        .or_else(|| estimate_audio_inbound_bitrate_kbps(stats, now_ms))
        .unwrap_or(0.0);
    let total = video_kbps + audio_kbps;
    if total > 0.0 { Some(total) } else { None }
}

#[cfg(test)]
mod tests {
    use xbxengine_protocol::XbxEngineTransportStateDto;

    use super::*;
    use crate::api::runtime::XbxEngineRuntimeSnapshot;

    fn test_snapshot() -> XbxEngineRuntimeSnapshot {
        XbxEngineRuntimeSnapshot {
            audio_volume: 1.0,
            keyboard_pointer_enabled: false,
            microphone_capturing: false,
            microphone_paused: false,
            display_state: None,
            viewport: None,
            surface_id: None,
            video_size: None,
            last_keyboard_pointer_event: None,
            last_pressed_controller_button: None,
            negotiation_attempt_count: 0,
            last_offer_sdp: None,
            last_answer_sdp: None,
            last_remote_candidates: Vec::new(),
            input_device_count: 0,
            input_pad_count: 0,
            input_route_attached: false,
            first_frame_packet_arrival_time_ms: None,
            frame_decoded_time_ms: None,
            frame_rendered_time_ms: None,
            latest_video_track_status: None,
            recovery_keyframe_request_count: 0,
            recovery_decoder_reset_count: 0,
            recovery_reconnect_count: 0,
            last_recovery_action: None,
            last_recovery_action_at_ms: None,
            last_recovery_reason: None,
        }
    }

    #[test]
    fn runtime_summary_includes_transport_recovery_epoch_note() {
        let stats = XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            transport_policy_profile: Some("cloud".to_string()),
            session_phase: Some("recovering".to_string()),
            direct_gaming_bitrate_band: Some("steady".to_string()),
            transport_recovery_epoch: 7,
            transport_recovery_epoch_at_last_escalation: 6,
            ..XbxEngineMediaRuntimeStats::default()
        };

        let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
        let runtime_summary = dto.runtime_summary.expect("runtime summary");

        assert!(runtime_summary.contains("repoch:7:active"));
    }

    #[test]
    fn latest_decision_summary_falls_back_to_transport_recovery_epoch_note() {
        let stats = XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            transport_recovery_epoch: 3,
            transport_recovery_epoch_at_last_escalation: 3,
            ..XbxEngineMediaRuntimeStats::default()
        };

        let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
        let latest_decision_summary = dto
            .latest_decision_summary
            .expect("latest decision summary");

        assert_eq!(latest_decision_summary, "repoch:3");
    }

    #[test]
    fn runtime_summary_includes_repair_probe_note_when_active() {
        let stats = XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            transport_policy_profile: Some("cloud".to_string()),
            session_phase: Some("steady".to_string()),
            direct_gaming_bitrate_band: Some("steady".to_string()),
            latest_video_repair_probe_observation: Some(
                crate::XbxEngineVideoRepairProbeObservation {
                    observation_id: 1,
                    phase: "packet".to_string(),
                    classification: "repair-mime".to_string(),
                    stream_id: "rtx-1".to_string(),
                    stream_ssrc: 11,
                    mime_type: "video/rtx".to_string(),
                    payload_type: 97,
                    clock_rate: 90_000,
                    associated_ssrc: Some(42),
                    associated_payload_type: Some(124),
                    stream_packet_count: 8,
                    observed_at_ms: 2_000.0,
                },
            ),
            video_repair_probe_active_since_ms: Some(1_000.0),
            video_repair_probe_packet_count_total: 8,
            video_repair_probe_recovered_count_since_active: 3,
            video_repair_probe_late_recovered_count_since_active: 1,
            video_repair_probe_expired_count_since_active: 0,
            video_repair_probe_packet_gap_count_since_active: 2,
            ..XbxEngineMediaRuntimeStats::default()
        };

        let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
        let runtime_summary = dto.runtime_summary.expect("runtime summary");

        assert!(runtime_summary.contains("repair:repair-mime:video/rtx:packet"));
        assert!(runtime_summary.contains("id=rtx-1"));
        assert!(runtime_summary.contains("rec=3"));
        assert!(runtime_summary.contains("exp=0"));
    }

    #[test]
    fn runtime_summary_includes_rtx_reinject_note_when_present() {
        let stats = XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            latest_video_rtx_reinject_observation: Some(
                crate::XbxEngineVideoRtxReinjectObservation {
                    stage: "adapterResolved".to_string(),
                    primary_ssrc: 10,
                    repair_ssrc: 20,
                    sequence_number: 18_894,
                    rtp_timestamp: 123,
                    pending_queue_len: 0,
                    native_sequence_number: None,
                    matched_head_gap: true,
                    matched_nack_range: true,
                    matched_pending_gap: true,
                    matched_gap_sequence: Some(18_894),
                    matched_nack_first_sequence: Some(18_894),
                    matched_nack_last_sequence: Some(18_894),
                    observed_at_ms: 1_000.0,
                },
            ),
            video_rtx_reinject_head_match_count_total: 2,
            video_rtx_reinject_range_match_count_total: 1,
            video_rtx_reinject_miss_count_total: 1,
            ..XbxEngineMediaRuntimeStats::default()
        };

        let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
        let runtime_summary = dto.runtime_summary.expect("runtime summary");

        assert!(runtime_summary.contains("reinject:stage=adapterResolved seq=18894"));
        assert!(runtime_summary.contains("headMatch=true"));
        assert!(runtime_summary.contains("rangeMatch=true"));
        assert!(runtime_summary.contains("headHitRate=0.500"));
    }

    #[test]
    fn classify_video_health_ignores_stale_adapter_idle_timeout_when_output_is_fresh() {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as f64;
        let stats = XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            recovery_diagnosis: Some("adapterIdleTimeout".to_string()),
            latest_video_present_time_ms: Some(now_ms - 40.0),
            latest_video_decode_ok_time_ms: Some(now_ms - 40.0),
            video_present_fps: 58.0,
            ..XbxEngineMediaRuntimeStats::default()
        };

        assert_eq!(classify_video_health(Some(&stats)), Some("healthy".to_string()));
        assert_eq!(classify_stall_kind(Some(&stats)), Some("none".to_string()));
    }

    #[test]
    fn audio_inbound_bitrate_is_estimated_from_audio_bytes_when_playback_is_absent() {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as f64;
        let stats = XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            first_audio_packet_arrival_time_ms: Some(now_ms - 2_000.0),
            latest_audio_packet_arrival_time_ms: Some(now_ms - 120.0),
            inbound_audio_bytes_total: 250_000,
            inbound_video_bitrate_kbps: Some(16_000.0),
            ..XbxEngineMediaRuntimeStats::default()
        };

        let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
        assert!(dto.inbound_audio_bitrate_kbps.unwrap_or(0.0) > 0.0);
        assert!(dto.inbound_bitrate_kbps.unwrap_or(0.0) >= dto.inbound_video_bitrate_kbps.unwrap_or(0.0));
    }
}
