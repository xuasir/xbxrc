//! Session 层运行期事实拼装：`OwnerRuntimeFacts`、`VideoSchedulingOwnerInput` 等。
//! RFC：顶层编排入口在 `session::policy`；本模块不承载 transport 昂贵恢复主权。

use std::sync::{Arc, Mutex};

use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::policy::display_supply::SchedulingDemandSignal;
use crate::transport::rtc::policy::video_scheduling_owner::VideoSchedulingOwnerInput;
use crate::transport::rtc::projection::TransportSnapshot;
use crate::transport::rtc::recovery::contract::RecoveryDisplayFacts;
use crate::transport::rtc::recovery::contract::{
    current_clean_anchor_bridge_observed_at_ms, current_clean_anchor_observed_at_ms_from_stats,
    derive_gap_severity_from_timeline_observation, derive_gap_severity_with_episode_stall,
    frame_value_from_gap_severity, has_current_clean_anchor_from_stats,
    recovery_episode_stage_from_status, recovery_exit_path_from_stats,
    recovery_progress_level_from_episode, sync_derived_recovery_contract_fields,
    DerivedDecoderHealth, RecoveryContractSnapshot, RecoveryExitPath, RecoveryExitThresholds,
    RecoverySurfacePhase,
};
use crate::transport::rtc::recovery::escalation::VideoEscalationReason;
use crate::transport::rtc::recovery::policy::{DisplaySupplyThresholds, ScenarioPolicyProfileKind};
use crate::transport::rtc::recovery::remote_profile_runtime::persist_runtime_remote_profile_facts;
use crate::transport::rtc::recovery::runtime_state::{
    resolve_recovery_profile, resolve_runtime_recovery_profile,
};
use crate::transport::rtc::recovery::timing::resolve_recovery_dynamic_timing;
use crate::XbxEngineAnchorCandidateLedger;
use crate::XbxEngineH264InspectionObservation;
use crate::XbxEngineKeyframeRequestEpisodeObservation as XbxEnginePictureRecoveryEpisodeObservation;
use crate::XbxEngineMediaRuntimeStats;
use crate::XbxEngineVideoTimelineObservation;
use crate::XbxEngineVideoTrackStatus;

use super::startup_compat::{
    first_frame_acquisition_priority_active, should_absorb_first_frame_acquisition_anchor_issue,
};

/// RFC Batch 1：`session::policy` 主路径只读此聚合，不直接散落读取 stats 子字段拼装 owner 输入。
#[derive(Clone, Debug)]
pub(crate) struct RtcSessionPolicyOrchestrationInput {
    /// 与 `owner_input.demand` 同源；按 RFC 字段表保留，供顶层只读聚合扩展。
    #[allow(dead_code)]
    pub(crate) demand: SchedulingDemandSignal,
    pub(crate) owner_facts: OwnerRuntimeFacts,
    pub(crate) owner_input: VideoSchedulingOwnerInput,
    pub(crate) recovery_profile_kind: ScenarioPolicyProfileKind,
}

/// 与合入前一致的顺序：`demand` 在 `persist_runtime_remote_profile_facts` 之前采集（避免 persist 副作用改变 demand）。
pub(crate) fn build_rtc_session_policy_orchestration_input(
    snapshot: &TransportSnapshot,
    runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    observed_at_ms: f64,
    pre_first_frame_reconnect_fallback_ms: f64,
) -> RtcSessionPolicyOrchestrationInput {
    RuntimeStatsSink::new(runtime_stats.clone()).update(|stats| {
        sync_derived_recovery_contract_fields(stats, observed_at_ms);
    });
    let demand = build_scheduling_demand_signal(runtime_stats.as_ref());
    RuntimeStatsSink::new(runtime_stats.clone()).update(|stats| {
        persist_runtime_remote_profile_facts(stats, observed_at_ms);
    });
    let profile = resolve_recovery_profile(runtime_stats.as_ref());
    let owner_facts = read_owner_runtime_facts(runtime_stats.as_ref(), observed_at_ms);
    let absorb_first_frame_acquisition_anchor_issue = owner_facts
        .latest_video_timeline_observation
        .as_ref()
        .is_some_and(|timeline| {
            should_absorb_first_frame_acquisition_anchor_issue(
                snapshot,
                timeline.chain.reason.as_deref(),
                timeline.source_event.as_str(),
                runtime_stats.as_ref(),
                pre_first_frame_reconnect_fallback_ms,
            )
        });
    let owner_input = build_owner_input(
        snapshot,
        demand.clone(),
        &owner_facts,
        first_frame_acquisition_priority_active(
            snapshot,
            snapshot.now_ms,
            runtime_stats.as_ref(),
            pre_first_frame_reconnect_fallback_ms,
        ),
        profile.display_supply_thresholds,
        observed_at_ms,
        absorb_first_frame_acquisition_anchor_issue,
    );
    RtcSessionPolicyOrchestrationInput {
        demand,
        owner_facts,
        owner_input,
        recovery_profile_kind: profile.kind,
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct OwnerRuntimeFacts {
    pub(crate) recovery_epoch: u64,
    pub(crate) latest_video_receiver_observation: Option<crate::XbxEngineVideoReceiverObservation>,
    pub(crate) latest_video_timeline_observation: Option<XbxEngineVideoTimelineObservation>,
    pub(crate) clean_anchor_epoch: Option<u64>,
    pub(crate) clean_anchor_observed_at_ms: Option<f64>,
    pub(crate) clean_anchor_source_event: Option<String>,
    pub(crate) clean_anchor_bridge_epoch: Option<u64>,
    pub(crate) clean_anchor_bridge_observed_at_ms: Option<f64>,
    pub(crate) clean_anchor_bridge_source_event: Option<String>,
    pub(crate) latest_anchor_candidate_ledger: Option<XbxEngineAnchorCandidateLedger>,
    pub(crate) latest_video_track_status: Option<XbxEngineVideoTrackStatus>,
    pub(crate) latest_h264_inspection_observation: Option<XbxEngineH264InspectionObservation>,
    pub(crate) recovery_displayed_idr_at_ms: Option<f64>,
    pub(crate) recovery_fresh_anchor_recovered_at_ms: Option<f64>,
    pub(crate) recovery_exit_path: RecoveryExitPath,
    pub(crate) recovery_surface_phase: RecoverySurfacePhase,
    pub(crate) derived_decoder_health: DerivedDecoderHealth,
    pub(crate) contract_snapshot: RecoveryContractSnapshot,
}

pub(crate) fn build_scheduling_demand_signal(
    runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
) -> SchedulingDemandSignal {
    let (
        no_pending_pressure_level,
        no_pending_streak,
        present_age_ms,
        decode_age_ms,
        video_renderer_stalled,
        host_display_tick_epoch,
        host_frame_present_epoch,
        host_cadence_phase,
        host_mailbox_enqueue_count_total,
        host_mailbox_drop_count_total,
        host_mailbox_overwrite_count_total,
        pacer_submit_count_total,
        pacer_drop_count_total,
        renderer_submit_count_total,
        renderer_drop_count_total,
        smoothed_present_fps,
        smoothed_decode_fps,
        submit_age_ms,
    ) = RuntimeStatsSink::read_shared(runtime_stats, |stats| {
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        (
            stats.host_no_pending_pressure_level.clone(),
            Some(stats.host_no_pending_streak),
            stats.display_age_ms.or_else(|| {
                stats
                    .latest_video_host_present_time_ms
                    .map(|ts| (now_ms - ts).max(0.0))
            }),
            stats
                .latest_video_decode_ok_time_ms
                .map(|ts| (now_ms - ts).max(0.0)),
            stats.video_renderer_stalled.unwrap_or(false),
            Some(stats.host_display_tick_epoch),
            Some(stats.host_frame_present_epoch),
            stats.host_cadence_phase.clone(),
            Some(stats.host_mailbox_enqueue_count_total),
            Some(stats.host_mailbox_drop_count_total),
            Some(stats.host_mailbox_overwrite_count_total),
            Some(stats.video_pacer_submit_count_total),
            Some(stats.video_pacer_drop_count_total),
            Some(stats.video_renderer_submit_count_total),
            Some(stats.video_renderer_drop_count_total),
            Some(stats.video_present_fps),
            Some(stats.video_decode_fps),
            stats.submit_age_ms,
        )
    })
    .unwrap_or((
        None, None, None, None, false, None, None, None, None, None, None, None, None, None, None,
        None, None, None,
    ));
    SchedulingDemandSignal {
        no_pending_pressure_level,
        no_pending_streak,
        present_age_ms,
        decode_age_ms,
        video_renderer_stalled,
        host_display_tick_epoch,
        host_frame_present_epoch,
        host_cadence_phase,
        host_mailbox_enqueue_count_total,
        host_mailbox_drop_count_total,
        host_mailbox_overwrite_count_total,
        pacer_submit_count_total,
        pacer_drop_count_total,
        renderer_submit_count_total,
        renderer_drop_count_total,
        smoothed_present_fps,
        smoothed_decode_fps,
        submit_age_ms,
    }
}

pub(crate) fn read_owner_runtime_facts(
    runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
    observed_at_ms: f64,
) -> OwnerRuntimeFacts {
    RuntimeStatsSink::read_shared(runtime_stats, |stats| {
        let profile = resolve_runtime_recovery_profile(stats);
        let exit_thresholds = RecoveryExitThresholds {
            degraded_decode_age_ms: profile.display_supply_thresholds.degraded_decode_age_ms,
            ..RecoveryExitThresholds::default()
        };
        let contract_snapshot =
            RecoveryContractSnapshot::from_stats(stats, observed_at_ms, exit_thresholds);
        OwnerRuntimeFacts {
            recovery_epoch: stats.transport_recovery_epoch,
            latest_video_receiver_observation: stats.latest_video_receiver_observation.clone(),
            latest_video_timeline_observation: stats.latest_video_timeline_observation.clone(),
            clean_anchor_epoch: stats.video_anchor_clean_epoch,
            clean_anchor_observed_at_ms: stats.video_anchor_clean_observed_at_ms,
            clean_anchor_source_event: stats.video_anchor_clean_source_event.clone(),
            clean_anchor_bridge_epoch: stats.video_anchor_bridge_epoch,
            clean_anchor_bridge_observed_at_ms: stats.video_anchor_bridge_observed_at_ms,
            clean_anchor_bridge_source_event: stats.video_anchor_bridge_source_event.clone(),
            latest_anchor_candidate_ledger: stats.latest_anchor_candidate_ledger.clone(),
            latest_video_track_status: stats.latest_video_track_status.clone(),
            latest_h264_inspection_observation: stats.latest_h264_inspection_observation.clone(),
            recovery_displayed_idr_at_ms: {
                let display = RecoveryDisplayFacts::from_stats(stats);
                display.displayed_idr_at_ms
            },
            recovery_fresh_anchor_recovered_at_ms: {
                let display = RecoveryDisplayFacts::from_stats(stats);
                display.fresh_anchor_recovered_at_ms
            },
            recovery_exit_path: contract_snapshot.exit_path,
            recovery_surface_phase: contract_snapshot.surface_phase,
            derived_decoder_health: contract_snapshot.derived_health,
            contract_snapshot,
        }
    })
    .unwrap_or_default()
}

pub(crate) fn build_owner_input(
    snapshot: &TransportSnapshot,
    demand: SchedulingDemandSignal,
    owner_facts: &OwnerRuntimeFacts,
    first_frame_acquisition_priority_allowed: bool,
    display_supply_thresholds: DisplaySupplyThresholds,
    observed_at_ms: f64,
    absorb_first_frame_acquisition_anchor_issue: bool,
) -> VideoSchedulingOwnerInput {
    let anchor_reason_label =
        resolve_anchor_reason_label(owner_facts, absorb_first_frame_acquisition_anchor_issue);
    let latest_timeline_chain_state = owner_facts
        .latest_video_timeline_observation
        .as_ref()
        .map(|observation| observation.chain.state.clone());
    let latest_timeline_source_event = owner_facts
        .latest_video_timeline_observation
        .as_ref()
        .map(|observation| observation.source_event.clone());
    let latest_track_state = owner_facts
        .latest_video_track_status
        .as_ref()
        .map(|status| status.state.clone());
    let latest_track_video_bytes_total = owner_facts
        .latest_video_track_status
        .as_ref()
        .map(|status| status.video_bytes_total);
    let receiver_state = owner_facts
        .latest_video_receiver_observation
        .as_ref()
        .map(|observation| observation.receiver_state.clone());
    VideoSchedulingOwnerInput {
        connection_state: snapshot.connection.lifecycle_state,
        recovery_epoch: owner_facts.recovery_epoch,
        receiver_state,
        first_frame_acquisition_priority_allowed,
        anchor_reason_label,
        demand,
        clean_anchor_epoch: owner_facts.clean_anchor_epoch,
        clean_anchor_observed_at_ms: owner_facts.clean_anchor_observed_at_ms,
        clean_anchor_source_event: owner_facts.clean_anchor_source_event.clone(),
        clean_anchor_bridge_epoch: owner_facts.clean_anchor_bridge_epoch,
        clean_anchor_bridge_observed_at_ms: owner_facts.clean_anchor_bridge_observed_at_ms,
        clean_anchor_bridge_source_event: owner_facts.clean_anchor_bridge_source_event.clone(),
        latest_anchor_candidate_ledger: owner_facts.latest_anchor_candidate_ledger.clone(),
        latest_video_timeline_observation: owner_facts.latest_video_timeline_observation.clone(),
        latest_timeline_chain_state,
        latest_timeline_source_event,
        latest_track_state,
        latest_track_video_bytes_total,
        latest_h264_bootstrap_ready: owner_facts
            .latest_h264_inspection_observation
            .as_ref()
            .map(|inspection| inspection.bootstrap_ready),
        latest_h264_bootstrap_reject_reason: owner_facts
            .latest_h264_inspection_observation
            .as_ref()
            .and_then(|inspection| inspection.bootstrap_reject_reason.clone()),
        latest_h264_committed_sps_present: owner_facts
            .latest_h264_inspection_observation
            .as_ref()
            .map(|inspection| inspection.committed_sps_present),
        latest_h264_committed_pps_present: owner_facts
            .latest_h264_inspection_observation
            .as_ref()
            .map(|inspection| inspection.committed_pps_present),
        latest_h264_delta_continuation_ready: owner_facts
            .latest_h264_inspection_observation
            .as_ref()
            .map(|inspection| inspection.delta_continuation_ready),
        latest_h264_observed_at_ms: owner_facts
            .latest_h264_inspection_observation
            .as_ref()
            .map(|inspection| inspection.observed_at_ms),
        recovery_displayed_idr_at_ms: owner_facts.recovery_displayed_idr_at_ms,
        recovery_fresh_anchor_recovered_at_ms: owner_facts.recovery_fresh_anchor_recovered_at_ms,
        recovery_exit_path: owner_facts.recovery_exit_path,
        recovery_surface_phase: owner_facts.recovery_surface_phase,
        derived_decoder_health: owner_facts.derived_decoder_health,
        displayed_idr_serving_wide: owner_facts.contract_snapshot.serving_wide,
        contract_snapshot: owner_facts.contract_snapshot,
        display_supply_thresholds,
        observed_at_ms,
    }
}

fn resolve_anchor_reason_label(
    owner_facts: &OwnerRuntimeFacts,
    absorb_first_frame_acquisition_anchor_issue: bool,
) -> Option<String> {
    if absorb_first_frame_acquisition_anchor_issue {
        return None;
    }
    let receiver = owner_facts.latest_video_receiver_observation.as_ref()?;
    let label = match receiver.receiver_state.as_str() {
        "waiting-keyframe" => "receiverWaitingKeyframe",
        _ => return None,
    };
    VideoEscalationReason::from_recovery_reason_label(label).map(|_| label.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn receiver_observation(state: &str) -> crate::XbxEngineVideoReceiverObservation {
        crate::XbxEngineVideoReceiverObservation {
            observation_id: 1,
            receiver_state: state.to_string(),
            gap_sequence: None,
            gap_span: None,
            nack_in_flight: false,
            keyframe_request_pending: false,
            bootstrap_reject_reason: None,
            observed_at_ms: 100.0,
        }
    }

    #[test]
    fn stale_timeline_without_receiver_waiting_does_not_raise_anchor_reason() {
        let facts = OwnerRuntimeFacts {
            recovery_epoch: 7,
            latest_video_receiver_observation: Some(receiver_observation("repairing")),
            latest_video_timeline_observation: Some(crate::XbxEngineVideoTimelineObservation {
                observation_id: 1,
                source_event: "frame-await-recovery-anchor".to_string(),
                gap: None,
                frame: None,
                chain: crate::XbxEngineVideoTimelineChainSnapshot {
                    state: "waiting-keyframe".to_string(),
                    reason: Some("receiverWaitingKeyframe".to_string()),
                    chain_break_evidence: None,
                    observed_at_ms: 110.0,
                },
                observed_at_ms: 110.0,
            }),
            ..OwnerRuntimeFacts::default()
        };

        assert_eq!(resolve_anchor_reason_label(&facts, false), None);
    }

    #[test]
    fn receiver_waiting_keyframe_raises_anchor_reason() {
        let facts = OwnerRuntimeFacts {
            recovery_epoch: 7,
            latest_video_receiver_observation: Some(receiver_observation("waiting-keyframe")),
            ..OwnerRuntimeFacts::default()
        };

        assert_eq!(
            resolve_anchor_reason_label(&facts, false),
            Some("receiverWaitingKeyframe".to_string())
        );
    }

    #[test]
    fn owner_facts_read_supply_break_surface_after_sync() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        RuntimeStatsSink::new(runtime_stats.clone()).update(|stats| {
            stats.video_decoder_recovery_state = Some("waiting-keyframe".to_string());
            stats.recovery_playback_recovered_at_ms = Some(1.0);
            stats.submit_age_ms = Some(5_000.0);
            sync_derived_recovery_contract_fields(stats, 10_000.0);
        });
        let facts = read_owner_runtime_facts(runtime_stats.as_ref(), 10_000.0);
        assert_eq!(
            facts.recovery_surface_phase,
            RecoverySurfacePhase::SupplyBreak
        );
        assert!(facts.contract_snapshot.supply_break_active);
    }
}

// ============================================================================
// 统一恢复模型：事实计算层
// ============================================================================

use crate::transport::rtc::recovery::contract::{
    FrameValue, GapSeverity, RecoveryEpisodeStage, RecoveryProgressLevel,
};

/// 统一恢复事实快照（原始事实归一）
#[derive(Clone, Debug)]
pub(crate) struct RecoveryFactsSnapshot {
    pub frame_value: Option<FrameValue>,
    pub gap_severity: Option<GapSeverity>,
    pub repairability: Option<f64>,
    #[allow(dead_code)]
    pub recovery_episode_stage: Option<RecoveryEpisodeStage>,
    pub recovery_progress_level: Option<RecoveryProgressLevel>,
    pub recovery_episode_progress_at_ms: Option<f64>,
    #[cfg(test)]
    pub episode_stalled: bool,
    #[cfg(test)]
    pub has_current_clean_anchor: bool,
}

/// 检查 episode 是否 stalled（无推进边沿）
fn check_episode_stalled(
    timeline: &XbxEngineVideoTimelineObservation,
    stats: &XbxEngineMediaRuntimeStats,
) -> bool {
    match timeline.chain.reason.as_deref() {
        Some("receiverWaitingKeyframe") => {
            let clean_anchor_at_ms = current_clean_anchor_observed_at_ms_from_stats(stats);
            let has_recent_progress = clean_anchor_at_ms
                .map(|t| timeline.observed_at_ms - t < 2000.0)
                .unwrap_or(false);
            !has_recent_progress
        }
        _ => false,
    }
}

/// 计算恢复事实快照（纯函数，无状态）
pub(crate) fn compute_recovery_facts(
    timeline: &XbxEngineVideoTimelineObservation,
    stats: &XbxEngineMediaRuntimeStats,
) -> RecoveryFactsSnapshot {
    let recovery_profile = resolve_runtime_recovery_profile(stats);
    // 1. 计算基础语义
    let base_severity = derive_gap_severity_from_timeline_observation(timeline);
    let episode_stalled = check_episode_stalled(timeline, stats);
    let gap_severity = Some(if episode_stalled {
        derive_gap_severity_with_episode_stall(timeline, true)
    } else {
        base_severity
    });

    let frame_value = gap_severity.and_then(frame_value_from_gap_severity);

    // 计算 repairability 评分
    let repairability = compute_repairability_score(timeline, stats);

    let recovery_episode = select_relevant_picture_recovery_episode_for_progress(stats);
    let recovery_episode_stage =
        recovery_episode.and_then(|ep| recovery_episode_stage_from_status(ep.status.as_str()));
    let has_current_clean_anchor = has_current_clean_anchor_from_stats(stats);
    let has_display_stable = matches!(stats.video_owner_state.as_deref(), Some("stable-serving"));
    let base_recovery_progress_level = recovery_episode
        .and_then(|ep| {
            recovery_progress_level_from_episode(
                ep.status.as_str(),
                ep.response_verdict.as_deref(),
                ep.first_video_packet_is_keyframe,
                ep.first_keyframe_packet_at_ms,
                ep.first_keyframe_decoded_at_ms,
                has_current_clean_anchor,
                has_display_stable,
            )
        })
        .or_else(|| {
            if has_display_stable {
                Some(RecoveryProgressLevel::DisplayStable)
            } else if has_current_clean_anchor {
                Some(RecoveryProgressLevel::CleanAnchorCommitted)
            } else {
                None
            }
        });
    let (recovery_progress_level, recovery_progress_override_at_ms) =
        reconcile_recovery_progress_from_current_bootstrap(
            base_recovery_progress_level,
            recovery_episode,
            stats.latest_h264_inspection_observation.as_ref(),
            has_current_clean_anchor,
            timeline.observed_at_ms,
            stats,
            &recovery_profile,
        );

    let base_recovery_episode_progress_at_ms = stats
        .video_anchor_clean_observed_at_ms
        .or_else(|| recovery_episode.and_then(|ep| ep.first_keyframe_decoded_at_ms));
    let recovery_episode_progress_at_ms =
        recovery_progress_override_at_ms.or(base_recovery_episode_progress_at_ms);

    #[cfg(test)]
    let clean_anchor_observed_at_ms = current_clean_anchor_observed_at_ms_from_stats(stats);
    #[cfg(test)]
    let has_current_clean_anchor = clean_anchor_observed_at_ms.is_some();

    RecoveryFactsSnapshot {
        frame_value,
        gap_severity,
        repairability,
        recovery_episode_stage,
        recovery_progress_level,
        recovery_episode_progress_at_ms,
        #[cfg(test)]
        episode_stalled,
        #[cfg(test)]
        has_current_clean_anchor,
    }
}

fn current_clean_anchor_bridge_observed_at_ms_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
) -> Option<f64> {
    current_clean_anchor_bridge_observed_at_ms(
        stats.video_anchor_bridge_epoch,
        stats.video_anchor_bridge_observed_at_ms,
        stats.video_anchor_bridge_source_event.as_deref(),
        stats.transport_recovery_epoch,
    )
}

fn event_is_fresh(
    observed_at_ms: Option<f64>,
    now_ms: f64,
    fresh_window_ms: f64,
    floor_at_ms: f64,
) -> Option<f64> {
    observed_at_ms.filter(|observed_at| {
        *observed_at >= floor_at_ms && (now_ms - *observed_at).max(0.0) <= fresh_window_ms
    })
}

fn continuation_only_observed_at_ms(
    recovery_episode: &XbxEnginePictureRecoveryEpisodeObservation,
    inspection: Option<&XbxEngineH264InspectionObservation>,
) -> Option<f64> {
    let inspection = inspection?;
    if inspection.bound_episode_id != Some(recovery_episode.episode_id)
        || inspection.observed_at_ms <= recovery_episode.requested_at_ms
        || inspection.bootstrap_ready
        || !inspection.admission_accepted
        || !matches!(
            inspection.bootstrap_reject_reason.as_deref(),
            Some("bootstrapMissingIdr" | "NonIdrVcl")
        )
    {
        return None;
    }
    if inspection.continuation_verdict.as_deref() == Some("receiverLocalContinuation")
        && inspection.committed_sps_present
        && inspection.committed_pps_present
        && inspection.delta_continuation_ready
    {
        return Some(inspection.observed_at_ms);
    }
    None
}

fn playback_recovered_observed_at_ms(
    recovery_episode: &XbxEnginePictureRecoveryEpisodeObservation,
    now_ms: f64,
    stats: &XbxEngineMediaRuntimeStats,
    profile: &crate::transport::rtc::recovery::policy::RecoveryScenarioProfile,
    continuation_only_observed_at_ms: Option<f64>,
) -> Option<f64> {
    let floor_at_ms = recovery_episode.requested_at_ms;
    let bridge_at_ms = event_is_fresh(
        current_clean_anchor_bridge_observed_at_ms_from_stats(stats),
        now_ms,
        profile.playback_recovered_host_present_fresh_ms,
        floor_at_ms,
    );
    let host_present_at_ms = event_is_fresh(
        stats.latest_video_host_present_time_ms,
        now_ms,
        profile.playback_recovered_host_present_fresh_ms,
        floor_at_ms,
    );
    let render_submit_at_ms = event_is_fresh(
        stats.latest_host_mailbox_submit_time_ms,
        now_ms,
        profile.playback_recovered_render_submit_fresh_ms,
        floor_at_ms,
    );
    let track_progress_at_ms = stats.latest_video_track_status.as_ref().and_then(|status| {
        (status.video_bytes_total > 0)
            .then_some(status.observed_at_ms)
            .and_then(|observed_at_ms| {
                event_is_fresh(
                    Some(observed_at_ms),
                    now_ms,
                    profile.playback_recovered_track_progress_fresh_ms,
                    floor_at_ms,
                )
            })
    });
    let decode_progress_at_ms = event_is_fresh(
        stats.latest_video_decode_ok_time_ms,
        now_ms,
        profile.playback_recovered_decode_progress_fresh_ms,
        floor_at_ms,
    );
    let decode_and_track_progress_at_ms = decode_progress_at_ms
        .zip(track_progress_at_ms)
        .map(|(decode_at_ms, track_at_ms)| decode_at_ms.max(track_at_ms));
    [
        bridge_at_ms,
        host_present_at_ms,
        render_submit_at_ms,
        decode_and_track_progress_at_ms,
        continuation_only_observed_at_ms
            .filter(|_| decode_progress_at_ms.is_some() || track_progress_at_ms.is_some()),
    ]
    .into_iter()
    .flatten()
    .max_by(|left, right| left.total_cmp(right))
}

fn decoded_stage_is_current(
    recovery_episode: &XbxEnginePictureRecoveryEpisodeObservation,
    stats: &XbxEngineMediaRuntimeStats,
    profile: &crate::transport::rtc::recovery::policy::RecoveryScenarioProfile,
) -> bool {
    let Some(decoded_at_ms) = recovery_episode.first_keyframe_decoded_at_ms else {
        return false;
    };
    let now_ms = stats
        .latest_video_decode_ok_time_ms
        .unwrap_or(decoded_at_ms.max(recovery_episode.requested_at_ms));
    let timing = resolve_recovery_dynamic_timing(stats, *profile);
    if (now_ms - decoded_at_ms).max(0.0) <= timing.clean_anchor_commit_patience_window_ms {
        return true;
    }
    stats
        .latest_video_decode_ok_time_ms
        .is_some_and(|decode_ok_at_ms| {
            decode_ok_at_ms >= decoded_at_ms
                && (now_ms - decode_ok_at_ms).max(0.0) <= profile.decoded_progress_fresh_ms
        })
}

fn reconcile_recovery_progress_from_current_bootstrap(
    progress: Option<RecoveryProgressLevel>,
    recovery_episode: Option<&XbxEnginePictureRecoveryEpisodeObservation>,
    inspection: Option<&XbxEngineH264InspectionObservation>,
    has_current_clean_anchor: bool,
    now_ms: f64,
    stats: &XbxEngineMediaRuntimeStats,
    profile: &crate::transport::rtc::recovery::policy::RecoveryScenarioProfile,
) -> (Option<RecoveryProgressLevel>, Option<f64>) {
    if has_current_clean_anchor {
        let fresh_at_ms = stats
            .recovery_fresh_anchor_recovered_at_ms
            .or(stats.recovery_displayed_idr_at_ms);
        return (
            Some(RecoveryProgressLevel::CleanAnchorCommitted),
            fresh_at_ms,
        );
    }
    if stats.recovery_playback_recovered_at_ms.is_some()
        && matches!(
            progress,
            Some(
                RecoveryProgressLevel::Decoded
                    | RecoveryProgressLevel::ContinuationSeen
                    | RecoveryProgressLevel::AnchorSeen
            )
        )
    {
        return (
            Some(RecoveryProgressLevel::PlaybackRecovered),
            stats.recovery_playback_recovered_at_ms,
        );
    }
    let Some(episode) = recovery_episode else {
        return (progress, None);
    };
    let continuation_only_at_ms = continuation_only_observed_at_ms(episode, inspection);
    let playback_recovered_at_ms =
        playback_recovered_observed_at_ms(episode, now_ms, stats, profile, continuation_only_at_ms);
    if matches!(
        progress,
        Some(
            RecoveryProgressLevel::ContinuationSeen
                | RecoveryProgressLevel::AnchorSeen
                | RecoveryProgressLevel::Decoded
                | RecoveryProgressLevel::PlaybackRecovered
        )
    ) {
        if let Some(playback_recovered_at_ms) = playback_recovered_at_ms {
            return (
                Some(RecoveryProgressLevel::PlaybackRecovered),
                Some(playback_recovered_at_ms),
            );
        }
    }
    if !matches!(
        progress,
        Some(RecoveryProgressLevel::Decoded | RecoveryProgressLevel::PlaybackRecovered)
    ) {
        return (progress, None);
    };
    if decoded_stage_is_current(episode, stats, profile) {
        return (Some(RecoveryProgressLevel::Decoded), None);
    }
    if let Some(continuation_only_at_ms) = continuation_only_at_ms {
        return (
            Some(RecoveryProgressLevel::ContinuationSeen),
            Some(continuation_only_at_ms),
        );
    }
    (
        Some(RecoveryProgressLevel::WaitingResponse),
        inspection.map(|inspection| inspection.observed_at_ms),
    )
}

fn select_relevant_picture_recovery_episode_for_progress(
    stats: &XbxEngineMediaRuntimeStats,
) -> Option<&XbxEnginePictureRecoveryEpisodeObservation> {
    let latest_active = stats
        .latest_keyframe_request_episode
        .as_ref()
        .filter(|episode| episode.retired_at_ms.is_none());
    let recent_active = stats
        .recent_keyframe_request_episodes
        .iter()
        .filter(|episode| episode.retired_at_ms.is_none())
        .max_by(|left, right| {
            left.requested_at_ms
                .total_cmp(&right.requested_at_ms)
                .then_with(|| left.episode_id.cmp(&right.episode_id))
        });
    match (latest_active, recent_active) {
        (Some(latest), Some(recent))
            if recent.requested_at_ms > latest.requested_at_ms
                || (recent.requested_at_ms == latest.requested_at_ms
                    && recent.episode_id > latest.episode_id) =>
        {
            Some(recent)
        }
        (Some(latest), _) => Some(latest),
        (None, Some(recent)) => Some(recent),
        (None, None) => None,
    }
}

/// 计算可修复性评分（0.0-1.0）
/// 基于当前网络状况和待修复gap数量评估包级恢复的可行性
fn compute_repairability_score(
    timeline: &XbxEngineVideoTimelineObservation,
    stats: &XbxEngineMediaRuntimeStats,
) -> Option<f64> {
    // 如果没有gap，返回高评分
    let gap = timeline.gap.as_ref()?;
    if matches!(gap.state.as_str(), "resolved" | "expired") {
        return Some(1.0);
    }

    // 基础评分从1.0开始，根据各种因素降低
    let mut score: f64 = 1.0;

    // 因素1: RTT影响（RTT越高，可修复性越低）
    if let Some(rtt_ms) = stats.video_rtt_ms {
        if rtt_ms > 500.0 {
            score *= 0.3; // 高RTT严重降低可修复性
        } else if rtt_ms > 200.0 {
            score *= 0.6;
        } else if rtt_ms > 100.0 {
            score *= 0.8;
        }
    }

    // 因素2: 丢包率影响（使用累计丢包估算）
    let loss_estimate = stats.inbound_video_packet_loss_estimate_total;
    let received = stats.inbound_video_packet_count_total;
    if received > 0 {
        let loss_rate = loss_estimate as f64 / (received + loss_estimate) as f64;
        if loss_rate > 0.1 {
            score *= 0.4; // 高丢包率严重降低可修复性
        } else if loss_rate > 0.05 {
            score *= 0.7;
        } else if loss_rate > 0.02 {
            score *= 0.9;
        }
    }

    // 因素3: 待修复gap数量
    let pending_count = stats.video_pending_missing_packets;
    if pending_count > 50 {
        score *= 0.3; // 大量待修复包降低可修复性
    } else if pending_count > 20 {
        score *= 0.6;
    } else if pending_count > 10 {
        score *= 0.8;
    }

    // 因素4: 链状态影响
    if matches!(timeline.chain.state.as_str(), "broken" | "repairing") {
        score *= 0.7; // 链断裂时降低可修复性
    }

    // 确保评分在合理范围内
    Some(score.max(0.0).min(1.0))
}

#[cfg(test)]
#[path = "facts.test.rs"]
mod facts_test;
