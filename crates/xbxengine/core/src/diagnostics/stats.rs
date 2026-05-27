use xbxengine_protocol::{
    XbxEnginePresentationMilestoneDto, XbxEngineStatsDto, XbxEngineTransportStateDto,
};

use crate::transport::rtc::recovery::escalation_label::escalation_structured_label;
use crate::transport::rtc::recovery::remote_profile_runtime::classify_runtime_remote_profile;
use crate::transport::rtc::recovery::runtime_state::{
    project_runtime_state_from_stats, renderer_shadow_blocks_serviceability,
    resolve_runtime_recovery_profile, RecoveryRuntimeState,
};
use crate::transport::rtc::recovery::startup::SessionPhase;
use crate::{
    XbxEngineFirstFrameLatencyObservation, XbxEngineH264InspectionObservation,
    XbxEngineKeyframeRequestEpisodeObservation, XbxEngineMediaRuntimeStats,
    XbxEnginePictureRecoveryBlockerObservation, XbxEnginePictureRecoveryTransitionObservation,
    XbxEngineRuntimeSnapshot, XbxEngineVideoIngressTerminationObservation,
};

fn resolve_panel_fps(stats: &XbxEngineMediaRuntimeStats) -> f64 {
    if stats.video_present_fps > 0.0 {
        return stats.video_present_fps;
    }
    if stats.video_decode_fps > 0.0 {
        return stats.video_decode_fps;
    }
    if stats.inbound_video_frame_rate_fps > 0.0 {
        return stats.inbound_video_frame_rate_fps;
    }
    stats
        .latest_video_frame
        .as_ref()
        .map(|frame| frame.fps)
        .unwrap_or(0.0)
}

fn keyframe_request_episode_to_protocol_dto(
    episode: &XbxEngineKeyframeRequestEpisodeObservation,
) -> xbxengine_protocol::XbxEngineKeyframeRequestEpisodeObservationDto {
    let family_id = episode.request_reason.as_ref().map(|reason| {
        let kind = episode.request_kind.as_deref().unwrap_or("unknown");
        format!("{reason}:{kind}")
    });
    let suppressed = matches!(episode.status.as_str(), "deferred" | "failed");
    xbxengine_protocol::XbxEngineKeyframeRequestEpisodeObservationDto {
        episode_id: episode.episode_id,
        request_reason: episode.request_reason.clone(),
        request_kind: episode.request_kind.clone(),
        status: episode.status.clone(),
        status_detail: episode.status_detail.clone(),
        requested_at_ms: episode.requested_at_ms,
        sent_at_ms: episode.sent_at_ms,
        deadline_at_ms: episode.deadline_at_ms,
        transport_detail: episode.transport_detail.clone(),
        first_video_packet_at_ms: episode.first_video_packet_at_ms,
        first_video_packet_rtp_timestamp: episode.first_video_packet_rtp_timestamp,
        first_video_packet_is_keyframe: episode.first_video_packet_is_keyframe,
        first_keyframe_packet_at_ms: episode.first_keyframe_packet_at_ms,
        first_keyframe_decoded_at_ms: episode.first_keyframe_decoded_at_ms,
        response_rtp_timestamp: episode.response_rtp_timestamp,
        response_frame_seq: episode.response_frame_seq,
        response_verdict: episode.response_verdict.clone(),
        lifecycle_phase: episode.lifecycle_phase.clone(),
        retired_at_ms: episode.retired_at_ms,
        family_id: suppressed.then_some(family_id).flatten(),
        owner_episode_id: suppressed.then_some(episode.episode_id),
        suppress_duration_ms: suppressed.then_some(
            episode
                .sent_at_ms
                .map(|sent_at_ms| (sent_at_ms - episode.requested_at_ms).max(0.0))
                .unwrap_or(0.0),
        ),
        release_reason: suppressed.then(|| {
            episode
                .transport_detail
                .clone()
                .or_else(|| episode.status_detail.clone())
                .unwrap_or_else(|| "suppressed".to_string())
        }),
    }
}

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

fn should_project_recent_nack_owner_fallback(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> Option<VideoOwnerContract> {
    let nack_obs = stats.latest_video_nack_observation.as_ref()?;
    if !matches!(
        nack_obs.action.as_str(),
        "expiredDeadline" | "expiredMaxAge"
    ) {
        return None;
    }
    if (now_ms - nack_obs.observed_at_ms).max(0.0) > crate::session::recovery::NACK_RECENT_GRACE_MS
    {
        return None;
    }
    let escalation_reason = stats.recovery_active_escalation_reason.as_ref()?;
    if !escalation_reason.starts_with("transport") {
        return None;
    }
    if stats
        .video_anchor_clean_observed_at_ms
        .is_some_and(|anchor_at_ms| anchor_at_ms >= nack_obs.observed_at_ms)
    {
        return None;
    }
    if stats
        .latest_video_decode_ok_time_ms
        .is_some_and(|decode_at_ms| decode_at_ms >= nack_obs.observed_at_ms)
    {
        return None;
    }
    if stats
        .latest_video_host_present_time_ms
        .is_some_and(|present_at_ms| present_at_ms >= nack_obs.observed_at_ms)
    {
        return None;
    }
    Some(VideoOwnerContract {
        state: "recovering".to_string(),
        reason: Some(escalation_reason.clone()),
        source: Some("nack".to_string()),
        observed_at_ms: Some(nack_obs.observed_at_ms),
    })
}

fn map_internal_owner_state_to_contract_state(internal_state: &str) -> Option<String> {
    Some(
        match internal_state {
            "stable-serving" | "degraded-serving" => "playing",
            "rebuilding-supply" => "waitingKeyframe",
            "supply-starved" => "displayStalled",
            "seeking-anchor" | "priming" => "starting",
            _ => return None,
        }
        .to_string(),
    )
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
        if let Some(owner) = should_project_recent_nack_owner_fallback(stats, now_ms_f64()) {
            return Some(owner);
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

fn map_presentation_milestone(
    milestone: Option<&XbxEnginePresentationMilestoneDto>,
) -> Option<String> {
    milestone
        .map(|value| match value {
            XbxEnginePresentationMilestoneDto::Idle => "idle",
            XbxEnginePresentationMilestoneDto::Connected => "connected",
            XbxEnginePresentationMilestoneDto::MediaReady => "mediaReady",
            XbxEnginePresentationMilestoneDto::Degraded => "degraded",
            XbxEnginePresentationMilestoneDto::Failed => "failed",
            XbxEnginePresentationMilestoneDto::Closed => "closed",
        })
        .map(str::to_string)
}

fn should_project_runtime_owner_fallback(stats: &XbxEngineMediaRuntimeStats) -> bool {
    matches!(
        stats.session_phase.as_deref(),
        Some(
            "recovering"
                | "observing"
                | "local-self-healing"
                | "recovery-eligible"
                | "active-recovery"
                | "recovery-blocked"
        )
    ) || (stats.message_handshake_acked_at_ms.is_some()
        && stats.control_ready_at_ms.is_some()
        && has_visible_video_output(stats))
}

fn map_owner_state_to_video_health(owner_state: &str, owner_reason: Option<&str>) -> String {
    if owner_state == "supply-starved" && owner_reason == Some("hostPresentStalled") {
        return "hostPresentStalled".to_string();
    }
    match owner_state {
        "seeking-anchor" | "priming" => "priming".to_string(),
        "stable-serving" | "degraded-serving" => "healthy".to_string(),
        "rebuilding-supply" => "recovering".to_string(),
        "supply-starved" => "displaySupplyStarved".to_string(),
        other => other.to_string(),
    }
}

fn resolve_presentation_health(
    runtime_stats: Option<&XbxEngineMediaRuntimeStats>,
    chain_health: Option<&str>,
    present_age_ms: Option<f64>,
) -> Option<String> {
    let stats = runtime_stats?;
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0);
    if stats.video_owner_reason.as_deref() == Some("hostPresentStalled") {
        return Some("hostPresentStalled".to_string());
    }
    let has_present_history = stats.latest_video_host_present_time_ms.is_some()
        || stats.host_mailbox_enqueue_count_total > 0;
    let supply_pressure = matches!(
        stats.host_no_pending_pressure_level.as_deref(),
        Some("high" | "critical")
    );
    let present_stale = present_age_ms.is_some_and(|age| age >= 600.0);
    if has_present_history && (supply_pressure || present_stale) {
        if recovery_progress_is_still_serviceable(stats, now_ms) {
            return chain_health
                .map(str::to_string)
                .or(Some("recovering".to_string()));
        }
        return Some("displaySupplyStarved".to_string());
    }
    if renderer_shadow_blocks_serviceability(stats, now_ms) {
        return Some("displaySupplyStarved".to_string());
    }
    if stats.latest_video_host_present_time_ms.is_some() {
        return Some("healthy".to_string());
    }
    chain_health.map(str::to_string)
}

fn recovery_progress_is_still_serviceable(stats: &XbxEngineMediaRuntimeStats, now_ms: f64) -> bool {
    const RECOVERY_PROGRESS_SERVICEABLE_WINDOW_MS: f64 = 480.0;
    if !matches!(
        stats.session_phase.as_deref(),
        Some("recovering" | "recovery-eligible" | "active-recovery" | "recovery-blocked")
    ) {
        return false;
    }
    if stats.transport_state != XbxEngineTransportStateDto::Connected {
        return false;
    }
    if stats.video_decoder_stalled == Some(true) {
        return false;
    }
    stats
        .latest_video_decode_ok_time_ms
        .is_some_and(|at_ms| (now_ms - at_ms).max(0.0) <= RECOVERY_PROGRESS_SERVICEABLE_WINDOW_MS)
        || stats
            .latest_host_mailbox_submit_time_ms
            .is_some_and(|at_ms| {
                (now_ms - at_ms).max(0.0) <= RECOVERY_PROGRESS_SERVICEABLE_WINDOW_MS
            })
}

/// inspection reject 簇与 submit 尾延迟尖峰同现，便于 trace 归因控制面脉冲。
fn resolve_inspection_pulse_active(
    stats: &crate::XbxEngineMediaRuntimeStats,
    submit_age_ms: Option<f64>,
) -> bool {
    let inspection_pulse = stats
        .latest_h264_inspection_observation
        .as_ref()
        .is_some_and(|observation| !observation.admission_accepted);
    let submit_spike = submit_age_ms.is_some_and(|age_ms| age_ms >= 200.0);
    inspection_pulse && submit_spike
}

fn merge_video_health(
    chain_health: Option<&str>,
    presentation_health: Option<&str>,
) -> Option<String> {
    if matches!(
        presentation_health,
        Some("displaySupplyStarved" | "hostPresentStalled")
    ) {
        return presentation_health.map(str::to_string);
    }
    chain_health
        .map(str::to_string)
        .or_else(|| presentation_health.map(str::to_string))
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
    observation: Option<&XbxEngineH264InspectionObservation>,
) -> Option<xbxengine_protocol::XbxEngineH264InspectionObservationDto> {
    observation.map(
        |observation| xbxengine_protocol::XbxEngineH264InspectionObservationDto {
            observation_id: observation.observation_id,
            frame_rtp_timestamp: observation.frame_rtp_timestamp,
            nal_types: observation.nal_types.clone(),
            nal_count: observation.nal_count,
            vcl_nal_count: observation.vcl_nal_count,
            has_inband_sps: observation.has_inband_sps,
            has_inband_pps: observation.has_inband_pps,
            committed_sps_present: observation.committed_sps_present,
            committed_pps_present: observation.committed_pps_present,
            slice_headers_valid: observation.slice_headers_valid,
            delta_continuation_ready: observation.delta_continuation_ready,
            parameter_sets_changed: observation.parameter_sets_changed,
            config_changed: observation.config_changed,
            is_idr: observation.is_idr,
            sample_width: observation.sample_width,
            sample_height: observation.sample_height,
            bootstrap_ready: observation.bootstrap_ready,
            bootstrap_reject_reason: observation.bootstrap_reject_reason.clone(),
            continuation_verdict: observation.continuation_verdict.clone(),
            admission_accepted: observation.admission_accepted,
            observed_at_ms: observation.observed_at_ms,
            bound_episode_id: observation.bound_episode_id,
            bound_episode_status: observation.bound_episode_status.clone(),
            bound_as_recovery_response: observation.bound_as_recovery_response,
            bound_response_rtp_timestamp: observation.bound_response_rtp_timestamp,
            bound_recovery_epoch: observation.bound_recovery_epoch,
            episode_phase_at_observation: observation.episode_phase_at_observation.clone(),
            is_post_recovery_degradation: observation.is_post_recovery_degradation,
            reject_classification: observation.reject_classification.clone(),
        },
    )
}

fn picture_recovery_transition_dto_from_observation(
    observation: Option<&XbxEnginePictureRecoveryTransitionObservation>,
) -> Option<xbxengine_protocol::XbxEnginePictureRecoveryTransitionObservationDto> {
    observation.map(|observation| {
        xbxengine_protocol::XbxEnginePictureRecoveryTransitionObservationDto {
            observation_id: observation.observation_id,
            episode_id: observation.episode_id,
            recovery_epoch: observation.recovery_epoch,
            phase: observation.phase.clone(),
            from_phase: observation.from_phase.clone(),
            to_phase: observation.to_phase.clone(),
            cause: observation.cause.clone(),
            detail: observation.detail.clone(),
            rtp_timestamp: observation.rtp_timestamp,
            frame_seq: observation.frame_seq,
            owner_state: observation.owner_state.clone(),
            transport_state: observation.transport_state.clone(),
            observed_at_ms: observation.observed_at_ms,
        }
    })
}

fn picture_recovery_blocker_dto_from_observation(
    observation: Option<&XbxEnginePictureRecoveryBlockerObservation>,
) -> Option<xbxengine_protocol::XbxEnginePictureRecoveryBlockerObservationDto> {
    observation.map(|observation| {
        xbxengine_protocol::XbxEnginePictureRecoveryBlockerObservationDto {
            observation_id: observation.observation_id,
            episode_id: observation.episode_id,
            recovery_epoch: observation.recovery_epoch,
            gate: observation.gate.clone(),
            blocker_kind: observation.blocker_kind.clone(),
            severity: observation.severity.clone(),
            first_observed_at_ms: observation.first_observed_at_ms,
            observed_at_ms: observation.observed_at_ms,
            count: observation.count,
            frame_rtp_timestamp: observation.frame_rtp_timestamp,
            frame_seq: observation.frame_seq,
            owner_state: observation.owner_state.clone(),
            transport_state: observation.transport_state.clone(),
        }
    })
}

fn video_ingress_termination_dto_from_observation(
    observation: Option<&XbxEngineVideoIngressTerminationObservation>,
) -> Option<xbxengine_protocol::XbxEngineVideoIngressTerminationObservationDto> {
    observation.map(|observation| {
        xbxengine_protocol::XbxEngineVideoIngressTerminationObservationDto {
            observation_id: observation.observation_id,
            termination_id: observation.termination_id,
            derived_from_termination_id: observation.derived_from_termination_id,
            kind: observation.kind.clone(),
            cause: observation.cause.clone(),
            upstream_cause: observation.upstream_cause.clone(),
            source_subsystem: observation.source_subsystem.clone(),
            linked_recovery_epoch: observation.linked_recovery_epoch,
            linked_episode_id: observation.linked_episode_id,
            transport_state: observation.transport_state.clone(),
            owner_state: observation.owner_state.clone(),
            video_track_state: observation.video_track_state.clone(),
            recent_command: observation.recent_command.clone(),
            observed_at_ms: observation.observed_at_ms,
        }
    })
}

fn first_frame_latency_dto_from_observation(
    observation: Option<&XbxEngineFirstFrameLatencyObservation>,
) -> Option<xbxengine_protocol::XbxEngineFirstFrameLatencyObservationDto> {
    observation.map(
        |observation| xbxengine_protocol::XbxEngineFirstFrameLatencyObservationDto {
            observation_id: observation.observation_id,
            episode_id: observation.episode_id,
            recovery_epoch: observation.recovery_epoch,
            control_ready_to_pli_sent_ms: observation.control_ready_to_pli_sent_ms,
            pli_sent_to_first_idr_packet_ms: observation.pli_sent_to_first_idr_packet_ms,
            first_idr_packet_to_first_decode_ms: observation.first_idr_packet_to_first_decode_ms,
            first_decode_to_clean_anchor_committed_ms: observation
                .first_decode_to_clean_anchor_committed_ms,
            clean_anchor_committed_to_display_stable_ms: observation
                .clean_anchor_committed_to_display_stable_ms,
            terminal_phase: observation.terminal_phase.clone(),
            incomplete_reason: observation.incomplete_reason.clone(),
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
            "referenceChainUnrecoverable" | "awaitingRecoveryAnchor" | "chainBroken"
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
        Some("waitKeyframe" | "awaitingRecoveryAnchor")
    ) {
        return "wait-anchor".to_string();
    }
    "local-drop".to_string()
}

fn normalize_budget_recovery_stage(failure_cost: &str, window_source: &str) -> String {
    match (failure_cost, window_source) {
        ("chain-broken" | "wait-anchor", _) => "awaiting-anchor".to_string(),
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
    let window_source = if drop
        .detail
        .as_deref()
        .is_some_and(|detail| matches!(detail, "outputQueueOverflow"))
    {
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
        observation.frame_recovery_disposition.as_deref(),
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
    runtime_state: Option<&RecoveryRuntimeState>,
    owner: Option<&VideoOwnerContract>,
) -> Option<String> {
    owner
        .and_then(|owner| owner.reason.clone())
        .or_else(|| runtime_state.map(|state| state.diagnosis_label.clone()))
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
    let submit_age_ms = runtime_stats
        .and_then(|stats| stats.latest_host_mailbox_submit_time_ms)
        .map(|at| (now_ms - at).max(0.0));
    let audio_playout_latency_ms = runtime_stats.and_then(resolve_audio_playout_latency_ms);
    let audio_video_playout_delta_ms =
        runtime_stats.and_then(|stats| resolve_audio_video_playout_delta_ms(stats, present_age_ms));
    let packet_to_decode_ms = runtime_stats.and_then(|stats| {
        stats
            .latest_video_packet_arrival_rtp_timestamp
            .zip(stats.latest_video_decode_ok_rtp_timestamp)
            .filter(|(packet_rtp, decode_rtp)| packet_rtp == decode_rtp)
            .and_then(|_| {
                stats
                    .latest_video_packet_arrival_time_ms
                    .zip(stats.latest_video_decode_ok_time_ms)
                    .map(|(packet_at_ms, decode_at_ms)| (decode_at_ms - packet_at_ms).max(0.0))
            })
    });
    let decode_to_present_ms = runtime_stats.and_then(|stats| {
        stats
            .latest_video_decode_ok_rtp_timestamp
            .zip(stats.last_displayed_frame_rtp_timestamp)
            .filter(|(decode_rtp, displayed_rtp)| decode_rtp == displayed_rtp)
            .and_then(|_| {
                stats
                    .latest_video_decode_ok_time_ms
                    .zip(
                        stats
                            .last_displayed_at_ms
                            .or(stats.latest_video_host_present_time_ms),
                    )
                    .map(|(decode_at_ms, present_at_ms)| (present_at_ms - decode_at_ms).max(0.0))
            })
    });
    let submit_to_present_ms = runtime_stats.and_then(|stats| {
        stats
            .latest_host_mailbox_submit_time_ms
            .zip(
                stats
                    .last_displayed_at_ms
                    .or(stats.latest_video_host_present_time_ms),
            )
            .map(|(submit_at_ms, present_at_ms)| (present_at_ms - submit_at_ms).max(0.0))
    });
    let inspection_pulse_active =
        runtime_stats.map(|stats| resolve_inspection_pulse_active(stats, submit_age_ms));
    let packet_to_present_ms = runtime_stats.and_then(|stats| {
        stats
            .latest_video_packet_arrival_rtp_timestamp
            .zip(stats.last_displayed_frame_rtp_timestamp)
            .filter(|(packet_rtp, displayed_rtp)| packet_rtp == displayed_rtp)
            .and_then(|_| {
                stats
                    .latest_video_packet_arrival_time_ms
                    .zip(
                        stats
                            .last_displayed_at_ms
                            .or(stats.latest_video_host_present_time_ms),
                    )
                    .map(|(packet_at_ms, present_at_ms)| (present_at_ms - packet_at_ms).max(0.0))
            })
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
    let fps = runtime_stats.map(resolve_panel_fps).unwrap_or_default();
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
    let lifecycle_phase = classify_unified_lifecycle(
        runtime_stats,
        recovery_runtime_state.as_ref(),
        video_owner,
        now_ms,
    );
    let presentation_milestone =
        map_presentation_milestone(snapshot.presentation_milestone.as_ref());
    let connected_milestone_elapsed_ms = snapshot
        .connected_milestone_at_ms
        .map(|at| (now_ms - at).max(0.0));
    let media_ready_milestone_elapsed_ms = snapshot
        .media_ready_milestone_at_ms
        .map(|at| (now_ms - at).max(0.0));
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
        resolve_recovery_diagnosis(recovery_runtime_state.as_ref(), video_owner);
    let recovery_rfc_fault_domain =
        runtime_stats.and_then(|s| s.recovery_rfc_authoritative_fault_domain.clone());
    let recovery_rfc_stage = runtime_stats.and_then(|s| s.recovery_rfc_authoritative_stage.clone());
    let recovery_rfc_ceiling =
        runtime_stats.and_then(|s| s.recovery_rfc_authoritative_ceiling.clone());
    let renderer_stall_blocks_presentation = runtime_stats.map(|stats| {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis() as f64)
            .unwrap_or(0.0);
        renderer_shadow_blocks_serviceability(stats, now_ms)
    });
    let chain_health = video_owner.as_ref().map(|owner| {
        map_owner_state_to_video_health(owner.state.as_str(), owner.reason.as_deref())
    });
    let presentation_health =
        resolve_presentation_health(runtime_stats, chain_health.as_deref(), present_age_ms);
    let video_health = merge_video_health(chain_health.as_deref(), presentation_health.as_deref());
    let observation_note = build_observation_note(runtime_stats);
    let transport_recovery_note = build_transport_recovery_note(runtime_stats);
    let repair_probe_note = build_repair_probe_note(runtime_stats);
    let reinject_note = build_rtx_reinject_note(runtime_stats);
    let summary_phase =
        resolve_runtime_summary_phase(display_phase.as_deref(), lifecycle_phase.as_deref());
    let runtime_summary = build_runtime_summary(
        runtime_stats,
        remote_profile_effective_label.as_deref(),
        remote_profile_baseline.as_deref(),
        video_owner,
        summary_phase,
        video_health.as_deref(),
        observation_note.as_deref(),
        transport_recovery_note.as_deref(),
        repair_probe_note.as_deref(),
        reinject_note.as_deref(),
    );
    let primary_issue_chain = build_primary_issue_chain(
        runtime_stats,
        lifecycle_phase.as_deref(),
        video_owner,
        stall_kind.as_deref(),
    );
    let latest_decision_summary = build_latest_decision_summary(
        runtime_stats,
        lifecycle_phase.as_deref(),
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
    let recovery_owner_contract_state = runtime_stats
        .and_then(|stats| stats.video_owner_contract_state.clone())
        .or_else(|| {
            video_owner
                .as_ref()
                .and_then(|owner| map_internal_owner_state_to_contract_state(owner.state.as_str()))
        });
    let recovery_owner_reason = video_owner.and_then(|owner| owner.reason.clone());

    XbxEngineStatsDto {
        resolution,
        rtt,
        fps,
        runtime_summary,
        presentation_milestone,
        connected_milestone_elapsed_ms,
        media_ready_milestone_elapsed_ms,
        presentation_failed_stage: snapshot.presentation_failed_stage.clone(),
        primary_issue_chain,
        latest_decision_summary,
        remote_profile_baseline,
        remote_profile_dynamic,
        remote_profile_effective_label,
        session_phase: display_phase,
        stream_lifecycle_phase: lifecycle_phase,
        transport_strategy_profile,
        recovery_strategy_profile,
        recovery_diagnosis,
        recovery_rfc_fault_domain,
        recovery_rfc_stage,
        recovery_rfc_ceiling,
        recovery_playback_recovered_at_ms: runtime_stats
            .and_then(|stats| stats.recovery_playback_recovered_at_ms),
        recovery_playback_recovered_phase: runtime_stats
            .and_then(|stats| stats.recovery_playback_recovered_phase.clone()),
        recovery_fresh_anchor_recovered_at_ms: runtime_stats
            .and_then(|stats| stats.recovery_fresh_anchor_recovered_at_ms),
        recovery_displayed_idr_rtp: runtime_stats
            .and_then(|stats| stats.recovery_displayed_idr_rtp),
        recovery_displayed_idr_at_ms: runtime_stats
            .and_then(|stats| stats.recovery_displayed_idr_at_ms),
        recovery_effective_rtt_ms: runtime_stats.and_then(|stats| stats.recovery_effective_rtt_ms),
        recovery_dynamic_nack_timeout_ms: runtime_stats
            .and_then(|stats| stats.recovery_dynamic_nack_timeout_ms),
        recovery_dynamic_nack_retry_interval_ms: runtime_stats
            .and_then(|stats| stats.recovery_dynamic_nack_retry_interval_ms),
        recovery_dynamic_pli_refresh_interval_ms: runtime_stats
            .and_then(|stats| stats.recovery_dynamic_pli_refresh_interval_ms),
        recovery_dynamic_fir_retry_interval_ms: runtime_stats
            .and_then(|stats| stats.recovery_dynamic_fir_retry_interval_ms),
        recovery_dynamic_decoded_pending_commit_hold_ms: runtime_stats
            .and_then(|stats| stats.recovery_dynamic_decoded_pending_commit_hold_ms),
        recovery_dynamic_continuation_patience_ms: runtime_stats
            .and_then(|stats| stats.recovery_dynamic_continuation_patience_ms),
        recovery_dynamic_clean_anchor_patience_ms: runtime_stats
            .and_then(|stats| stats.recovery_dynamic_clean_anchor_patience_ms),
        recovery_codec_bootstrap_salvage_applied: runtime_stats
            .and_then(|stats| stats.recovery_codec_bootstrap_salvage_applied),
        recovery_codec_bootstrap_salvage_failed_reason: runtime_stats
            .and_then(|stats| stats.recovery_codec_bootstrap_salvage_failed_reason.clone()),
        recovery_nack_first_attempt_survival_window_ms: runtime_stats
            .and_then(|stats| stats.recovery_nack_first_attempt_survival_window_ms),
        recovery_nack_first_attempt_deadline_at_ms: runtime_stats
            .and_then(|stats| stats.recovery_nack_first_attempt_deadline_at_ms),
        recovery_nack_first_attempt_still_economical: runtime_stats
            .and_then(|stats| stats.recovery_nack_first_attempt_still_economical),
        recovery_nack_retry_allowed_reason: runtime_stats
            .and_then(|stats| stats.recovery_nack_retry_allowed_reason.clone()),
        recovery_nack_retry_suppressed_reason: runtime_stats
            .and_then(|stats| stats.recovery_nack_retry_suppressed_reason.clone()),
        direct_gaming_bitrate_band: runtime_stats
            .and_then(|stats| stats.direct_gaming_bitrate_band.clone()),
        recovery_owner_state,
        recovery_owner_contract_state,
        recovery_owner_reason,
        recovery_surface_phase: runtime_stats
            .and_then(|stats| stats.recovery_surface_phase.clone()),
        media_supply_phase: runtime_stats.and_then(|stats| stats.media_supply_phase.clone()),
        keyframe_request_outcome_seq: runtime_stats
            .map(|stats| stats.keyframe_request_outcome_seq)
            .unwrap_or(0),
        derived_decoder_health: runtime_stats
            .and_then(|stats| stats.derived_decoder_health.clone()),
        video_owner_source: video_owner.and_then(|owner| owner.source.clone()),
        video_owner_observed_at_ms: video_owner.and_then(|owner| owner.observed_at_ms),
        video_health,
        chain_health,
        presentation_health,
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
        latest_video_packet_arrival_rtp_timestamp: runtime_stats
            .and_then(|stats| stats.latest_video_packet_arrival_rtp_timestamp),
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
        latest_video_decoder_probe_observation: runtime_stats.and_then(|stats| {
            stats
                .latest_video_decoder_probe_observation
                .as_ref()
                .map(
                    |probe| xbxengine_protocol::XbxEngineVideoDecoderProbeObservationDto {
                        observation_id: probe.observation_id,
                        selected_backend_name: probe.selected_backend_name.clone(),
                        selected_backend_kind: probe.selected_backend_kind.clone(),
                        fallback_count: probe.fallback_count,
                        fallback_summary: probe.fallback_summary.clone(),
                        observed_at_ms: probe.observed_at_ms,
                    },
                )
        }),
        latest_video_decoder_bootstrap_gate_observation: runtime_stats.and_then(|stats| {
            stats
                .latest_video_decoder_bootstrap_gate_observation
                .as_ref()
                .map(|observation| {
                    xbxengine_protocol::XbxEngineVideoDecoderBootstrapGateObservationDto {
                        observation_id: observation.observation_id,
                        recovery_state: observation.recovery_state.clone(),
                        frame_rtp_timestamp: observation.frame_rtp_timestamp,
                        is_idr: observation.is_idr,
                        has_inband_sps: observation.has_inband_sps,
                        has_inband_pps: observation.has_inband_pps,
                        committed_sps_present: observation.committed_sps_present,
                        committed_pps_present: observation.committed_pps_present,
                        bootstrap_ready: observation.bootstrap_ready,
                        bootstrap_reject_reason: observation.bootstrap_reject_reason.clone(),
                        observed_at_ms: observation.observed_at_ms,
                    }
                })
        }),
        latest_decode_output_path_observation: runtime_stats.and_then(|stats| {
            stats
                .latest_decode_output_path_observation
                .as_ref()
                .map(
                    |observation| xbxengine_protocol::XbxEngineDecodeOutputPathObservationDto {
                        observation_id: observation.observation_id,
                        verdict: observation.verdict.clone(),
                        detail: observation.detail.clone(),
                        frame_rtp_timestamp: observation.frame_rtp_timestamp,
                        is_keyframe: observation.is_keyframe,
                        status: observation.status,
                        send_packet_status: observation.send_packet_status,
                        receive_frame_status: observation.receive_frame_status,
                        backend_no_output_streak: observation.backend_no_output_streak,
                        input_frames_since_last_decoded: observation
                            .input_frames_since_last_decoded,
                        bootstrap_reject_reason: observation.bootstrap_reject_reason.clone(),
                        observed_at_ms: observation.observed_at_ms,
                    },
                )
        }),
        latest_remote_frame_capture_observation: runtime_stats.and_then(|stats| {
            stats
                .latest_remote_frame_capture_observation
                .as_ref()
                .map(
                    |observation| xbxengine_protocol::XbxEngineRemoteFrameCaptureObservationDto {
                        observation_id: observation.observation_id,
                        trigger: observation.trigger.clone(),
                        backend_name: observation.backend_name.clone(),
                        frame_rtp_timestamp: observation.frame_rtp_timestamp,
                        is_keyframe: observation.is_keyframe,
                        width: observation.width,
                        height: observation.height,
                        payload_bytes: observation.payload_bytes,
                        payload_fingerprint: observation.payload_fingerprint,
                        payload_prefix_hex: observation.payload_prefix_hex.clone(),
                        nal_types: observation.nal_types.clone(),
                        nal_count: observation.nal_count,
                        has_inband_sps: observation.has_inband_sps,
                        has_inband_pps: observation.has_inband_pps,
                        bootstrap_ready: observation.bootstrap_ready,
                        bootstrap_reject_reason: observation.bootstrap_reject_reason.clone(),
                        parameter_sets_changed: observation.parameter_sets_changed,
                        config_changed: observation.config_changed,
                        slice_headers_valid: observation.slice_headers_valid,
                        send_packet_status: observation.send_packet_status,
                        receive_frame_status: observation.receive_frame_status,
                        status: observation.status,
                        backend_no_output_streak: observation.backend_no_output_streak,
                        input_frames_since_last_decoded: observation
                            .input_frames_since_last_decoded,
                        observed_at_ms: observation.observed_at_ms,
                    },
                )
        }),
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
        latest_video_decode_ok_rtp_timestamp: runtime_stats
            .and_then(|stats| stats.latest_video_decode_ok_rtp_timestamp),
        video_renderer_stalled: runtime_stats.and_then(|stats| stats.video_renderer_stalled),
        video_renderer_stall_blocks_presentation: renderer_stall_blocks_presentation,
        packet_age_ms,
        decode_age_ms,
        present_age_ms,
        packet_to_decode_ms,
        decode_to_present_ms,
        submit_to_present_ms,
        packet_to_present_ms,
        inspection_pulse_active,
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
        host_mailbox_drop_count_total: runtime_stats
            .map(|stats| stats.host_mailbox_drop_count_total),
        host_mailbox_overwrite_count_total: runtime_stats
            .map(|stats| stats.host_mailbox_overwrite_count_total),
        host_mailbox_enqueue_count_total: runtime_stats
            .map(|stats| stats.host_mailbox_enqueue_count_total),
        host_no_pending_take_count_total: runtime_stats
            .map(|stats| stats.host_no_pending_take_count_total),
        host_no_pending_streak: runtime_stats.map(|stats| stats.host_no_pending_streak),
        host_no_pending_max_streak: runtime_stats.map(|stats| stats.host_no_pending_max_streak),
        host_no_pending_pressure_level: runtime_stats
            .and_then(|stats| stats.host_no_pending_pressure_level.clone()),
        host_mailbox_submit_epoch: runtime_stats.map(|stats| stats.host_mailbox_submit_epoch),
        host_display_tick_epoch: runtime_stats.map(|stats| stats.host_display_tick_epoch),
        host_frame_present_epoch: runtime_stats.map(|stats| stats.host_frame_present_epoch),
        host_cadence_phase: runtime_stats.and_then(|stats| stats.host_cadence_phase.clone()),
        latest_host_mailbox_submit_time_ms: runtime_stats
            .and_then(|stats| stats.latest_host_mailbox_submit_time_ms),
        latest_video_host_submit_rtp_timestamp: runtime_stats
            .and_then(|stats| stats.latest_video_host_submit_rtp_timestamp),
        latest_video_host_present_time_ms: runtime_stats
            .and_then(|stats| stats.latest_video_host_present_time_ms),
        submit_age_ms,
        display_age_ms: runtime_stats.and_then(|stats| stats.display_age_ms),
        host_view_generation: runtime_stats.map(|stats| stats.host_view_generation),
        latest_host_view_created_at_ms: runtime_stats
            .and_then(|stats| stats.latest_host_view_created_at_ms),
        last_displayed_frame_seq: runtime_stats.and_then(|stats| stats.last_displayed_frame_seq),
        last_displayed_frame_rtp_timestamp: runtime_stats
            .and_then(|stats| stats.last_displayed_frame_rtp_timestamp),
        last_displayed_at_ms: runtime_stats.and_then(|stats| stats.last_displayed_at_ms),
        video_present_descriptor_upload_mode: runtime_stats
            .and_then(|stats| stats.video_present_descriptor_upload_mode.clone()),
        video_present_descriptor_metal_import_count_total: runtime_stats
            .map(|stats| stats.video_present_descriptor_metal_import_count_total),
        video_present_descriptor_cpu_upload_count_total: runtime_stats
            .map(|stats| stats.video_present_descriptor_cpu_upload_count_total),
        latest_feedback_target_availability_state: runtime_stats
            .and_then(|stats| stats.latest_feedback_target_availability_state.clone()),
        latest_feedback_target_availability_reason: runtime_stats
            .and_then(|stats| stats.latest_feedback_target_availability_reason.clone()),
        latest_feedback_target_availability_target: runtime_stats
            .and_then(|stats| stats.latest_feedback_target_availability_target.clone()),
        latest_feedback_target_availability_observed_at_ms: runtime_stats
            .and_then(|stats| stats.latest_feedback_target_availability_observed_at_ms),
        latest_video_rtcp_send_failure_time_ms: runtime_stats
            .and_then(|stats| stats.latest_video_rtcp_send_failure_time_ms),
        latest_video_rtcp_send_failure_reason: runtime_stats
            .and_then(|stats| stats.latest_video_rtcp_send_failure_reason.clone()),
        latest_keyframe_request_episode: runtime_stats.and_then(|stats| {
            stats
                .latest_keyframe_request_episode
                .as_ref()
                .map(keyframe_request_episode_to_protocol_dto)
        }),
        recent_keyframe_request_episodes: runtime_stats
            .map(|stats| {
                stats
                    .recent_keyframe_request_episodes
                    .iter()
                    .map(keyframe_request_episode_to_protocol_dto)
                    .collect()
            })
            .unwrap_or_default(),
        latest_h264_inspection_observation: runtime_stats.and_then(|stats| {
            h264_inspection_dto_from_observation(stats.latest_h264_inspection_observation.as_ref())
        }),
        latest_picture_recovery_transition_observation: runtime_stats.and_then(|stats| {
            picture_recovery_transition_dto_from_observation(
                stats
                    .latest_picture_recovery_transition_observation
                    .as_ref(),
            )
        }),
        latest_picture_recovery_blocker_observation: runtime_stats.and_then(|stats| {
            picture_recovery_blocker_dto_from_observation(
                stats.latest_picture_recovery_blocker_observation.as_ref(),
            )
        }),
        latest_video_ingress_termination_observation: runtime_stats.and_then(|stats| {
            video_ingress_termination_dto_from_observation(
                stats.latest_video_ingress_termination_observation.as_ref(),
            )
        }),
        latest_first_frame_latency_observation: runtime_stats.and_then(|stats| {
            first_frame_latency_dto_from_observation(
                stats.latest_first_frame_latency_observation.as_ref(),
            )
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
        host_present_take_empty_streak: Some(snapshot.host_present_take_empty_streak),
        host_mailbox_latest_submit_at_ms: runtime_stats
            .and_then(|stats| stats.latest_host_mailbox_submit_time_ms)
            .or(snapshot.host_mailbox_latest_submit_at_ms),
        ice_policy_mode: snapshot.ice_policy_mode.clone(),
        ice_policy_digest: snapshot.ice_policy_digest.clone(),
        ice_policy_source: snapshot.ice_policy_source.clone(),
        ice_policy_filtered_count: snapshot.ice_policy_filtered_count,
        ice_policy_derived_count: snapshot.ice_policy_derived_count,
        ice_policy_skipped_by_family_mismatch_count: snapshot
            .ice_policy_skipped_by_family_mismatch_count,
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
                        replacement_decision: decision.replacement_decision.as_ref().map(
                            |replacement| {
                                xbxengine_protocol::XbxEngineReplacementDecisionObservationDto {
                                    dropped_frame_seq: replacement.dropped_frame_seq,
                                    dropped_rtp_timestamp: replacement.dropped_rtp_timestamp,
                                    dropped_presentation_value_role: replacement
                                        .dropped_presentation_value_role
                                        .clone(),
                                    kept_frame_seq: replacement.kept_frame_seq,
                                    kept_rtp_timestamp: replacement.kept_rtp_timestamp,
                                    kept_presentation_value_role: replacement
                                        .kept_presentation_value_role
                                        .clone(),
                                    same_recovery_epoch: replacement.same_recovery_epoch,
                                    same_recovery_owner_chain: replacement
                                        .same_recovery_owner_chain,
                                    supersede_reason: replacement.supersede_reason.clone(),
                                }
                            },
                        ),
                        observed_at_ms: decision.observed_at_ms,
                    }
                })
        }),
        latest_render_mailbox_decision: runtime_stats.and_then(|stats| {
            stats
                .latest_render_mailbox_decision
                .as_ref()
                .map(|decision| {
                    xbxengine_protocol::XbxEnginePipelineCandidateDecisionObservationDto {
                        decision_id: decision.decision_id,
                        state: decision.state.clone(),
                        action: decision.action.clone(),
                        detail: decision.detail.clone(),
                        frame_seq: decision.frame_seq,
                        replacement_decision: decision.replacement_decision.as_ref().map(
                            |replacement| {
                                xbxengine_protocol::XbxEngineReplacementDecisionObservationDto {
                                    dropped_frame_seq: replacement.dropped_frame_seq,
                                    dropped_rtp_timestamp: replacement.dropped_rtp_timestamp,
                                    dropped_presentation_value_role: replacement
                                        .dropped_presentation_value_role
                                        .clone(),
                                    kept_frame_seq: replacement.kept_frame_seq,
                                    kept_rtp_timestamp: replacement.kept_rtp_timestamp,
                                    kept_presentation_value_role: replacement
                                        .kept_presentation_value_role
                                        .clone(),
                                    same_recovery_epoch: replacement.same_recovery_epoch,
                                    same_recovery_owner_chain: replacement
                                        .same_recovery_owner_chain,
                                    supersede_reason: replacement.supersede_reason.clone(),
                                }
                            },
                        ),
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
                    replacement_decision: drop.replacement_decision.as_ref().map(|replacement| {
                        xbxengine_protocol::XbxEngineReplacementDecisionObservationDto {
                            dropped_frame_seq: replacement.dropped_frame_seq,
                            dropped_rtp_timestamp: replacement.dropped_rtp_timestamp,
                            dropped_presentation_value_role: replacement
                                .dropped_presentation_value_role
                                .clone(),
                            kept_frame_seq: replacement.kept_frame_seq,
                            kept_rtp_timestamp: replacement.kept_rtp_timestamp,
                            kept_presentation_value_role: replacement
                                .kept_presentation_value_role
                                .clone(),
                            same_recovery_epoch: replacement.same_recovery_epoch,
                            same_recovery_owner_chain: replacement.same_recovery_owner_chain,
                            supersede_reason: replacement.supersede_reason.clone(),
                        }
                    }),
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
                        frame_value: ledger.frame_value.clone(),
                        gap_severity: ledger.gap_severity.clone(),
                        repairability: ledger.repairability,
                        recovery_episode_stage: ledger.recovery_episode_stage.clone(),
                        recovery_episode_progress_at_ms: ledger.recovery_episode_progress_at_ms,
                        coalescing_mode: ledger.coalescing_mode.clone(),
                        unlock_reason: ledger.unlock_reason.clone(),
                        preempt_reason: ledger.preempt_reason.clone(),
                        recovery_primary_action: ledger.recovery_primary_action.clone(),
                        owner_surface_state: ledger.owner_surface_state.clone(),
                        anchor_evidence: ledger.anchor_evidence.clone(),
                        keyframe_episode_health: ledger.keyframe_episode_health.clone(),
                        escalation_basis: ledger.escalation_basis.clone(),
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
                        trigger_observation_label: ledger.trigger_observation_label.clone(),
                        trigger_observation_summary: ledger.trigger_observation_summary.clone(),
                        command_result: ledger.command_result.clone(),
                        command_detail: ledger.command_detail.clone(),
                        observed_at_ms: ledger.observed_at_ms,
                    },
                )
        }),
        latest_video_receiver_observation: runtime_stats.and_then(|stats| {
            stats
                .latest_video_receiver_observation
                .as_ref()
                .map(
                    |observation| xbxengine_protocol::XbxEngineVideoReceiverObservationDto {
                        observation_id: observation.observation_id,
                        receiver_state: observation.receiver_state.clone(),
                        gap_sequence: observation.gap_sequence,
                        gap_span: observation.gap_span,
                        nack_in_flight: observation.nack_in_flight,
                        keyframe_request_pending: observation.keyframe_request_pending,
                        bootstrap_reject_reason: observation.bootstrap_reject_reason.clone(),
                        observed_at_ms: observation.observed_at_ms,
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
                                budget_importance: gap.budget_importance.clone(),
                                evidence_importance: gap.evidence_importance.clone(),
                                gap_dependency_confidence: gap.gap_dependency_confidence.clone(),
                                observed_at_ms: gap.observed_at_ms,
                            }
                        }),
                        frame: timeline.frame.as_ref().map(|frame| {
                            xbxengine_protocol::XbxEngineVideoTimelineFrameSnapshotDto {
                                state: frame.state.clone(),
                                frame_rtp_timestamp: frame.frame_rtp_timestamp,
                                is_keyframe: frame.is_keyframe,
                                frame_importance: frame.frame_importance.clone(),
                                budget_importance: frame.budget_importance.clone(),
                                evidence_importance: frame.evidence_importance.clone(),
                                close_reason: frame.close_reason.clone(),
                                observed_at_ms: frame.observed_at_ms,
                            }
                        }),
                        chain: xbxengine_protocol::XbxEngineVideoTimelineChainSnapshotDto {
                            state: timeline.chain.state.clone(),
                            reason: timeline.chain.reason.clone(),
                            chain_break_evidence: timeline.chain.chain_break_evidence.clone(),
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
    let surface = stats.recovery_surface_phase.as_deref().unwrap_or("-");
    let decoder_health = stats.derived_decoder_health.as_deref().unwrap_or("-");
    let base =
        format!("{profile}/{phase}/{band}/{owner_state}/{health}/{surface}/{decoder_health}");
    Some(append_runtime_notes(
        base,
        observation_note,
        transport_recovery_note,
        repair_probe_note,
        reinject_note,
    ))
}

fn resolve_runtime_summary_phase<'a>(
    display_phase: Option<&'a str>,
    lifecycle_phase: Option<&'a str>,
) -> Option<&'a str> {
    match lifecycle_phase {
        Some(
            "observing" | "local-self-healing" | "recovery-eligible" | "active-recovery"
            | "recovery-blocked" | "recovering" | "ramp-up" | "degraded" | "failed" | "closed",
        ) => lifecycle_phase,
        _ => display_phase,
    }
}

// 将当前主问题链显式归类，避免每次回归都手工拼 diagnosis/band/health。
fn build_primary_issue_chain(
    runtime_stats: Option<&XbxEngineMediaRuntimeStats>,
    display_phase: Option<&str>,
    video_owner: Option<&VideoOwnerContract>,
    _stall_kind: Option<&str>,
) -> Option<String> {
    let stats = runtime_stats?;
    let recovery_reason = video_owner
        .and_then(|owner| owner.reason.as_deref())
        .or_else(|| escalation_structured_label(stats))
        .unwrap_or("none");
    if let Some(owner) = video_owner {
        match owner.state.as_str() {
            "supply-starved" => {
                return Some(format!(
                    "display:{}",
                    owner.reason.as_deref().unwrap_or("supply-starved")
                ));
            }
            _ => {}
        }
    }
    match display_phase.unwrap_or("unknown") {
        "observing" => return Some(format!("observing:{recovery_reason}")),
        "local-self-healing" => {
            return Some(format!("local-self-healing:{recovery_reason}"));
        }
        "recovery-eligible" => return Some(format!("recovery-eligible:{recovery_reason}")),
        "active-recovery" | "recovering" => {
            return Some(format!("active-recovery:{recovery_reason}"));
        }
        "recovery-blocked" => return Some(format!("recovery-blocked:{recovery_reason}")),
        _ => {}
    }
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
    runtime_stats: Option<&XbxEngineMediaRuntimeStats>,
    display_phase: Option<&str>,
    video_owner: Option<&VideoOwnerContract>,
    _observation_note: Option<&str>,
    _transport_recovery_note: Option<&str>,
    _repair_probe_note: Option<&str>,
    _reinject_note: Option<&str>,
) -> Option<String> {
    let stats = runtime_stats?;
    if let Some(ledger) = stats.latest_recovery_decision_ledger.as_ref() {
        if is_network_session_recovery_decision(ledger) {
            return Some(format!("network_session_recovery:{}", ledger.gate_result));
        }
        if is_local_decoder_maintenance_decision(ledger.action_selected.as_str()) {
            return Some(format!(
                "local_decoder_maintenance:{}:{}",
                ledger.state_after, ledger.action_selected
            ));
        }
        if ledger.gate_result.contains("reconnectGranted:")
            || ledger.gate_result.contains("reconnectBlocked:")
        {
            return Some(format!("reconnect:{}", ledger.gate_result));
        }
        return Some(format!(
            "decision:{}:{}",
            ledger.state_after, ledger.action_selected
        ));
    }
    if let Some(phase) = display_phase {
        if let Some(owner) = video_owner {
            return Some(format!(
                "phase:{}:{}",
                phase,
                owner.reason.as_deref().unwrap_or("none")
            ));
        }
    }
    let owner = video_owner?;
    Some(format!(
        "owner:{}:{}",
        owner.state,
        owner.reason.as_deref().unwrap_or("none")
    ))
}

fn is_network_session_recovery_decision(
    ledger: &crate::XbxEngineRecoveryDecisionLedgerObservation,
) -> bool {
    ledger.action_selected == "requestReconnectCandidate"
        || ledger.gate_result.contains("reconnectGranted:")
        || ledger.gate_result.contains("reconnectBlocked:")
}

fn is_local_decoder_maintenance_decision(action_selected: &str) -> bool {
    matches!(
        action_selected,
        "requestDecoderReset"
            | "requestPli"
            | "coalesced:keyframeInFlight"
            | "coalesced:decoderResetInFlight"
            | "waitForBurst"
            | "waitForDecoderResetBurst"
            | "cooldownSuppressed"
            | "startupGraceSuppressed"
    )
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

fn now_ms_f64() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0)
}

fn build_transport_recovery_note(
    runtime_stats: Option<&XbxEngineMediaRuntimeStats>,
) -> Option<String> {
    use crate::transport::rtc::recovery::contract::{
        recovery_exit_path_from_stats, recovery_exit_trace_await_suffix, RecoveryExitPath,
        RecoveryExitThresholds,
    };

    let stats = runtime_stats?;
    let now_ms = now_ms_f64();
    let mut parts: Vec<String> = Vec::new();
    if let Some(surface) = stats.recovery_surface_phase.as_deref() {
        parts.push(format!("surface:{surface}"));
    }
    if let Some(media) = stats.media_supply_phase.as_deref() {
        parts.push(format!("mediaSupply:{media}"));
    }
    if let Some(health) = stats.derived_decoder_health.as_deref() {
        parts.push(format!("decoderHealth:{health}"));
    }
    if stats.transport_recovery_epoch > 0 {
        if stats.transport_recovery_episode_active {
            parts.push(format!("repoch:{}:active", stats.transport_recovery_epoch));
        } else {
            parts.push(format!("repoch:{}", stats.transport_recovery_epoch));
        }
    }
    // 便于 trace/面板对齐：transport-await 长窗时标明关注点（主机 IDR / 本端干净锚）。
    let exit_path = recovery_exit_path_from_stats(stats, now_ms, RecoveryExitThresholds::default());
    let waiting_keyframe_stall = stats
        .video_decoder_recovery_state
        .as_deref()
        .is_some_and(|state| state == "waiting-keyframe");
    if stats
        .video_owner_reason
        .as_deref()
        .is_some_and(|r| r == "receiverWaitingKeyframe")
        || waiting_keyframe_stall
    {
        let suffix = recovery_exit_trace_await_suffix(exit_path);
        parts.push(format!("awaitKeyframe:{suffix}"));
    } else if stats.recovery_surface_phase.is_none()
        && stats
            .video_owner_reason
            .as_deref()
            .is_some_and(|r| r == "recoverySustaining")
    {
        // 兼容旧 trace：不再写入 recoverySustaining，仅读取遗留字段。
        parts.push("recoverySustaining:cleanAnchorHolding".to_string());
        if exit_path == RecoveryExitPath::TimedFallback {
            parts.push("awaitKeyframe:timedFallback".to_string());
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" | "))
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
enum UnifiedLifecycleState {
    Startup,
    Recovering,
    Observing,
    LocalSelfHealing,
    RecoveryEligible,
    ActiveRecovery,
    RecoveryBlocked,
    RampUp,
    Steady,
    Degraded,
    Failed,
    Closed,
}

impl UnifiedLifecycleState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Startup => "startup",
            Self::Recovering => "recovering",
            Self::Observing => "observing",
            Self::LocalSelfHealing => "local-self-healing",
            Self::RecoveryEligible => "recovery-eligible",
            Self::ActiveRecovery => "active-recovery",
            Self::RecoveryBlocked => "recovery-blocked",
            Self::RampUp => "ramp-up",
            Self::Steady => "steady",
            Self::Degraded => "degraded",
            Self::Failed => "failed",
            Self::Closed => "closed",
        }
    }

    fn from_runtime_value(value: &str) -> Option<Self> {
        match value {
            "startup" | "connecting" | "handshaking" | "priming" => Some(Self::Startup),
            "recovering" => Some(Self::Recovering),
            "observing" => Some(Self::Observing),
            "local-self-healing" => Some(Self::LocalSelfHealing),
            "recovery-eligible" => Some(Self::RecoveryEligible),
            "active-recovery" => Some(Self::ActiveRecovery),
            "recovery-blocked" => Some(Self::RecoveryBlocked),
            "ramp-up" => Some(Self::RampUp),
            "steady" => Some(Self::Steady),
            "degraded" | "degraded-serving" => Some(Self::Degraded),
            "failed" | "failed-terminal" => Some(Self::Failed),
            "closed" => Some(Self::Closed),
            _ => None,
        }
    }
}

fn classify_unified_lifecycle(
    runtime_stats: Option<&XbxEngineMediaRuntimeStats>,
    runtime_state: Option<&RecoveryRuntimeState>,
    owner: Option<&VideoOwnerContract>,
    now_ms: f64,
) -> Option<String> {
    let stats = runtime_stats?;
    let phase = compute_unified_lifecycle(stats, runtime_state, owner, now_ms);
    Some(phase.as_str().to_string())
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

fn compute_unified_lifecycle(
    stats: &XbxEngineMediaRuntimeStats,
    runtime_state: Option<&RecoveryRuntimeState>,
    owner: Option<&VideoOwnerContract>,
    now_ms: f64,
) -> UnifiedLifecycleState {
    if let Some(phase) = stats
        .session_phase
        .as_deref()
        .and_then(UnifiedLifecycleState::from_runtime_value)
    {
        if phase != UnifiedLifecycleState::Steady {
            return phase;
        }
    }
    let transport_state = format!("{:?}", stats.transport_state);
    if transport_state == "Closed" {
        return UnifiedLifecycleState::Closed;
    }
    if transport_state == "Failed"
        || stats
            .latest_recovery_decision_ledger
            .as_ref()
            .is_some_and(|ledger| ledger.state_after == "failed-terminal")
    {
        return UnifiedLifecycleState::Failed;
    }
    if let Some(owner) = owner {
        return match owner.state.as_str() {
            "degraded-serving" => UnifiedLifecycleState::Degraded,
            "stable-serving" => {
                if stats.transport_recovery_episode_active {
                    UnifiedLifecycleState::RampUp
                } else {
                    UnifiedLifecycleState::Steady
                }
            }
            "rebuilding-supply" | "supply-starved" => UnifiedLifecycleState::RecoveryEligible,
            "seeking-anchor" | "priming" => UnifiedLifecycleState::Startup,
            _ => UnifiedLifecycleState::Steady,
        };
    }
    if transport_state != "Connected" {
        return UnifiedLifecycleState::Startup;
    }
    if stats.message_handshake_acked_at_ms.is_none() {
        return UnifiedLifecycleState::Startup;
    }
    if !has_visible_video_output(stats) || stats.control_ready_at_ms.is_none() {
        return UnifiedLifecycleState::Startup;
    }
    if let Some(phase) = stats.session_phase.as_deref() {
        if phase == "observing" {
            return UnifiedLifecycleState::Observing;
        }
        if phase == "local-self-healing" {
            return UnifiedLifecycleState::LocalSelfHealing;
        }
        if phase == "recovery-eligible" {
            return UnifiedLifecycleState::RecoveryEligible;
        }
        if phase == "active-recovery" || phase == "recovering" {
            return UnifiedLifecycleState::ActiveRecovery;
        }
        if phase == "recovery-blocked" {
            return UnifiedLifecycleState::RecoveryBlocked;
        }
        if phase == "ramp-up" {
            return UnifiedLifecycleState::RampUp;
        }
    }
    let fresh_output = has_recent_video_output(stats, now_ms);
    if !fresh_output
        && runtime_state
            .map(|state| {
                state.phase == crate::transport::rtc::recovery::startup::SessionPhase::Recovering
            })
            .unwrap_or_else(|| {
                matches!(
                    stats.session_phase.as_deref(),
                    Some(
                        "recovering" | "recovery-eligible" | "active-recovery" | "recovery-blocked"
                    )
                )
            })
    {
        return UnifiedLifecycleState::ActiveRecovery;
    }
    if runtime_state
        .map(|state| state.phase == crate::transport::rtc::recovery::startup::SessionPhase::Startup)
        .unwrap_or(false)
    {
        return UnifiedLifecycleState::Startup;
    }
    UnifiedLifecycleState::Steady
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
            "adapterIdleTimeout" => "idleTimeout".to_string(),
            "displaySupplyCritical" => "displaySupplyCritical".to_string(),
            "displaySupplyDegraded" => "displaySupplyDegraded".to_string(),
            "decoderBackendFailure" => "decoderBackendFailure".to_string(),
            "transportSampleLoss" => "sampleLoss".to_string(),
            "receiverWaitingKeyframe" | "ingressWaitKeyframe" => "waitingKeyframe".to_string(),
            "reconfigure" => "reconfigure".to_string(),
            // steady 主路径不应落在笼统 recovering，避免与传输/解码恢复混淆。
            "steady" => "none".to_string(),
            "degradedSteady" => "displayDegradedSteady".to_string(),
            "supplyStarved" => "displaySupplyStarved".to_string(),
            "hostPresentStalled" => "hostPresentStalled".to_string(),
            "seekingAnchor" | "priming" => "startupPriming".to_string(),
            "rebuildingSupply" => "transportRecovering".to_string(),
            _ => {
                if stats.video_decoder_stalled == Some(true)
                    || renderer_shadow_blocks_serviceability(stats, now_ms)
                {
                    "pipelineStall".to_string()
                } else {
                    "recovering".to_string()
                }
            }
        });
    }
    let fresh_output = has_recent_video_output(stats, now_ms);
    if stats.video_decoder_stalled == Some(true)
        || renderer_shadow_blocks_serviceability(stats, now_ms)
    {
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
            || matches!(
                stats.session_phase.as_deref(),
                Some("recovering" | "recovery-eligible" | "active-recovery" | "recovery-blocked")
            ))
    {
        return Some("active-recovery".to_string());
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
    // 只检查真实事件时间戳，不使用平滑 FPS 指标避免误判
    present_fresh || decode_fresh
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
#[path = "stats.test.rs"]
mod tests;
