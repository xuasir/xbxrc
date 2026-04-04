use xbxengine_protocol::XbxEngineStatsDto;

use crate::transport::rtc::recovery::remote_profile_runtime::classify_runtime_remote_profile;
use crate::transport::rtc::recovery::runtime_state::{
    project_runtime_state_from_stats, resolve_runtime_recovery_profile, RecoveryRuntimeState,
};
use crate::transport::rtc::recovery::startup::SessionPhase;
use crate::{XbxEngineMediaRuntimeStats, XbxEngineRuntimeSnapshot};

#[derive(Clone, Debug)]
struct VideoOwnerContract {
    state: String,
    reason: Option<String>,
    source: Option<String>,
    observed_at_ms: Option<f64>,
}

#[derive(Clone, Debug, Default)]
struct RuntimeRemoteProfileStrings {
    baseline: Option<String>,
    dynamic: Option<String>,
    effective_label: Option<String>,
}

fn runtime_state_owner_source(phase: SessionPhase) -> &'static str {
    match phase {
        SessionPhase::Startup => "runtime-startup",
        SessionPhase::Steady => "runtime-steady",
        SessionPhase::Recovering => "runtime-recovering",
    }
}

fn project_video_owner_contract(
    runtime_stats: Option<&XbxEngineMediaRuntimeStats>,
    runtime_state: Option<&RecoveryRuntimeState>,
) -> Option<VideoOwnerContract> {
    if let Some(stats) = runtime_stats {
        if let Some(state) = stats.video_owner_state.clone() {
            return Some(VideoOwnerContract {
                state,
                reason: stats.video_owner_reason.clone(),
                source: stats.video_owner_source.clone(),
                observed_at_ms: stats.video_owner_observed_at_ms,
            });
        }
        if !should_project_runtime_owner_fallback(stats) {
            return None;
        }
    }
    runtime_state.map(|state| VideoOwnerContract {
        state: state.primary_view.owner_state.clone(),
        reason: Some(state.primary_view.owner_reason.clone()),
        source: Some(runtime_state_owner_source(state.phase).to_string()),
        observed_at_ms: None,
    })
}

fn should_project_runtime_owner_fallback(stats: &XbxEngineMediaRuntimeStats) -> bool {
    stats.session_phase.as_deref() == Some("recovering")
        || (stats.message_handshake_acked_at_ms.is_some()
            && stats.control_ready_at_ms.is_some()
            && has_visible_video_output(stats))
}

fn map_owner_state_to_video_health(owner_state: &str) -> String {
    match owner_state {
        "seeking-anchor" | "priming" => "priming".to_string(),
        "stable-serving" | "degraded-serving" => "healthy".to_string(),
        "rebuilding-supply" => "recovering".to_string(),
        "supply-starved" => "displaySupplyStarved".to_string(),
        other => other.to_string(),
    }
}

fn frame_budget_dto_from_observation(
    budget: Option<&crate::XbxEngineFrameBudgetObservation>,
) -> Option<xbxengine_protocol::XbxEngineFrameBudgetDto> {
    budget.map(|budget| xbxengine_protocol::XbxEngineFrameBudgetDto {
        recovery_stage: budget.recovery_stage.clone(),
        chain_value: budget.chain_value.clone(),
        rtt_slack: budget.rtt_slack.clone(),
        failure_cost: budget.failure_cost.clone(),
        window_source: budget.window_source.clone(),
    })
}

fn h264_inspection_dto_from_observation(
    observation: Option<&crate::XbxEngineH264InspectionObservation>,
) -> Option<xbxengine_protocol::XbxEngineH264InspectionObservationDto> {
    observation.map(
        |observation| xbxengine_protocol::XbxEngineH264InspectionObservationDto {
            observation_id: observation.observation_id,
            frame_rtp_timestamp: observation.frame_rtp_timestamp,
            nal_types: observation.nal_types.clone(),
            has_inband_sps: observation.has_inband_sps,
            has_inband_pps: observation.has_inband_pps,
            committed_sps_present: observation.committed_sps_present,
            committed_pps_present: observation.committed_pps_present,
            slice_headers_valid: observation.slice_headers_valid,
            delta_continuation_ready: observation.delta_continuation_ready,
            parameter_sets_changed: observation.parameter_sets_changed,
            config_changed: observation.config_changed,
            is_idr: observation.is_idr,
            bootstrap_ready: observation.bootstrap_ready,
            bootstrap_reject_reason: observation.bootstrap_reject_reason.clone(),
            admission_accepted: observation.admission_accepted,
            observed_at_ms: observation.observed_at_ms,
        },
    )
}

fn normalize_budget_link_value(
    frame_importance: Option<&str>,
    is_keyframe: Option<bool>,
) -> String {
    if is_keyframe.unwrap_or(false) || matches!(frame_importance, Some("keyframe")) {
        return "anchor".to_string();
    }
    if matches!(frame_importance, Some("reference")) {
        return "supply".to_string();
    }
    "disposable".to_string()
}

fn normalize_budget_failure_cost(
    frame_recovery_disposition: Option<&str>,
    frame_unrecoverable_reason: Option<&str>,
    nack_disposition: Option<&str>,
) -> String {
    if matches!(
        frame_recovery_disposition,
        Some("unrecoverable-reference-chain")
    ) || frame_unrecoverable_reason.is_some_and(|reason| {
        matches!(
            reason,
            "referenceChainUnrecoverable" | "awaitingRecoveryKeyframe" | "chainBroken"
        )
    }) || matches!(nack_disposition, Some("skippedChainBroken"))
    {
        return "chain-broken".to_string();
    }
    if frame_unrecoverable_reason.is_some_and(|reason| {
        matches!(
            reason,
            "parameterSetsChanged" | "dimensionsChanged" | "codecChanged" | "configChanged"
        )
    }) {
        return "reconfigure".to_string();
    }
    if matches!(
        frame_unrecoverable_reason,
        Some("waitKeyframe" | "awaitingRecoveryKeyframe")
    ) {
        return "wait-keyframe".to_string();
    }
    "local-drop".to_string()
}

fn normalize_budget_recovery_stage(failure_cost: &str, window_source: &str) -> String {
    match (failure_cost, window_source) {
        ("chain-broken" | "wait-keyframe", _) => "awaiting-keyframe".to_string(),
        ("reconfigure", _) => "reconfiguring".to_string(),
        (_, "recovery") => "repairing".to_string(),
        _ => "steady".to_string(),
    }
}

fn normalize_budget_rtt_slack(
    estimated_recovery_arrival_ms: Option<f64>,
    deadline_at_ms: Option<f64>,
    failure_cost: &str,
) -> String {
    if let Some(slack_ms) = deadline_at_ms
        .zip(estimated_recovery_arrival_ms)
        .map(|(deadline, arrival)| deadline - arrival)
    {
        if slack_ms <= 0.0 {
            return "exhausted".to_string();
        }
        if slack_ms <= 12.0 {
            return "tight".to_string();
        }
        return "ample".to_string();
    }
    if failure_cost == "chain-broken" {
        return "tight".to_string();
    }
    "unknown".to_string()
}

fn infer_budget_from_drop(
    drop: &crate::XbxEngineVideoFrameDropObservation,
) -> xbxengine_protocol::XbxEngineFrameBudgetDto {
    let window_source = if drop.detail.as_deref().is_some_and(|detail| {
        matches!(
            detail,
            "queuePressure" | "queuePressureAggressive" | "outputQueueOverflow"
        )
    }) {
        "recovery".to_string()
    } else {
        "playout".to_string()
    };
    let failure_cost = normalize_budget_failure_cost(
        drop.frame_recovery_disposition.as_deref(),
        drop.frame_unrecoverable_reason.as_deref(),
        None,
    );
    xbxengine_protocol::XbxEngineFrameBudgetDto {
        recovery_stage: normalize_budget_recovery_stage(&failure_cost, &window_source),
        chain_value: normalize_budget_link_value(None, Some(drop.is_keyframe)),
        rtt_slack: normalize_budget_rtt_slack(None, None, &failure_cost),
        failure_cost,
        window_source,
    }
}

fn infer_budget_from_frame_recovery(
    observation: &crate::XbxEngineFrameRecoveryObservation,
) -> xbxengine_protocol::XbxEngineFrameBudgetDto {
    let window_source = if observation.frame_playout_deadline_at_ms.is_some() {
        "recovery".to_string()
    } else {
        "playout".to_string()
    };
    let failure_cost = normalize_budget_failure_cost(
        Some(observation.frame_recovery_disposition.as_str()),
        observation.frame_unrecoverable_reason.as_deref(),
        None,
    );
    let chain_value = if failure_cost == "chain-broken" {
        "supply".to_string()
    } else {
        "disposable".to_string()
    };
    xbxengine_protocol::XbxEngineFrameBudgetDto {
        recovery_stage: normalize_budget_recovery_stage(&failure_cost, &window_source),
        chain_value,
        rtt_slack: normalize_budget_rtt_slack(
            None,
            observation.frame_playout_deadline_at_ms,
            &failure_cost,
        ),
        failure_cost,
        window_source,
    }
}

fn infer_budget_from_nack(
    nack: &crate::XbxEngineVideoNackObservation,
) -> xbxengine_protocol::XbxEngineFrameBudgetDto {
    let window_source = if nack.source == "sampleLoss" {
        "recovery".to_string()
    } else {
        "transport".to_string()
    };
    let failure_cost = normalize_budget_failure_cost(
        None,
        nack.frame_unrecoverable_reason.as_deref(),
        nack.nack_disposition.as_deref(),
    );
    xbxengine_protocol::XbxEngineFrameBudgetDto {
        recovery_stage: normalize_budget_recovery_stage(&failure_cost, &window_source),
        chain_value: normalize_budget_link_value(
            nack.frame_importance.as_deref(),
            nack.frame_is_keyframe,
        ),
        rtt_slack: normalize_budget_rtt_slack(
            nack.estimated_recovery_arrival_ms,
            nack.deadline_at_ms,
            &failure_cost,
        ),
        failure_cost,
        window_source,
    }
}

fn resolve_recovery_strategy_profile(
    runtime_stats: Option<&XbxEngineMediaRuntimeStats>,
) -> Option<String> {
    runtime_stats.map(|stats| {
        resolve_runtime_recovery_profile(stats)
            .kind
            .as_str()
            .to_string()
    })
}

fn resolve_recovery_diagnosis(
    runtime_stats: Option<&XbxEngineMediaRuntimeStats>,
    runtime_state: Option<&RecoveryRuntimeState>,
    owner: Option<&VideoOwnerContract>,
) -> Option<String> {
    owner
        .and_then(|owner| owner.reason.clone())
        .or_else(|| runtime_state.map(|state| state.diagnosis_label.clone()))
        .or_else(|| runtime_stats.and_then(|stats| stats.recovery_diagnosis.clone()))
}

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
        .and_then(|stats| stats.latest_video_host_present_time_ms)
        .map(|at| (now_ms - at).max(0.0));
    let audio_playout_latency_ms = runtime_stats.and_then(resolve_audio_playout_latency_ms);
    let audio_video_playout_delta_ms =
        runtime_stats.and_then(|stats| resolve_audio_video_playout_delta_ms(stats, present_age_ms));
    let packet_to_decode_ms = runtime_stats.and_then(|stats| {
        stats
            .latest_video_packet_arrival_time_ms
            .zip(stats.latest_video_decode_ok_time_ms)
            .map(|(packet_at_ms, decode_at_ms)| (decode_at_ms - packet_at_ms).max(0.0))
    });
    let decode_to_present_ms = runtime_stats.and_then(|stats| {
        stats
            .latest_video_decode_ok_time_ms
            .zip(stats.latest_video_host_present_time_ms)
            .map(|(decode_at_ms, present_at_ms)| (present_at_ms - decode_at_ms).max(0.0))
    });
    let packet_to_present_ms = runtime_stats.and_then(|stats| {
        stats
            .latest_video_packet_arrival_time_ms
            .zip(stats.latest_video_host_present_time_ms)
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
    let resolved_video_bitrate_kbps =
        runtime_stats.and_then(|stats| resolve_video_inbound_bitrate_kbps(stats, now_ms));
    let resolved_audio_bitrate_kbps =
        runtime_stats.and_then(|stats| resolve_audio_inbound_bitrate_kbps(stats, now_ms));
    let resolved_total_bitrate_kbps =
        runtime_stats.and_then(|stats| resolve_total_inbound_bitrate_kbps(stats, now_ms));
    let latest_bwe = runtime_stats.and_then(|stats| stats.latest_video_bwe_observation.as_ref());
    let latest_twcc = runtime_stats.and_then(|stats| stats.latest_video_twcc_observation.as_ref());
    // 主口径固定为 transport inbound video bitrate。TWCC 仅作为诊断/BWE输入。
    let actual_video_bitrate_kbps = resolved_video_bitrate_kbps;
    let actual_video_bitrate_source = if resolved_video_bitrate_kbps.is_some() {
        Some("transport-metrics".to_string())
    } else {
        Some("unavailable".to_string())
    };
    let twcc_observation_state = runtime_stats.map(resolve_twcc_observation_state);
    let bitrate = resolved_video_bitrate_kbps
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
    let recovery_runtime_state = runtime_stats.map(project_runtime_state_from_stats);
    let video_owner = project_video_owner_contract(runtime_stats, recovery_runtime_state.as_ref());
    let video_owner = video_owner.as_ref();
    let stall_kind =
        classify_stall_kind(runtime_stats, recovery_runtime_state.as_ref(), video_owner);
    let display_phase = classify_display_phase(
        runtime_stats,
        recovery_runtime_state.as_ref(),
        video_owner,
        now_ms,
    );
    let runtime_remote_profile = resolve_runtime_remote_profile_strings(runtime_stats, now_ms);
    let remote_profile_baseline = recovery_runtime_state
        .as_ref()
        .map(|state| state.input_profile.baseline.clone())
        .or_else(|| runtime_remote_profile.baseline.clone());
    let remote_profile_effective_label = recovery_runtime_state
        .as_ref()
        .map(|state| state.input_profile.effective_label.clone())
        .or_else(|| runtime_remote_profile.effective_label.clone());
    let recovery_strategy_profile = resolve_recovery_strategy_profile(runtime_stats);
    let recovery_diagnosis =
        resolve_recovery_diagnosis(runtime_stats, recovery_runtime_state.as_ref(), video_owner);
    let video_health = video_owner
        .as_ref()
        .map(|owner| map_owner_state_to_video_health(owner.state.as_str()));
    let observation_note = build_observation_note(runtime_stats);
    let transport_recovery_note = build_transport_recovery_note(runtime_stats);
    let repair_probe_note = build_repair_probe_note(runtime_stats);
    let reinject_note = build_rtx_reinject_note(runtime_stats);
    let runtime_summary = build_runtime_summary(
        runtime_stats,
        remote_profile_effective_label.as_deref(),
        remote_profile_baseline.as_deref(),
        video_owner,
        display_phase.as_deref(),
        video_health.as_deref(),
        observation_note.as_deref(),
        transport_recovery_note.as_deref(),
        repair_probe_note.as_deref(),
        reinject_note.as_deref(),
    );
    let primary_issue_chain = build_primary_issue_chain(
        runtime_stats,
        display_phase.as_deref(),
        video_owner,
        stall_kind.as_deref(),
    );
    let latest_decision_summary = build_latest_decision_summary(
        snapshot,
        runtime_stats,
        video_owner,
        observation_note.as_deref(),
        transport_recovery_note.as_deref(),
        repair_probe_note.as_deref(),
        reinject_note.as_deref(),
    );
    let transport_strategy_profile =
        runtime_stats.and_then(|stats| stats.transport_policy_profile.clone());
    let remote_profile_dynamic = runtime_remote_profile.dynamic;
    let recovery_owner_state = video_owner.map(|owner| owner.state.clone());
    let recovery_owner_reason = video_owner.and_then(|owner| owner.reason.clone());

    XbxEngineStatsDto {
        resolution,
        rtt,
        fps,
        runtime_summary,
        primary_issue_chain,
        latest_decision_summary,
        remote_profile_baseline,
        remote_profile_dynamic,
        remote_profile_effective_label,
        session_phase: display_phase,
        transport_strategy_profile,
        recovery_strategy_profile,
        recovery_diagnosis,
        direct_gaming_bitrate_band: runtime_stats
            .and_then(|stats| stats.direct_gaming_bitrate_band.clone()),
        recovery_owner_state,
        recovery_owner_reason,
        video_owner_source: video_owner.and_then(|owner| owner.source.clone()),
        video_owner_observed_at_ms: video_owner.and_then(|owner| owner.observed_at_ms),
        video_health,
        stall_kind,
        inbound_video_fps: runtime_stats.map(|stats| stats.inbound_video_frame_rate_fps),
        decode_fps: runtime_stats.map(|stats| stats.video_decode_fps),
        present_fps: runtime_stats.map(|stats| stats.video_present_fps),
        pl: packet_loss,
        fl: String::new(),
        jit: jitter,
        br: bitrate,
        decode: String::new(),
        transport_path: runtime_stats.and_then(|stats| stats.transport_path.clone()),
        transport_candidate_pair: runtime_stats
            .and_then(|stats| stats.transport_candidate_pair.clone()),
        transport_protocol: runtime_stats.and_then(|stats| stats.transport_protocol.clone()),
        transport_address_family: runtime_stats
            .and_then(|stats| stats.transport_address_family.clone()),
        transport_state,
        video_rtt_source: runtime_stats.and_then(|stats| stats.video_rtt_source.clone()),
        video_remb_bps: runtime_stats.and_then(|stats| stats.video_remb_bps),
        // 统一口径：total 始终按 video + audio 聚合推导，避免历史缓存字段瞬时失真。
        inbound_bitrate_kbps: resolved_total_bitrate_kbps,
        inbound_video_bitrate_kbps: resolved_video_bitrate_kbps,
        inbound_audio_bitrate_kbps: resolved_audio_bitrate_kbps,
        latest_audio_playout_time_ms: runtime_stats
            .and_then(|stats| stats.latest_audio_playout_time_ms),
        audio_playout_latency_ms,
        audio_video_playout_delta_ms,
        actual_video_bitrate_source,
        video_bwe_mode: latest_bwe.map(|bwe| bwe.mode.clone()),
        video_bwe_reason: latest_bwe.map(|bwe| bwe.decision_reason.clone()),
        video_target_remb_kbps: latest_bwe.map(|bwe| bwe.target_remb_kbps).or_else(|| {
            runtime_stats.and_then(|stats| stats.video_remb_bps.map(|bps| bps / 1_000))
        }),
        video_observed_remb_kbps: latest_bwe.and_then(|bwe| bwe.observed_remb_kbps),
        video_actual_bitrate_kbps: actual_video_bitrate_kbps,
        video_twcc_receive_bitrate_kbps: latest_twcc
            .and_then(|twcc| twcc.receive_bitrate_kbps)
            .or_else(|| latest_bwe.and_then(|bwe| bwe.twcc_receive_bitrate_kbps)),
        video_twcc_loss_ratio: latest_twcc
            .map(|twcc| twcc.packet_loss_ratio)
            .or_else(|| latest_bwe.and_then(|bwe| bwe.twcc_loss_ratio)),
        video_twcc_delivery_ratio: latest_twcc
            .map(|twcc| twcc.delivery_ratio)
            .or_else(|| latest_bwe.and_then(|bwe| bwe.twcc_delivery_ratio)),
        video_twcc_feedback_interval_ms: latest_twcc
            .and_then(|twcc| twcc.feedback_interval_ms)
            .or_else(|| latest_bwe.and_then(|bwe| bwe.twcc_feedback_interval_ms)),
        twcc_observation_state,
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
        video_decoder_recovery_state: runtime_stats
            .and_then(|stats| stats.video_decoder_recovery_state.clone()),
        video_decoder_recovery_event: runtime_stats
            .and_then(|stats| stats.video_decoder_recovery_event.clone()),
        video_decoder_recovery_detail: runtime_stats
            .and_then(|stats| stats.video_decoder_recovery_detail.clone()),
        video_decoder_recovery_status: runtime_stats
            .and_then(|stats| stats.video_decoder_recovery_status),
        video_decoder_recovery_state_changed_at_ms: runtime_stats
            .and_then(|stats| stats.video_decoder_recovery_state_changed_at_ms),
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
        video_present_drop_count_total: runtime_stats
            .map(|stats| stats.video_present_drop_count_total),
        video_present_overwrite_count_total: runtime_stats
            .map(|stats| stats.video_present_overwrite_count_total),
        video_present_submit_count_total: runtime_stats
            .map(|stats| stats.video_present_submit_count_total),
        host_no_pending_take_count_total: runtime_stats
            .map(|stats| stats.host_no_pending_take_count_total),
        host_no_pending_streak: runtime_stats.map(|stats| stats.host_no_pending_streak),
        host_no_pending_max_streak: runtime_stats.map(|stats| stats.host_no_pending_max_streak),
        host_no_pending_pressure_level: runtime_stats
            .and_then(|stats| stats.host_no_pending_pressure_level.clone()),
        host_display_tick_epoch: runtime_stats.map(|stats| stats.host_display_tick_epoch),
        video_present_epoch: runtime_stats.map(|stats| stats.video_present_epoch),
        host_cadence_phase: runtime_stats.and_then(|stats| stats.host_cadence_phase.clone()),
        video_present_descriptor_upload_mode: runtime_stats
            .and_then(|stats| stats.video_present_descriptor_upload_mode.clone()),
        video_present_descriptor_metal_import_count_total: runtime_stats
            .map(|stats| stats.video_present_descriptor_metal_import_count_total),
        video_present_descriptor_cpu_upload_count_total: runtime_stats
            .map(|stats| stats.video_present_descriptor_cpu_upload_count_total),
        latest_keyframe_request_episode: runtime_stats.and_then(|stats| {
            stats
                .latest_keyframe_request_episode
                .as_ref()
                .map(
                    |episode| xbxengine_protocol::XbxEngineKeyframeRequestEpisodeObservationDto {
                        episode_id: episode.episode_id,
                        request_reason: episode.request_reason.clone(),
                        request_kind: episode.request_kind.clone(),
                        status: episode.status.clone(),
                        requested_at_ms: episode.requested_at_ms,
                        sent_at_ms: episode.sent_at_ms,
                        deadline_at_ms: episode.deadline_at_ms,
                        first_keyframe_packet_at_ms: episode.first_keyframe_packet_at_ms,
                        first_keyframe_decoded_at_ms: episode.first_keyframe_decoded_at_ms,
                        response_rtp_timestamp: episode.response_rtp_timestamp,
                        response_frame_seq: episode.response_frame_seq,
                        response_verdict: episode.response_verdict.clone(),
                    },
                )
        }),
        latest_h264_inspection_observation: runtime_stats.and_then(|stats| {
            h264_inspection_dto_from_observation(stats.latest_h264_inspection_observation.as_ref())
        }),
        recovery_keyframe_request_count: Some(snapshot.recovery_keyframe_request_count),
        recovery_decoder_reset_count: Some(snapshot.recovery_decoder_reset_count),
        recovery_reconnect_count: Some(snapshot.recovery_reconnect_count),
        recovery_hard_fallback_timer_ms: runtime_stats
            .and_then(|stats| stats.recovery_hard_fallback_timer_ms),
        recovery_hard_fallback_trigger_reason: runtime_stats
            .and_then(|stats| stats.recovery_hard_fallback_trigger_reason.clone()),
        recovery_hard_fallback_timer_reset_reason: runtime_stats
            .and_then(|stats| stats.recovery_hard_fallback_timer_reset_reason.clone()),
        last_recovery_action: snapshot.last_recovery_action.clone(),
        last_recovery_action_at_ms: snapshot.last_recovery_action_at_ms,
        last_recovery_reason: snapshot.last_recovery_reason.clone(),
        reconnect_trigger_source: snapshot.reconnect_trigger_source.clone(),
        latest_decode_candidate_decision: runtime_stats.and_then(|stats| {
            stats
                .latest_decode_candidate_decision
                .as_ref()
                .map(|decision| {
                    xbxengine_protocol::XbxEnginePipelineCandidateDecisionObservationDto {
                        decision_id: decision.decision_id,
                        state: decision.state.clone(),
                        action: decision.action.clone(),
                        detail: decision.detail.clone(),
                        frame_seq: decision.frame_seq,
                        observed_at_ms: decision.observed_at_ms,
                    }
                })
        }),
        latest_render_candidate_decision: runtime_stats.and_then(|stats| {
            stats
                .latest_render_candidate_decision
                .as_ref()
                .map(|decision| {
                    xbxengine_protocol::XbxEnginePipelineCandidateDecisionObservationDto {
                        decision_id: decision.decision_id,
                        state: decision.state.clone(),
                        action: decision.action.clone(),
                        detail: decision.detail.clone(),
                        frame_seq: decision.frame_seq,
                        observed_at_ms: decision.observed_at_ms,
                    }
                })
        }),
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
                    stage: drop.stage.clone(),
                    action: drop.action.clone(),
                    detail: drop.detail.clone(),
                    frame_rtp_timestamp: drop.frame_rtp_timestamp,
                    frame_seq: drop.frame_seq,
                    frame_recovery_disposition: drop.frame_recovery_disposition.clone(),
                    frame_unrecoverable_reason: drop.frame_unrecoverable_reason.clone(),
                    frame_budget: frame_budget_dto_from_observation(drop.frame_budget.as_ref())
                        .or_else(|| Some(infer_budget_from_drop(drop))),
                    observed_at_ms: drop.observed_at_ms,
                    width: drop.width,
                    height: drop.height,
                    is_keyframe: drop.is_keyframe,
                    queue_depth: drop.queue_depth,
                }
            })
        }),
        latest_video_frame_recovery_observation: runtime_stats.and_then(|stats| {
            stats
                .latest_video_frame_recovery_observation
                .as_ref()
                .map(
                    |observation| xbxengine_protocol::XbxEngineFrameRecoveryObservationDto {
                        observation_id: observation.observation_id,
                        action: observation.action.clone(),
                        frame_rtp_timestamp: observation.frame_rtp_timestamp,
                        frame_playout_deadline_at_ms: observation.frame_playout_deadline_at_ms,
                        frame_recovery_disposition: observation.frame_recovery_disposition.clone(),
                        frame_unrecoverable_reason: observation.frame_unrecoverable_reason.clone(),
                        frame_budget: frame_budget_dto_from_observation(
                            observation.frame_budget.as_ref(),
                        )
                        .or_else(|| Some(infer_budget_from_frame_recovery(observation))),
                        observed_at_ms: observation.observed_at_ms,
                    },
                )
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
                    estimated_recovery_arrival_ms: nack.estimated_recovery_arrival_ms,
                    nack_disposition: nack.nack_disposition.clone(),
                    frame_playout_deadline_at_ms: nack.frame_playout_deadline_at_ms,
                    frame_unrecoverable_reason: nack.frame_unrecoverable_reason.clone(),
                    frame_budget: frame_budget_dto_from_observation(nack.frame_budget.as_ref())
                        .or_else(|| Some(infer_budget_from_nack(nack))),
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
                        recovery_stage: escalation.recovery_stage.clone(),
                        recovery_chain_value: escalation.recovery_chain_value.clone(),
                        recovery_failure_cost: escalation.recovery_failure_cost.clone(),
                        recovery_window_source: escalation.recovery_window_source.clone(),
                        observed_at_ms: escalation.observed_at_ms,
                    },
                )
        }),
        latest_recovery_decision_ledger: runtime_stats.and_then(|stats| {
            stats
                .latest_recovery_decision_ledger
                .as_ref()
                .map(
                    |ledger| xbxengine_protocol::XbxEngineRecoveryDecisionLedgerObservationDto {
                        decision_id: ledger.decision_id,
                        state_before: ledger.state_before.clone(),
                        state_after: ledger.state_after.clone(),
                        input_signal: ledger.input_signal.clone(),
                        gate_result: ledger.gate_result.clone(),
                        action_selected: ledger.action_selected.clone(),
                        budget_before: ledger.budget_before.as_ref().map(|budget| {
                            xbxengine_protocol::XbxEngineRecoveryBudgetSnapshotDto {
                                recovery_epoch: budget.recovery_epoch,
                                keyframe_budget_used: budget.keyframe_budget_used,
                                keyframe_budget_limit: budget.keyframe_budget_limit,
                                decoder_reset_budget_used: budget.decoder_reset_budget_used,
                                decoder_reset_budget_limit: budget.decoder_reset_budget_limit,
                                reconnect_budget_used: budget.reconnect_budget_used,
                                reconnect_budget_limit: budget.reconnect_budget_limit,
                            }
                        }),
                        budget_after: ledger.budget_after.as_ref().map(|budget| {
                            xbxengine_protocol::XbxEngineRecoveryBudgetSnapshotDto {
                                recovery_epoch: budget.recovery_epoch,
                                keyframe_budget_used: budget.keyframe_budget_used,
                                keyframe_budget_limit: budget.keyframe_budget_limit,
                                decoder_reset_budget_used: budget.decoder_reset_budget_used,
                                decoder_reset_budget_limit: budget.decoder_reset_budget_limit,
                                reconnect_budget_used: budget.reconnect_budget_used,
                                reconnect_budget_limit: budget.reconnect_budget_limit,
                            }
                        }),
                        command_result: ledger.command_result.clone(),
                        command_detail: ledger.command_detail.clone(),
                        observed_at_ms: ledger.observed_at_ms,
                    },
                )
        }),
        latest_video_timeline_observation: runtime_stats.and_then(|stats| {
            stats
                .latest_video_timeline_observation
                .as_ref()
                .map(
                    |timeline| xbxengine_protocol::XbxEngineVideoTimelineObservationDto {
                        observation_id: timeline.observation_id,
                        source_event: timeline.source_event.clone(),
                        gap: timeline.gap.as_ref().map(|gap| {
                            xbxengine_protocol::XbxEngineVideoTimelineGapSnapshotDto {
                                state: gap.state.clone(),
                                sequence: gap.sequence,
                                frame_rtp_timestamp: gap.frame_rtp_timestamp,
                                frame_importance: gap.frame_importance.clone(),
                                observed_at_ms: gap.observed_at_ms,
                            }
                        }),
                        frame: timeline.frame.as_ref().map(|frame| {
                            xbxengine_protocol::XbxEngineVideoTimelineFrameSnapshotDto {
                                state: frame.state.clone(),
                                frame_rtp_timestamp: frame.frame_rtp_timestamp,
                                is_keyframe: frame.is_keyframe,
                                frame_importance: frame.frame_importance.clone(),
                                close_reason: frame.close_reason.clone(),
                                observed_at_ms: frame.observed_at_ms,
                            }
                        }),
                        chain: xbxengine_protocol::XbxEngineVideoTimelineChainSnapshotDto {
                            state: timeline.chain.state.clone(),
                            reason: timeline.chain.reason.clone(),
                            observed_at_ms: timeline.chain.observed_at_ms,
                        },
                        observed_at_ms: timeline.observed_at_ms,
                    },
                )
        }),
        latest_anchor_candidate_ledger: runtime_stats.and_then(|stats| {
            stats
                .latest_anchor_candidate_ledger
                .as_ref()
                .map(
                    |candidate| xbxengine_protocol::XbxEngineAnchorCandidateLedgerDto {
                        recovery_epoch: candidate.recovery_epoch,
                        frame_rtp_timestamp: candidate.frame_rtp_timestamp,
                        state: candidate.state.as_str().to_string(),
                        source_event: candidate.source_event.clone(),
                        failure_reason: candidate
                            .failure_reason
                            .map(|reason| reason.as_str().to_string()),
                        observed_at_ms: candidate.observed_at_ms,
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
                    source: twcc.source.clone(),
                    quality: twcc.quality.as_str().to_string(),
                    feedback_packet_count: twcc.feedback_packet_count,
                    covered_sequence_start: twcc.covered_sequence_start,
                    covered_sequence_end: twcc.covered_sequence_end,
                    covered_sequence_span: twcc.covered_sequence_span,
                    observed_packet_count: twcc.observed_packet_count,
                    observed_byte_count: twcc.observed_byte_count,
                    coverage_ratio: twcc.coverage_ratio,
                    ledger_hit_ratio: twcc.ledger_hit_ratio,
                    feedback_interval_ms: twcc.feedback_interval_ms,
                    arrival_span_ms: twcc.arrival_span_ms,
                    receive_bitrate_kbps: twcc.receive_bitrate_kbps,
                    twcc_sample_valid: twcc.twcc_sample_valid,
                    twcc_invalid_reason: twcc.twcc_invalid_reason.clone(),
                    delivery_ratio: twcc.delivery_ratio,
                    packet_loss_ratio: twcc.packet_loss_ratio,
                    observed_at_ms: twcc.observed_at_ms,
                }
            })
        }),
        latest_rtc_builder_observation: runtime_stats.and_then(|stats| {
            stats
                .latest_rtc_builder_observation
                .as_ref()
                .map(
                    |observation| xbxengine_protocol::XbxEngineRtcBuilderObservationDto {
                        observation_id: observation.observation_id,
                        controlled_twcc_registry: observation.controlled_twcc_registry,
                        feedback_interval_ms: observation.feedback_interval_ms,
                        registered_header_extensions: observation
                            .registered_header_extensions
                            .clone(),
                        registered_rtcp_feedback: observation.registered_rtcp_feedback.clone(),
                        observed_at_ms: observation.observed_at_ms,
                    },
                )
        }),
        latest_twcc_remote_stream_observation: runtime_stats.and_then(|stats| {
            stats
                .latest_twcc_remote_stream_observation
                .as_ref()
                .map(
                    |observation| xbxengine_protocol::XbxEngineTwccRemoteStreamObservationDto {
                        observation_id: observation.observation_id,
                        ssrc: observation.ssrc,
                        mime_type: observation.mime_type.clone(),
                        twcc_ext_id: observation.twcc_ext_id,
                        header_extensions: observation.header_extensions.clone(),
                        rtcp_feedback: observation.rtcp_feedback.clone(),
                        observed_at_ms: observation.observed_at_ms,
                    },
                )
        }),
        latest_remote_answer_observation: runtime_stats.and_then(|stats| {
            stats
                .latest_remote_answer_observation
                .as_ref()
                .map(
                    |observation| xbxengine_protocol::XbxEngineRemoteAnswerObservationDto {
                        observation_id: observation.observation_id,
                        video_payload_order: observation.video_payload_order.clone(),
                        selected_video_payload_type: observation.selected_video_payload_type,
                        selected_video_mime_type: observation.selected_video_mime_type.clone(),
                        selected_video_profile_level_id: observation
                            .selected_video_profile_level_id
                            .clone(),
                        selected_video_h264_sprop_parameter_sets: observation
                            .selected_video_h264_sprop_parameter_sets
                            .clone(),
                        accepted_video_rtcp_feedback: observation
                            .accepted_video_rtcp_feedback
                            .clone(),
                        accepted_audio_rtcp_feedback: observation
                            .accepted_audio_rtcp_feedback
                            .clone(),
                        accepted_video_header_extensions: observation
                            .accepted_video_header_extensions
                            .clone(),
                        accepted_audio_header_extensions: observation
                            .accepted_audio_header_extensions
                            .clone(),
                        observed_at_ms: observation.observed_at_ms,
                    },
                )
        }),
        latest_twcc_extension_observation: runtime_stats.and_then(|stats| {
            stats
                .latest_twcc_extension_observation
                .as_ref()
                .map(
                    |observation| xbxengine_protocol::XbxEngineTwccExtensionObservationDto {
                        observation_id: observation.observation_id,
                        state: observation.state.clone(),
                        ssrc: observation.ssrc,
                        sequence_number: observation.sequence_number,
                        expected_ext_id: observation.expected_ext_id,
                        packet_seen_count: observation.packet_seen_count,
                        missing_count: observation.missing_count,
                        observed_at_ms: observation.observed_at_ms,
                    },
                )
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
        latest_observation_label: runtime_stats
            .and_then(|stats| stats.latest_observation_label.clone()),
        latest_observation_summary: runtime_stats
            .and_then(|stats| stats.latest_observation_summary.clone()),
        latest_target_remb_action: runtime_stats
            .and_then(|stats| stats.latest_target_remb_action.clone()),
        latest_target_remb_summary: runtime_stats
            .and_then(|stats| stats.latest_target_remb_summary.clone()),
        build_fingerprint: None,
    }
}

fn resolve_runtime_remote_profile_strings(
    runtime_stats: Option<&XbxEngineMediaRuntimeStats>,
    now_ms: f64,
) -> RuntimeRemoteProfileStrings {
    if let Some(stats) = runtime_stats {
        let remote_profile_baseline = stats.baseline_remote_profile.clone();
        let remote_profile_dynamic = stats.dynamic_remote_subprofile.clone();
        let remote_profile_effective_label = stats.effective_remote_profile_label.clone();
        if remote_profile_baseline.is_some()
            && remote_profile_dynamic.is_some()
            && remote_profile_effective_label.is_some()
        {
            return RuntimeRemoteProfileStrings {
                baseline: remote_profile_baseline,
                dynamic: remote_profile_dynamic,
                effective_label: remote_profile_effective_label,
            };
        }
    }
    let fallback = classify_runtime_remote_profile(runtime_stats, now_ms);
    RuntimeRemoteProfileStrings {
        baseline: fallback
            .as_ref()
            .map(|profile| profile.baseline.as_str().to_string()),
        dynamic: fallback
            .as_ref()
            .map(|profile| profile.dynamic.as_str().to_string()),
        effective_label: fallback.map(|profile| profile.effective_label()),
    }
}

fn resolve_twcc_observation_state(stats: &XbxEngineMediaRuntimeStats) -> String {
    // 这组 phase 名称同时服务 diagnostics 展示和 session warmup 判定，
    // 变更时要同步检查 scheduling/session 对 cloud warmup 的消费逻辑。
    let has_video_remote_twcc_binding = stats
        .latest_twcc_remote_stream_observation
        .as_ref()
        .is_some_and(is_video_twcc_remote_stream_observation);
    let has_video_extension_signal = stats
        .latest_twcc_extension_observation
        .as_ref()
        .is_some_and(|observation| observation.state == "seen" || observation.state == "missing");
    let has_video_twcc_chain_signal = has_video_remote_twcc_binding || has_video_extension_signal;

    if stats
        .latest_video_twcc_observation
        .as_ref()
        .is_some_and(|observation| observation.source == "local-feedback")
    {
        return "local-feedback".to_string();
    }
    if stats.latest_video_twcc_observation.is_some() {
        return "remote-observed".to_string();
    }
    if stats
        .latest_twcc_extension_observation
        .as_ref()
        .is_some_and(|observation| observation.state == "missing")
        && has_video_twcc_chain_signal
    {
        return "missing-header-extension".to_string();
    }
    if has_video_twcc_chain_signal {
        return "missing-local-feedback".to_string();
    }
    if stats.latest_rtc_builder_observation.is_some() {
        return "builder-configured".to_string();
    }
    "unavailable".to_string()
}

fn is_video_twcc_remote_stream_observation(
    observation: &crate::XbxEngineTwccRemoteStreamObservation,
) -> bool {
    observation
        .mime_type
        .get(..5)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("video"))
}

// 用统一摘要描述当前 runtime 所处状态，便于回归时快速判断是否落在预期档位。
fn build_runtime_summary(
    runtime_stats: Option<&XbxEngineMediaRuntimeStats>,
    remote_profile_effective_label: Option<&str>,
    remote_profile_baseline: Option<&str>,
    video_owner: Option<&VideoOwnerContract>,
    display_phase: Option<&str>,
    video_health: Option<&str>,
    observation_note: Option<&str>,
    transport_recovery_note: Option<&str>,
    repair_probe_note: Option<&str>,
    reinject_note: Option<&str>,
) -> Option<String> {
    let stats = runtime_stats?;
    let profile = remote_profile_effective_label
        .or(remote_profile_baseline)
        .unwrap_or("unknown");
    let phase = display_phase.unwrap_or("unknown");
    let band = stats
        .direct_gaming_bitrate_band
        .as_deref()
        .unwrap_or("unknown");
    let owner_state = video_owner
        .map(|owner| owner.state.as_str())
        .unwrap_or("unknown");
    let health = video_health.unwrap_or("unknown");
    let base = format!("{profile}/{phase}/{band}/{owner_state}/{health}");
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
    _display_phase: Option<&str>,
    video_owner: Option<&VideoOwnerContract>,
    _stall_kind: Option<&str>,
) -> Option<String> {
    let _ = runtime_stats?;
    let owner = video_owner?;
    match owner.state.as_str() {
        "seeking-anchor" | "priming" => Some("startup:priming".to_string()),
        "stable-serving" => Some("steady:healthy".to_string()),
        "degraded-serving" => Some(format!(
            "steady:{}",
            owner.reason.as_deref().unwrap_or("degraded-serving")
        )),
        "rebuilding-supply" => Some(format!(
            "recovery:{}",
            owner.reason.as_deref().unwrap_or("rebuilding-supply")
        )),
        "supply-starved" => Some(format!(
            "display:{}",
            owner.reason.as_deref().unwrap_or("supply-starved")
        )),
        _ => Some(format!(
            "owner:{}:{}",
            owner.state,
            owner.reason.as_deref().unwrap_or("none")
        )),
    }
}

// 把最近一次真正影响行为的决策压成摘要，便于对照 trace 和面板。
fn build_latest_decision_summary(
    _snapshot: &XbxEngineRuntimeSnapshot,
    runtime_stats: Option<&XbxEngineMediaRuntimeStats>,
    video_owner: Option<&VideoOwnerContract>,
    _observation_note: Option<&str>,
    _transport_recovery_note: Option<&str>,
    _repair_probe_note: Option<&str>,
    _reinject_note: Option<&str>,
) -> Option<String> {
    let _ = runtime_stats?;
    let owner = video_owner?;
    Some(format!(
        "owner:{}:{}",
        owner.state,
        owner.reason.as_deref().unwrap_or("none")
    ))
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
    if stats.transport_recovery_episode_active {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DisplaySessionPhase {
    Connecting,
    Handshaking,
    Priming,
    Steady,
    Recovering,
}

impl DisplaySessionPhase {
    fn as_str(self) -> &'static str {
        match self {
            Self::Connecting => "connecting",
            Self::Handshaking => "handshaking",
            Self::Priming => "priming",
            Self::Steady => "steady",
            Self::Recovering => "recovering",
        }
    }
}

fn classify_display_phase(
    runtime_stats: Option<&XbxEngineMediaRuntimeStats>,
    runtime_state: Option<&RecoveryRuntimeState>,
    owner: Option<&VideoOwnerContract>,
    now_ms: f64,
) -> Option<String> {
    let stats = runtime_stats?;
    let phase = compute_display_phase(stats, runtime_state, owner, now_ms);
    Some(phase.as_str().to_string())
}

fn compute_display_phase(
    stats: &XbxEngineMediaRuntimeStats,
    runtime_state: Option<&RecoveryRuntimeState>,
    owner: Option<&VideoOwnerContract>,
    now_ms: f64,
) -> DisplaySessionPhase {
    let transport_state = format!("{:?}", stats.transport_state);
    if transport_state != "Connected" {
        return DisplaySessionPhase::Connecting;
    }
    if stats.message_handshake_acked_at_ms.is_none() {
        return DisplaySessionPhase::Handshaking;
    }
    if !has_visible_video_output(stats) || stats.control_ready_at_ms.is_none() {
        return DisplaySessionPhase::Priming;
    }
    if let Some(owner) = owner {
        return match owner.state.as_str() {
            "seeking-anchor" | "priming" => DisplaySessionPhase::Priming,
            "rebuilding-supply" | "supply-starved" => DisplaySessionPhase::Recovering,
            _ => DisplaySessionPhase::Steady,
        };
    }
    let fresh_output = has_recent_video_output(stats, now_ms);
    if !fresh_output
        && runtime_state
            .map(|state| {
                state.phase == crate::transport::rtc::recovery::startup::SessionPhase::Recovering
            })
            .unwrap_or_else(|| stats.session_phase.as_deref() == Some("recovering"))
    {
        return DisplaySessionPhase::Recovering;
    }
    if runtime_state
        .map(|state| state.phase == crate::transport::rtc::recovery::startup::SessionPhase::Startup)
        .unwrap_or(false)
    {
        return DisplaySessionPhase::Priming;
    }
    DisplaySessionPhase::Steady
}

fn has_visible_video_output(stats: &XbxEngineMediaRuntimeStats) -> bool {
    // 可见输出只认 host present 事实，不能把 submit 累计值当作当前屏幕输出证据。
    stats.latest_video_host_present_time_ms.is_some()
}

/// stall kind 用于界面/离线分析统一解释“这次卡住属于哪条链”。
fn classify_stall_kind(
    runtime_stats: Option<&XbxEngineMediaRuntimeStats>,
    runtime_state: Option<&RecoveryRuntimeState>,
    owner: Option<&VideoOwnerContract>,
) -> Option<String> {
    let stats = runtime_stats?;
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0);
    if let Some(owner_reason) = owner.and_then(|owner| owner.reason.as_deref()) {
        return Some(match owner_reason {
            "adapterIdleTimeout" | "displaySupplyCritical" => "idleTimeout".to_string(),
            "decoderBackendFailure" => "decoderBackendFailure".to_string(),
            "transportSampleLoss" => "sampleLoss".to_string(),
            "transportAwaitRecoveryKeyframe" | "ingressWaitKeyframe" => {
                "waitingKeyframe".to_string()
            }
            "reconfigure" => "reconfigure".to_string(),
            _ => {
                if stats.video_decoder_stalled == Some(true)
                    || stats.video_renderer_stalled == Some(true)
                {
                    "pipelineStall".to_string()
                } else {
                    "recovering".to_string()
                }
            }
        });
    }
    let fresh_output = has_recent_video_output(stats, now_ms);
    if stats.video_decoder_stalled == Some(true) || stats.video_renderer_stalled == Some(true) {
        return Some("pipelineStall".to_string());
    }
    if stats.direct_gaming_bitrate_band.as_deref() == Some("paused")
        && stats.inbound_video_bitrate_kbps.unwrap_or(0.0) <= 0.1
        && stats.video_present_fps <= 1.0
    {
        return Some("videoPaused".to_string());
    }
    let recovery_phase = runtime_state.map(|state| state.phase);
    if (recovery_phase == Some(crate::transport::rtc::recovery::startup::SessionPhase::Startup)
        || stats.session_phase.as_deref() == Some("startup"))
        && stats.direct_gaming_bitrate_band.as_deref() == Some("startupLow")
    {
        return Some("startupLowQuality".to_string());
    }
    if !fresh_output
        && (recovery_phase
            == Some(crate::transport::rtc::recovery::startup::SessionPhase::Recovering)
            || stats.session_phase.as_deref() == Some("recovering"))
    {
        return Some("recovering".to_string());
    }
    Some("none".to_string())
}

fn has_recent_video_output(stats: &XbxEngineMediaRuntimeStats, now_ms: f64) -> bool {
    const RECENT_VIDEO_OUTPUT_WINDOW_MS: f64 = 500.0;
    let present_fresh = stats
        .latest_video_host_present_time_ms
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

fn resolve_video_inbound_bitrate_kbps(
    stats: &XbxEngineMediaRuntimeStats,
    _now_ms: f64,
) -> Option<f64> {
    stats
        .inbound_video_bitrate_kbps
        .filter(|value| *value > 0.1)
}

fn resolve_audio_inbound_bitrate_kbps(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> Option<f64> {
    stats
        .inbound_audio_bitrate_kbps
        .filter(|value| *value > 0.1)
        .or_else(|| estimate_audio_inbound_bitrate_kbps(stats, now_ms))
}

fn resolve_audio_playout_latency_ms(stats: &XbxEngineMediaRuntimeStats) -> Option<f64> {
    stats.audio_playout_latency_ms.filter(|value| *value >= 0.0)
}

fn resolve_audio_video_playout_delta_ms(
    stats: &XbxEngineMediaRuntimeStats,
    present_age_ms: Option<f64>,
) -> Option<f64> {
    resolve_audio_playout_latency_ms(stats)
        .zip(present_age_ms)
        .map(|(audio, present)| audio - present)
}

fn resolve_total_inbound_bitrate_kbps(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> Option<f64> {
    let video_kbps = resolve_video_inbound_bitrate_kbps(stats, now_ms);
    let audio_kbps = resolve_audio_inbound_bitrate_kbps(stats, now_ms);
    match (video_kbps, audio_kbps) {
        (Some(video), Some(audio)) => Some(video.max(0.0) + audio.max(0.0)),
        (Some(video), None) => Some(video.max(0.0)),
        (None, Some(audio)) => Some(audio.max(0.0)),
        (None, None) => stats
            .inbound_bitrate_kbps
            .filter(|value| *value > 0.1)
            .or_else(|| estimate_total_inbound_bitrate_kbps(stats, now_ms)),
    }
}

fn estimate_total_inbound_bitrate_kbps(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> Option<f64> {
    let video_kbps = resolve_video_inbound_bitrate_kbps(stats, now_ms).unwrap_or(0.0);
    let audio_kbps = resolve_audio_inbound_bitrate_kbps(stats, now_ms).unwrap_or(0.0);
    let total = video_kbps + audio_kbps;
    if total > 0.0 {
        Some(total)
    } else {
        None
    }
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
            reconnect_trigger_source: None,
        }
    }

    #[test]
    fn audio_playout_latency_helpers_return_latency_and_av_delta() {
        let stats = XbxEngineMediaRuntimeStats {
            audio_playout_latency_ms: Some(42.5),
            ..XbxEngineMediaRuntimeStats::default()
        };

        assert_eq!(resolve_audio_playout_latency_ms(&stats), Some(42.5));
        assert_eq!(
            resolve_audio_video_playout_delta_ms(&stats, Some(30.0)),
            Some(12.5)
        );
        assert_eq!(resolve_audio_video_playout_delta_ms(&stats, None), None);
    }

    #[test]
    fn runtime_summary_includes_transport_recovery_epoch_note() {
        let stats = XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            transport_policy_profile: Some("cloud".to_string()),
            session_phase: Some("recovering".to_string()),
            message_handshake_acked_at_ms: Some(10.0),
            control_ready_at_ms: Some(20.0),
            latest_video_host_present_time_ms: Some(30.0),
            video_present_submit_count_total: 1,
            direct_gaming_bitrate_band: Some("steady".to_string()),
            video_owner_state: Some("rebuilding-supply".to_string()),
            video_owner_reason: Some("transportAwaitRecoveryKeyframe".to_string()),
            transport_recovery_epoch: 7,
            transport_recovery_epoch_at_last_escalation: 6,
            transport_recovery_episode_active: true,
            ..XbxEngineMediaRuntimeStats::default()
        };

        let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
        let runtime_summary = dto.runtime_summary.expect("runtime summary");

        assert!(runtime_summary.contains("repoch:7:active"));
    }

    #[test]
    fn runtime_summary_uses_remote_profile_input_and_owner_state_as_main_view() {
        let stats = XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            transport_policy_profile: Some("cloud".to_string()),
            baseline_remote_profile: Some("cloudGaming".to_string()),
            dynamic_remote_subprofile: Some("cloudHighRtt".to_string()),
            effective_remote_profile_label: Some("cloudGaming+cloudHighRtt".to_string()),
            session_phase: Some("recovering".to_string()),
            direct_gaming_bitrate_band: Some("steady".to_string()),
            video_owner_state: Some("rebuilding-supply".to_string()),
            video_owner_reason: Some("transportAwaitRecoveryKeyframe".to_string()),
            ..XbxEngineMediaRuntimeStats::default()
        };

        let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
        assert_eq!(
            dto.runtime_summary.as_deref(),
            Some("cloudGaming+cloudHighRtt/handshaking/steady/rebuilding-supply/recovering")
        );
    }

    #[test]
    fn latest_decision_summary_is_driven_by_canonical_owner_contract() {
        let stats = XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            transport_recovery_epoch: 3,
            transport_recovery_epoch_at_last_escalation: 3,
            video_owner_state: Some("rebuilding-supply".to_string()),
            video_owner_reason: Some("transportAwaitRecoveryKeyframe".to_string()),
            video_owner_source: Some("anchor".to_string()),
            video_owner_observed_at_ms: Some(1234.0),
            ..XbxEngineMediaRuntimeStats::default()
        };

        let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
        assert_eq!(
            dto.latest_decision_summary.as_deref(),
            Some("owner:rebuilding-supply:transportAwaitRecoveryKeyframe")
        );
    }

    #[test]
    fn runtime_summary_includes_repair_probe_note_when_active() {
        let stats = XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            transport_policy_profile: Some("cloud".to_string()),
            session_phase: Some("steady".to_string()),
            message_handshake_acked_at_ms: Some(10.0),
            control_ready_at_ms: Some(20.0),
            latest_video_host_present_time_ms: Some(30.0),
            video_present_submit_count_total: 1,
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
            message_handshake_acked_at_ms: Some(10.0),
            control_ready_at_ms: Some(20.0),
            latest_video_host_present_time_ms: Some(30.0),
            video_present_submit_count_total: 1,
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
    fn owner_contract_projection_reads_canonical_runtime_owner_fields() {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as f64;
        let stats = XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            message_handshake_acked_at_ms: Some(now_ms - 120.0),
            control_ready_at_ms: Some(now_ms - 110.0),
            latest_video_host_present_time_ms: Some(now_ms - 30.0),
            latest_video_decode_ok_time_ms: Some(now_ms - 30.0),
            video_present_submit_count_total: 2,
            video_owner_state: Some("rebuilding-supply".to_string()),
            video_owner_reason: Some("timelineReferenceBroken".to_string()),
            video_owner_source: Some("anchor".to_string()),
            video_owner_observed_at_ms: Some(now_ms - 10.0),
            ..XbxEngineMediaRuntimeStats::default()
        };

        let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
        assert_eq!(
            dto.recovery_owner_state.as_deref(),
            Some("rebuilding-supply")
        );
        assert_eq!(
            dto.recovery_owner_reason.as_deref(),
            Some("timelineReferenceBroken")
        );
        assert_eq!(dto.video_owner_source.as_deref(), Some("anchor"));
        assert_eq!(dto.video_owner_observed_at_ms, Some(now_ms - 10.0));
        assert_eq!(dto.video_health.as_deref(), Some("recovering"));
        assert_eq!(
            dto.primary_issue_chain.as_deref(),
            Some("recovery:timelineReferenceBroken")
        );
        assert_eq!(
            dto.latest_decision_summary.as_deref(),
            Some("owner:rebuilding-supply:timelineReferenceBroken")
        );
        // coupling 字段仅保留为辅助观测，不参与 owner 语义。
    }

    #[test]
    fn owner_contract_falls_back_to_runtime_state_primary_view() {
        let stats = XbxEngineMediaRuntimeStats {
            session_phase: Some("recovering".to_string()),
            recovery_diagnosis: Some("transportAwaitRecoveryKeyframe".to_string()),
            ..XbxEngineMediaRuntimeStats::default()
        };

        let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));

        assert_eq!(
            dto.recovery_owner_state.as_deref(),
            Some("rebuilding-supply")
        );
        assert_eq!(
            dto.recovery_owner_reason.as_deref(),
            Some("transportAwaitRecoveryKeyframe")
        );
        assert_eq!(
            dto.video_owner_source.as_deref(),
            Some("runtime-recovering")
        );
        assert_eq!(
            dto.latest_decision_summary.as_deref(),
            Some("owner:rebuilding-supply:transportAwaitRecoveryKeyframe")
        );
    }

    #[test]
    fn build_stats_uses_handshaking_phase_before_handshake_ack() {
        let stats = XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            transport_policy_profile: Some("cloud".to_string()),
            direct_gaming_bitrate_band: Some("steady".to_string()),
            ..XbxEngineMediaRuntimeStats::default()
        };

        let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));

        assert_eq!(dto.session_phase.as_deref(), Some("handshaking"));
        assert_eq!(dto.video_health, None);
        assert_eq!(dto.primary_issue_chain, None);
    }

    #[test]
    fn build_stats_uses_priming_phase_before_first_present() {
        let stats = XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            transport_policy_profile: Some("cloud".to_string()),
            direct_gaming_bitrate_band: Some("steady".to_string()),
            message_handshake_acked_at_ms: Some(10.0),
            control_ready_at_ms: Some(20.0),
            latest_video_packet_arrival_time_ms: Some(30.0),
            latest_video_decode_ok_time_ms: Some(35.0),
            ..XbxEngineMediaRuntimeStats::default()
        };

        let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));

        assert_eq!(dto.session_phase.as_deref(), Some("priming"));
        assert_eq!(dto.video_health, None);
        assert_eq!(dto.primary_issue_chain, None);
    }

    #[test]
    fn build_stats_keeps_priming_when_only_submit_count_exists_without_host_present() {
        let stats = XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            transport_policy_profile: Some("cloud".to_string()),
            direct_gaming_bitrate_band: Some("steady".to_string()),
            message_handshake_acked_at_ms: Some(10.0),
            control_ready_at_ms: Some(20.0),
            latest_video_packet_arrival_time_ms: Some(30.0),
            latest_video_decode_ok_time_ms: Some(35.0),
            video_present_submit_count_total: 120,
            video_present_fps: 0.0,
            ..XbxEngineMediaRuntimeStats::default()
        };

        let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));

        assert_eq!(dto.session_phase.as_deref(), Some("priming"));
        assert_eq!(dto.recovery_owner_state, None);
        assert_eq!(dto.video_health, None);
    }

    #[test]
    fn build_stats_only_turns_healthy_after_first_present() {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as f64;
        let stats = XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            transport_policy_profile: Some("cloud".to_string()),
            direct_gaming_bitrate_band: Some("steady".to_string()),
            message_handshake_acked_at_ms: Some(now_ms - 100.0),
            control_ready_at_ms: Some(now_ms - 90.0),
            latest_video_host_present_time_ms: Some(now_ms - 20.0),
            latest_video_decode_ok_time_ms: Some(now_ms - 20.0),
            video_present_submit_count_total: 1,
            video_present_fps: 60.0,
            video_owner_state: Some("stable-serving".to_string()),
            video_owner_reason: Some("steady".to_string()),
            video_owner_source: Some("anchor".to_string()),
            video_owner_observed_at_ms: Some(now_ms - 5.0),
            ..XbxEngineMediaRuntimeStats::default()
        };

        let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));

        assert_eq!(dto.session_phase.as_deref(), Some("steady"));
        assert_eq!(dto.video_health.as_deref(), Some("healthy"));
        assert_eq!(dto.recovery_owner_state.as_deref(), Some("stable-serving"));
        assert_eq!(dto.video_owner_source.as_deref(), Some("anchor"));
        assert_eq!(dto.primary_issue_chain.as_deref(), Some("steady:healthy"));
    }

    #[test]
    fn build_stats_reports_recovering_after_first_present_when_output_turns_stale() {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as f64;
        let stats = XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            transport_policy_profile: Some("cloud".to_string()),
            session_phase: Some("recovering".to_string()),
            direct_gaming_bitrate_band: Some("steady".to_string()),
            message_handshake_acked_at_ms: Some(now_ms - 1_000.0),
            control_ready_at_ms: Some(now_ms - 990.0),
            latest_video_host_present_time_ms: Some(now_ms - 800.0),
            latest_video_decode_ok_time_ms: Some(now_ms - 800.0),
            video_present_submit_count_total: 1,
            recovery_diagnosis: Some("adapterIdleTimeout".to_string()),
            video_owner_state: Some("rebuilding-supply".to_string()),
            video_owner_reason: Some("adapterIdleTimeout".to_string()),
            video_owner_source: Some("anchor".to_string()),
            video_owner_observed_at_ms: Some(now_ms - 10.0),
            ..XbxEngineMediaRuntimeStats::default()
        };

        let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));

        assert_eq!(dto.session_phase.as_deref(), Some("recovering"));
        assert_eq!(dto.video_health.as_deref(), Some("recovering"));
        assert_eq!(
            dto.primary_issue_chain.as_deref(),
            Some("recovery:adapterIdleTimeout")
        );
    }

    #[test]
    fn build_stats_prioritizes_display_supply_starved_when_no_pending_and_present_age_is_stale() {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as f64;
        let stats = XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            transport_policy_profile: Some("cloud".to_string()),
            session_phase: Some("steady".to_string()),
            direct_gaming_bitrate_band: Some("steady".to_string()),
            message_handshake_acked_at_ms: Some(now_ms - 4_000.0),
            control_ready_at_ms: Some(now_ms - 3_900.0),
            latest_video_host_present_time_ms: Some(now_ms - 2_200.0),
            latest_video_decode_ok_time_ms: Some(now_ms - 1_600.0),
            video_present_submit_count_total: 120,
            video_present_fps: 1.0,
            host_no_pending_pressure_level: Some("critical".to_string()),
            host_no_pending_streak: 1_280,
            host_no_pending_max_streak: 1_500,
            video_owner_state: Some("supply-starved".to_string()),
            video_owner_reason: Some("supply-starved".to_string()),
            video_owner_source: Some("supply".to_string()),
            video_owner_observed_at_ms: Some(now_ms - 10.0),
            ..XbxEngineMediaRuntimeStats::default()
        };

        let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
        assert_eq!(dto.video_health.as_deref(), Some("displaySupplyStarved"));
        assert_eq!(dto.recovery_owner_state.as_deref(), Some("supply-starved"));
        assert_eq!(dto.recovery_owner_reason.as_deref(), Some("supply-starved"));
        assert_eq!(dto.video_owner_source.as_deref(), Some("supply"));
        assert_eq!(
            dto.primary_issue_chain.as_deref(),
            Some("display:supply-starved")
        );
    }

    #[test]
    fn build_stats_projects_host_cadence_epoch_fields() {
        let stats = XbxEngineMediaRuntimeStats {
            host_display_tick_epoch: 128,
            video_present_epoch: 96,
            host_cadence_phase: Some("starved".to_string()),
            ..XbxEngineMediaRuntimeStats::default()
        };

        let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
        assert_eq!(dto.host_display_tick_epoch, Some(128));
        assert_eq!(dto.video_present_epoch, Some(96));
        assert_eq!(dto.host_cadence_phase.as_deref(), Some("starved"));
    }

    #[test]
    fn build_stats_ignores_stale_recovery_diagnosis_when_output_is_fresh() {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as f64;
        let stats = XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            transport_policy_profile: Some("cloud".to_string()),
            session_phase: Some("steady".to_string()),
            direct_gaming_bitrate_band: Some("steady".to_string()),
            recovery_diagnosis: Some("adapterIdleTimeout".to_string()),
            message_handshake_acked_at_ms: Some(now_ms - 80.0),
            control_ready_at_ms: Some(now_ms - 70.0),
            latest_video_host_present_time_ms: Some(now_ms - 35.0),
            latest_video_decode_ok_time_ms: Some(now_ms - 35.0),
            video_present_submit_count_total: 2,
            video_present_fps: 58.0,
            host_no_pending_pressure_level: Some("normal".to_string()),
            host_no_pending_streak: 1,
            video_owner_state: Some("stable-serving".to_string()),
            video_owner_reason: Some("steady".to_string()),
            video_owner_source: Some("anchor".to_string()),
            video_owner_observed_at_ms: Some(now_ms - 10.0),
            ..XbxEngineMediaRuntimeStats::default()
        };

        let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
        assert_eq!(dto.video_health.as_deref(), Some("healthy"));
        assert_eq!(dto.primary_issue_chain.as_deref(), Some("steady:healthy"));
    }

    #[test]
    fn build_stats_prioritizes_recent_timeline_recovering_over_healthy_summary() {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as f64;
        let stats = XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            transport_policy_profile: Some("cloud".to_string()),
            session_phase: Some("steady".to_string()),
            direct_gaming_bitrate_band: Some("steady".to_string()),
            message_handshake_acked_at_ms: Some(now_ms - 80.0),
            control_ready_at_ms: Some(now_ms - 70.0),
            latest_video_host_present_time_ms: Some(now_ms - 35.0),
            latest_video_decode_ok_time_ms: Some(now_ms - 35.0),
            video_present_submit_count_total: 2,
            video_present_fps: 58.0,
            video_owner_state: Some("rebuilding-supply".to_string()),
            video_owner_reason: Some("inspectionRejectInvalidSliceHeader".to_string()),
            video_owner_source: Some("anchor".to_string()),
            video_owner_observed_at_ms: Some(now_ms - 10.0),
            latest_video_timeline_observation: Some(crate::XbxEngineVideoTimelineObservation {
                observation_id: 7,
                source_event: "frame-await-recovery-keyframe".to_string(),
                gap: None,
                frame: Some(crate::XbxEngineVideoTimelineFrameSnapshot {
                    state: "closed".to_string(),
                    frame_rtp_timestamp: Some(123),
                    is_keyframe: Some(false),
                    frame_importance: Some("unknown".to_string()),
                    close_reason: Some("inspectionRejectInvalidSliceHeader".to_string()),
                    observed_at_ms: now_ms - 20.0,
                }),
                chain: crate::XbxEngineVideoTimelineChainSnapshot {
                    state: "recovering".to_string(),
                    reason: Some("inspectionRejectInvalidSliceHeader".to_string()),
                    observed_at_ms: now_ms - 20.0,
                },
                observed_at_ms: now_ms - 20.0,
            }),
            latest_video_escalation_observation: Some(crate::XbxEngineVideoEscalationObservation {
                observation_id: 1,
                reason: "transportAwaitRecoveryKeyframe".to_string(),
                action: "requestDecoderReset".to_string(),
                recovery_stage: "rebuilding-supply".to_string(),
                recovery_chain_value: "anchor".to_string(),
                recovery_failure_cost: "high".to_string(),
                recovery_window_source: "transport-await-window".to_string(),
                observed_at_ms: now_ms - 10.0,
            }),
            ..XbxEngineMediaRuntimeStats::default()
        };

        let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
        assert_eq!(dto.video_health.as_deref(), Some("recovering"));
        assert_eq!(
            dto.recovery_owner_state.as_deref(),
            Some("rebuilding-supply")
        );
        assert_eq!(
            dto.recovery_owner_reason.as_deref(),
            Some("inspectionRejectInvalidSliceHeader")
        );
        assert_eq!(dto.video_owner_source.as_deref(), Some("anchor"));
        assert_eq!(
            dto.primary_issue_chain.as_deref(),
            Some("recovery:inspectionRejectInvalidSliceHeader")
        );
        assert_eq!(
            dto.latest_decision_summary.as_deref(),
            Some("owner:rebuilding-supply:inspectionRejectInvalidSliceHeader")
        );
    }

    #[test]
    fn build_stats_prioritizes_recent_timeline_broken_over_steady_healthy() {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as f64;
        let stats = XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            transport_policy_profile: Some("cloud".to_string()),
            session_phase: Some("steady".to_string()),
            direct_gaming_bitrate_band: Some("steady".to_string()),
            message_handshake_acked_at_ms: Some(now_ms - 80.0),
            control_ready_at_ms: Some(now_ms - 70.0),
            latest_video_host_present_time_ms: Some(now_ms - 40.0),
            latest_video_decode_ok_time_ms: Some(now_ms - 35.0),
            video_present_submit_count_total: 3,
            video_present_fps: 60.0,
            video_owner_state: Some("rebuilding-supply".to_string()),
            video_owner_reason: Some("cloudHighRttLowValueAdmission".to_string()),
            video_owner_source: Some("anchor".to_string()),
            video_owner_observed_at_ms: Some(now_ms - 10.0),
            latest_video_timeline_observation: Some(crate::XbxEngineVideoTimelineObservation {
                observation_id: 8,
                source_event: "nack-observation".to_string(),
                gap: Some(crate::XbxEngineVideoTimelineGapSnapshot {
                    state: "expired".to_string(),
                    sequence: Some(38022),
                    frame_rtp_timestamp: Some(456),
                    frame_importance: Some("delta".to_string()),
                    observed_at_ms: now_ms - 15.0,
                }),
                frame: Some(crate::XbxEngineVideoTimelineFrameSnapshot {
                    state: "gap-present".to_string(),
                    frame_rtp_timestamp: Some(456),
                    is_keyframe: Some(false),
                    frame_importance: Some("delta".to_string()),
                    close_reason: None,
                    observed_at_ms: now_ms - 15.0,
                }),
                chain: crate::XbxEngineVideoTimelineChainSnapshot {
                    state: "broken".to_string(),
                    reason: Some("cloudHighRttLowValueAdmission".to_string()),
                    observed_at_ms: now_ms - 15.0,
                },
                observed_at_ms: now_ms - 15.0,
            }),
            ..XbxEngineMediaRuntimeStats::default()
        };

        let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
        assert_eq!(dto.video_health.as_deref(), Some("recovering"));
        assert_eq!(
            dto.recovery_owner_state.as_deref(),
            Some("rebuilding-supply")
        );
        assert_eq!(
            dto.recovery_owner_reason.as_deref(),
            Some("cloudHighRttLowValueAdmission")
        );
        assert_eq!(dto.video_owner_source.as_deref(), Some("anchor"));
        assert_eq!(
            dto.primary_issue_chain.as_deref(),
            Some("recovery:cloudHighRttLowValueAdmission")
        );
        assert_eq!(
            dto.latest_decision_summary.as_deref(),
            Some("owner:rebuilding-supply:cloudHighRttLowValueAdmission")
        );
    }

    #[test]
    fn build_stats_owner_contract_prefers_canonical_owner_over_coupling_signals() {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as f64;
        let stats = XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            transport_policy_profile: Some("cloud".to_string()),
            session_phase: Some("steady".to_string()),
            direct_gaming_bitrate_band: Some("steady".to_string()),
            message_handshake_acked_at_ms: Some(now_ms - 2000.0),
            control_ready_at_ms: Some(now_ms - 1900.0),
            latest_video_host_present_time_ms: Some(now_ms - 1700.0),
            latest_video_decode_ok_time_ms: Some(now_ms - 1700.0),
            video_present_submit_count_total: 64,
            host_no_pending_pressure_level: Some("critical".to_string()),
            host_no_pending_streak: 2048,
            video_renderer_stalled: Some(true),
            video_owner_state: Some("rebuilding-supply".to_string()),
            video_owner_reason: Some("inspectionRejectInvalidSliceHeader".to_string()),
            video_owner_source: Some("anchor".to_string()),
            video_owner_observed_at_ms: Some(now_ms - 10.0),
            latest_video_timeline_observation: Some(crate::XbxEngineVideoTimelineObservation {
                observation_id: 9,
                source_event: "frame-await-recovery-keyframe".to_string(),
                gap: None,
                frame: None,
                chain: crate::XbxEngineVideoTimelineChainSnapshot {
                    state: "recovering".to_string(),
                    reason: Some("inspectionRejectInvalidSliceHeader".to_string()),
                    observed_at_ms: now_ms - 15.0,
                },
                observed_at_ms: now_ms - 15.0,
            }),
            ..XbxEngineMediaRuntimeStats::default()
        };

        let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
        assert_eq!(dto.video_health.as_deref(), Some("recovering"));
        assert_eq!(
            dto.recovery_owner_state.as_deref(),
            Some("rebuilding-supply")
        );
        assert_eq!(dto.video_owner_source.as_deref(), Some("anchor"));
        assert_eq!(
            dto.primary_issue_chain.as_deref(),
            Some("recovery:inspectionRejectInvalidSliceHeader")
        );
        assert_eq!(
            dto.latest_decision_summary.as_deref(),
            Some("owner:rebuilding-supply:inspectionRejectInvalidSliceHeader")
        );
    }

    #[test]
    fn build_stats_falls_back_to_runtime_strategy_profile() {
        let stats = XbxEngineMediaRuntimeStats {
            baseline_remote_profile: Some("cloudGaming".to_string()),
            dynamic_remote_subprofile: Some("cloudHighRtt".to_string()),
            effective_remote_profile_label: Some("cloudGaming+cloudHighRtt".to_string()),
            ..XbxEngineMediaRuntimeStats::default()
        };

        let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
        assert_eq!(
            dto.recovery_strategy_profile.as_deref(),
            Some("cloudGaming")
        );
        assert_eq!(
            dto.remote_profile_effective_label.as_deref(),
            Some("cloudGaming+cloudHighRtt")
        );
    }

    #[test]
    fn build_stats_recovery_strategy_profile_follows_runtime_strategy_profile() {
        let stats = XbxEngineMediaRuntimeStats {
            session_target_type: Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud),
            baseline_remote_profile: Some("relayGaming".to_string()),
            ..XbxEngineMediaRuntimeStats::default()
        };

        let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
        assert_eq!(
            dto.recovery_strategy_profile.as_deref(),
            Some("relayGaming")
        );
        assert_eq!(dto.remote_profile_baseline.as_deref(), Some("relayGaming"));
    }

    #[test]
    fn runtime_summary_profile_slot_prefers_runtime_profile_over_transport_policy() {
        let stats = XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            transport_policy_profile: Some("cloud".to_string()),
            baseline_remote_profile: Some("relayGaming".to_string()),
            effective_remote_profile_label: Some("relayGaming+steady".to_string()),
            session_phase: Some("steady".to_string()),
            direct_gaming_bitrate_band: Some("steady".to_string()),
            message_handshake_acked_at_ms: Some(10.0),
            control_ready_at_ms: Some(20.0),
            latest_video_host_present_time_ms: Some(30.0),
            video_present_submit_count_total: 1,
            video_owner_state: Some("stable-serving".to_string()),
            video_owner_reason: Some("steady".to_string()),
            ..XbxEngineMediaRuntimeStats::default()
        };

        let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
        assert_eq!(
            dto.runtime_summary.as_deref(),
            Some("relayGaming+steady/steady/steady/stable-serving/healthy")
        );
    }

    #[test]
    fn runtime_summary_profile_slot_does_not_fallback_to_transport_policy_only() {
        let stats = XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            transport_policy_profile: Some("cloud".to_string()),
            message_handshake_acked_at_ms: Some(10.0),
            control_ready_at_ms: Some(20.0),
            latest_video_host_present_time_ms: Some(30.0),
            video_present_submit_count_total: 1,
            direct_gaming_bitrate_band: Some("steady".to_string()),
            ..XbxEngineMediaRuntimeStats::default()
        };

        let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
        assert_eq!(
            dto.runtime_summary.as_deref(),
            Some("homeLanGaming+steady/steady/steady/stable-serving/healthy")
        );
    }

    #[test]
    fn build_stats_prefers_owner_reason_for_recovery_diagnosis() {
        let stats = XbxEngineMediaRuntimeStats {
            recovery_diagnosis: Some("transportExpiredDeadline".to_string()),
            video_owner_state: Some("rebuilding-supply".to_string()),
            video_owner_reason: Some("inspectionRejectInvalidSliceHeader".to_string()),
            ..XbxEngineMediaRuntimeStats::default()
        };

        let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
        assert_eq!(
            dto.recovery_diagnosis.as_deref(),
            Some("inspectionRejectInvalidSliceHeader")
        );
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
        assert!(
            dto.inbound_bitrate_kbps.unwrap_or(0.0)
                >= dto.inbound_video_bitrate_kbps.unwrap_or(0.0)
        );
    }

    #[test]
    fn video_inbound_bitrate_does_not_fallback_to_media_ingress_bytes_when_transport_stats_are_zero(
    ) {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as f64;
        let stats = XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            first_video_packet_arrival_time_ms: Some(now_ms - 2_000.0),
            latest_video_packet_arrival_time_ms: Some(now_ms - 16.0),
            inbound_video_bytes_total: 2_000_000,
            inbound_video_bitrate_kbps: Some(0.0),
            ..XbxEngineMediaRuntimeStats::default()
        };

        let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
        assert_eq!(dto.inbound_video_bitrate_kbps, None);
        assert_eq!(dto.video_actual_bitrate_kbps, None);
        assert_eq!(
            dto.actual_video_bitrate_source.as_deref(),
            Some("unavailable")
        );
    }

    #[test]
    fn total_inbound_bitrate_prefers_video_plus_audio_components() {
        let stats = XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            inbound_bitrate_kbps: Some(128.0),
            inbound_video_bitrate_kbps: Some(8_800.0),
            inbound_audio_bitrate_kbps: Some(160.0),
            ..XbxEngineMediaRuntimeStats::default()
        };

        let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
        assert_eq!(dto.inbound_video_bitrate_kbps, Some(8_800.0));
        assert_eq!(dto.inbound_audio_bitrate_kbps, Some(160.0));
        assert_eq!(dto.inbound_bitrate_kbps, Some(8_960.0));
    }

    #[test]
    fn bwe_and_twcc_semantic_fields_are_projected_explicitly() {
        let stats = XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            video_remb_bps: Some(25_000_000),
            latest_video_bwe_observation: Some(crate::XbxEngineVideoBweObservation {
                observation_id: 7,
                mode: "twcc-gcc".to_string(),
                decision_reason: "twcc-gcc-cloud-stable-ramp".to_string(),
                target_remb_kbps: 25_000,
                observed_remb_kbps: Some(23_000),
                actual_video_bitrate_kbps: 21_500.0,
                loss_ratio: 0.01,
                rtt_ms: Some(82.0),
                transport_path: Some("Direct".to_string()),
                twcc_feedback_interval_ms: Some(80.0),
                twcc_observed_packet_count: Some(120),
                twcc_covered_sequence_span: Some(120),
                twcc_receive_bitrate_kbps: Some(22_800.0),
                twcc_delivery_ratio: Some(0.99),
                twcc_loss_ratio: Some(0.01),
                observed_at_ms: 1.0,
            }),
            latest_video_twcc_observation: Some(crate::XbxEngineVideoTwccObservation {
                observation_id: 8,
                source: "local-feedback".to_string(),
                feedback_packet_count: 3,
                covered_sequence_start: 100,
                covered_sequence_end: 220,
                covered_sequence_span: 120,
                observed_packet_count: 120,
                observed_byte_count: 340_000,
                coverage_ratio: Some(1.0),
                ledger_hit_ratio: Some(1.0),
                feedback_interval_ms: Some(80.0),
                arrival_span_ms: Some(70.0),
                receive_bitrate_kbps: Some(22_800.0),
                twcc_sample_valid: true,
                twcc_invalid_reason: None,
                quality: crate::XbxEngineTwccObservationQuality::Stable,
                delivery_ratio: 0.99,
                packet_loss_ratio: 0.01,
                observed_at_ms: 2.0,
            }),
            ..XbxEngineMediaRuntimeStats::default()
        };

        let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
        assert_eq!(dto.video_bwe_mode.as_deref(), Some("twcc-gcc"));
        assert_eq!(dto.video_target_remb_kbps, Some(25_000));
        assert_eq!(dto.video_observed_remb_kbps, Some(23_000));
        assert_eq!(dto.video_actual_bitrate_kbps, None);
        assert_eq!(dto.video_twcc_receive_bitrate_kbps, Some(22_800.0));
        assert_eq!(dto.video_twcc_loss_ratio, Some(0.01));
        assert_eq!(dto.video_twcc_delivery_ratio, Some(0.99));
        assert_eq!(dto.video_twcc_feedback_interval_ms, Some(80.0));
        assert_eq!(
            dto.actual_video_bitrate_source.as_deref(),
            Some("unavailable")
        );
        assert_eq!(
            dto.twcc_observation_state.as_deref(),
            Some("local-feedback")
        );
    }

    #[test]
    fn actual_video_bitrate_uses_transport_metrics_when_local_twcc_missing() {
        let stats = XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            inbound_video_bitrate_kbps: Some(8_600.0),
            latest_video_twcc_observation: Some(crate::XbxEngineVideoTwccObservation {
                observation_id: 8,
                source: "remote-rtcp".to_string(),
                feedback_packet_count: 3,
                covered_sequence_start: 100,
                covered_sequence_end: 220,
                covered_sequence_span: 120,
                observed_packet_count: 120,
                observed_byte_count: 340_000,
                coverage_ratio: Some(1.0),
                ledger_hit_ratio: None,
                feedback_interval_ms: Some(80.0),
                arrival_span_ms: Some(70.0),
                receive_bitrate_kbps: Some(22_800.0),
                twcc_sample_valid: true,
                twcc_invalid_reason: None,
                quality: crate::XbxEngineTwccObservationQuality::RemoteObserved,
                delivery_ratio: 0.99,
                packet_loss_ratio: 0.01,
                observed_at_ms: 2.0,
            }),
            latest_twcc_remote_stream_observation: Some(
                crate::XbxEngineTwccRemoteStreamObservation {
                    observation_id: 11,
                    ssrc: 42,
                    mime_type: "video/H264".to_string(),
                    twcc_ext_id: Some(7),
                    header_extensions: vec!["transport-cc#7".to_string()],
                    rtcp_feedback: vec!["transport-cc:".to_string()],
                    observed_at_ms: 3.0,
                },
            ),
            ..XbxEngineMediaRuntimeStats::default()
        };

        let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
        assert_eq!(dto.video_actual_bitrate_kbps, Some(8_600.0));
        assert_eq!(
            dto.actual_video_bitrate_source.as_deref(),
            Some("transport-metrics")
        );
        assert_eq!(
            dto.twcc_observation_state.as_deref(),
            Some("remote-observed")
        );
    }

    #[test]
    fn transport_details_fields_are_projected_from_runtime_stats() {
        let stats = XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            transport_path: Some("Direct (host->srflx)".to_string()),
            transport_candidate_pair: Some("host->srflx".to_string()),
            transport_protocol: Some("UDP".to_string()),
            transport_address_family: Some("ipv4".to_string()),
            ..XbxEngineMediaRuntimeStats::default()
        };

        let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
        assert_eq!(dto.transport_path.as_deref(), Some("Direct (host->srflx)"));
        assert_eq!(dto.transport_candidate_pair.as_deref(), Some("host->srflx"));
        assert_eq!(dto.transport_protocol.as_deref(), Some("UDP"));
        assert_eq!(dto.transport_address_family.as_deref(), Some("ipv4"));
    }

    #[test]
    fn actual_video_bitrate_uses_transport_metrics_when_local_twcc_is_guarded() {
        let stats = XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            inbound_video_bitrate_kbps: Some(8_600.0),
            latest_video_twcc_observation: Some(crate::XbxEngineVideoTwccObservation {
                observation_id: 9,
                source: "local-feedback".to_string(),
                feedback_packet_count: 3,
                covered_sequence_start: 100,
                covered_sequence_end: 220,
                covered_sequence_span: 120,
                observed_packet_count: 6,
                observed_byte_count: 0,
                coverage_ratio: Some(0.05),
                ledger_hit_ratio: Some(0.0),
                feedback_interval_ms: Some(900.0),
                arrival_span_ms: Some(120.0),
                receive_bitrate_kbps: None,
                twcc_sample_valid: false,
                twcc_invalid_reason: Some("missing-byte-ledger|sample-too-small".to_string()),
                quality: crate::XbxEngineTwccObservationQuality::Delayed,
                delivery_ratio: 1.0,
                packet_loss_ratio: 0.0,
                observed_at_ms: 2.0,
            }),
            ..XbxEngineMediaRuntimeStats::default()
        };

        let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
        assert_eq!(dto.video_actual_bitrate_kbps, Some(8_600.0));
        assert_eq!(
            dto.actual_video_bitrate_source.as_deref(),
            Some("transport-metrics")
        );
    }

    #[test]
    fn twcc_state_marks_missing_header_extension_when_feedback_chain_has_not_started() {
        let stats = XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            latest_rtc_builder_observation: Some(crate::XbxEngineRtcBuilderObservation {
                observation_id: 1,
                controlled_twcc_registry: true,
                feedback_interval_ms: 1_000.0,
                registered_header_extensions: vec!["video:transport-cc".to_string()],
                registered_rtcp_feedback: vec!["video:transport-cc".to_string()],
                observed_at_ms: 1.0,
            }),
            latest_twcc_remote_stream_observation: Some(
                crate::XbxEngineTwccRemoteStreamObservation {
                    observation_id: 2,
                    ssrc: 42,
                    mime_type: "video/H264".to_string(),
                    twcc_ext_id: Some(7),
                    header_extensions: vec!["transport-cc#7".to_string()],
                    rtcp_feedback: vec!["transport-cc:".to_string()],
                    observed_at_ms: 2.0,
                },
            ),
            latest_twcc_extension_observation: Some(crate::XbxEngineTwccExtensionObservation {
                observation_id: 3,
                state: "missing".to_string(),
                ssrc: 42,
                sequence_number: 99,
                expected_ext_id: 7,
                packet_seen_count: 1,
                missing_count: 1,
                observed_at_ms: 3.0,
            }),
            ..XbxEngineMediaRuntimeStats::default()
        };

        let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
        assert_eq!(dto.video_actual_bitrate_kbps, None);
        assert_eq!(
            dto.actual_video_bitrate_source.as_deref(),
            Some("unavailable")
        );
        assert_eq!(
            dto.twcc_observation_state.as_deref(),
            Some("missing-header-extension")
        );
    }

    #[test]
    fn twcc_state_stays_builder_configured_when_only_audio_remote_binding_exists() {
        let stats = XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            latest_rtc_builder_observation: Some(crate::XbxEngineRtcBuilderObservation {
                observation_id: 1,
                controlled_twcc_registry: true,
                feedback_interval_ms: 1_000.0,
                registered_header_extensions: vec!["video:transport-cc".to_string()],
                registered_rtcp_feedback: vec!["video:transport-cc".to_string()],
                observed_at_ms: 1.0,
            }),
            latest_twcc_remote_stream_observation: Some(
                crate::XbxEngineTwccRemoteStreamObservation {
                    observation_id: 2,
                    ssrc: 99,
                    mime_type: "audio/opus".to_string(),
                    twcc_ext_id: Some(7),
                    header_extensions: vec!["transport-cc#7".to_string()],
                    rtcp_feedback: vec!["transport-cc:".to_string()],
                    observed_at_ms: 2.0,
                },
            ),
            ..XbxEngineMediaRuntimeStats::default()
        };

        let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
        assert_eq!(
            dto.twcc_observation_state.as_deref(),
            Some("builder-configured")
        );
    }

    #[test]
    fn stats_expose_cloud_startup_dynamic_subprofile() {
        let stats = XbxEngineMediaRuntimeStats {
            session_target_type: Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud),
            session_phase: Some("startup".to_string()),
            direct_gaming_bitrate_band: Some("startupLow".to_string()),
            ..XbxEngineMediaRuntimeStats::default()
        };

        let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
        assert_eq!(dto.remote_profile_baseline.as_deref(), Some("cloudGaming"));
        assert_eq!(dto.remote_profile_dynamic.as_deref(), Some("cloudStartup"));
        assert_eq!(
            dto.remote_profile_effective_label.as_deref(),
            Some("cloudGaming+cloudStartup")
        );
    }

    #[test]
    fn stats_expose_display_constrained_dynamic_subprofile() {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as f64;
        let stats = XbxEngineMediaRuntimeStats {
            session_target_type: Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud),
            host_no_pending_pressure_level: Some("critical".to_string()),
            latest_video_decode_ok_time_ms: Some(now_ms - 1_500.0),
            latest_video_host_present_time_ms: Some(now_ms - 1_500.0),
            ..XbxEngineMediaRuntimeStats::default()
        };

        let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
        assert_eq!(
            dto.remote_profile_dynamic.as_deref(),
            Some("displayConstrained")
        );
        assert_eq!(
            dto.remote_profile_effective_label.as_deref(),
            Some("cloudGaming+displayConstrained")
        );
    }

    #[test]
    fn stats_prioritize_runtime_remote_profile_facts_when_present() {
        let stats = XbxEngineMediaRuntimeStats {
            session_target_type: Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud),
            session_phase: Some("startup".to_string()),
            direct_gaming_bitrate_band: Some("startupLow".to_string()),
            baseline_remote_profile: Some("relayGaming".to_string()),
            dynamic_remote_subprofile: Some("steady".to_string()),
            effective_remote_profile_label: Some("relayGaming+steady".to_string()),
            ..XbxEngineMediaRuntimeStats::default()
        };

        let dto = build_xbxengine_stats(&test_snapshot(), Some(&stats));
        assert_eq!(dto.remote_profile_baseline.as_deref(), Some("relayGaming"));
        assert_eq!(dto.remote_profile_dynamic.as_deref(), Some("steady"));
        assert_eq!(
            dto.remote_profile_effective_label.as_deref(),
            Some("relayGaming+steady")
        );
    }
}
