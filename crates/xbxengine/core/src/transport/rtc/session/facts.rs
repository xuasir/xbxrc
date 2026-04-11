use std::sync::Mutex;

use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::policy::display_supply::SchedulingDemandSignal;
use crate::transport::rtc::policy::video_scheduling_owner::VideoSchedulingOwnerInput;
use crate::transport::rtc::projection::TransportSnapshot;
use crate::transport::rtc::recovery::contract::{
    current_clean_anchor_observed_at_ms, has_current_transport_await_issue_from_observation,
    is_transport_await_probe_source_event,
};
use crate::transport::rtc::recovery::escalation::VideoEscalationReason;
use crate::transport::rtc::recovery::policy::DisplaySupplyThresholds;
use crate::XbxEngineAnchorCandidateLedger;
use crate::XbxEngineH264InspectionObservation;
use crate::XbxEngineMediaRuntimeStats;
use crate::XbxEngineVideoTimelineObservation;
use crate::XbxEngineVideoTrackStatus;

#[derive(Clone, Debug, Default)]
pub(crate) struct OwnerRuntimeFacts {
    pub(crate) recovery_epoch: u64,
    pub(crate) latest_video_timeline_observation: Option<XbxEngineVideoTimelineObservation>,
    pub(crate) clean_anchor_epoch: Option<u64>,
    pub(crate) clean_anchor_observed_at_ms: Option<f64>,
    pub(crate) clean_anchor_source_event: Option<String>,
    pub(crate) latest_anchor_candidate_ledger: Option<XbxEngineAnchorCandidateLedger>,
    pub(crate) latest_video_track_status: Option<XbxEngineVideoTrackStatus>,
    pub(crate) latest_h264_inspection_observation: Option<XbxEngineH264InspectionObservation>,
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
        host_present_epoch,
        host_cadence_phase,
        present_submit_count_total,
        present_drop_count_total,
        present_overwrite_count_total,
        pacer_submit_count_total,
        pacer_drop_count_total,
        renderer_submit_count_total,
        renderer_drop_count_total,
    ) = RuntimeStatsSink::read_shared(runtime_stats, |stats| {
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        (
            stats.host_no_pending_pressure_level.clone(),
            Some(stats.host_no_pending_streak),
            stats
                .latest_video_host_present_time_ms
                .map(|ts| (now_ms - ts).max(0.0)),
            stats
                .latest_video_decode_ok_time_ms
                .map(|ts| (now_ms - ts).max(0.0)),
            stats.video_renderer_stalled.unwrap_or(false),
            Some(stats.host_display_tick_epoch),
            Some(stats.video_present_epoch),
            stats.host_cadence_phase.clone(),
            Some(stats.video_present_submit_count_total),
            Some(stats.video_present_drop_count_total),
            Some(stats.video_present_overwrite_count_total),
            Some(stats.video_pacer_submit_count_total),
            Some(stats.video_pacer_drop_count_total),
            Some(stats.video_renderer_submit_count_total),
            Some(stats.video_renderer_drop_count_total),
        )
    })
    .unwrap_or((
        None, None, None, None, false, None, None, None, None, None, None, None, None, None, None,
    ));
    SchedulingDemandSignal {
        no_pending_pressure_level,
        no_pending_streak,
        present_age_ms,
        decode_age_ms,
        video_renderer_stalled,
        host_display_tick_epoch,
        host_present_epoch,
        host_cadence_phase,
        present_submit_count_total,
        present_drop_count_total,
        present_overwrite_count_total,
        pacer_submit_count_total,
        pacer_drop_count_total,
        renderer_submit_count_total,
        renderer_drop_count_total,
    }
}

pub(crate) fn read_owner_runtime_facts(
    runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
) -> OwnerRuntimeFacts {
    RuntimeStatsSink::read_shared(runtime_stats, |stats| OwnerRuntimeFacts {
        recovery_epoch: stats.transport_recovery_epoch,
        latest_video_timeline_observation: stats.latest_video_timeline_observation.clone(),
        clean_anchor_epoch: stats.video_anchor_clean_epoch,
        clean_anchor_observed_at_ms: stats.video_anchor_clean_observed_at_ms,
        clean_anchor_source_event: stats.video_anchor_clean_source_event.clone(),
        latest_anchor_candidate_ledger: stats.latest_anchor_candidate_ledger.clone(),
        latest_video_track_status: stats.latest_video_track_status.clone(),
        latest_h264_inspection_observation: stats.latest_h264_inspection_observation.clone(),
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
    VideoSchedulingOwnerInput {
        connection_state: snapshot.connection.lifecycle_state,
        recovery_epoch: owner_facts.recovery_epoch,
        first_frame_acquisition_priority_allowed,
        anchor_reason_label,
        demand,
        clean_anchor_epoch: owner_facts.clean_anchor_epoch,
        clean_anchor_observed_at_ms: owner_facts.clean_anchor_observed_at_ms,
        clean_anchor_source_event: owner_facts.clean_anchor_source_event.clone(),
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
        display_supply_thresholds,
        observed_at_ms,
    }
}

fn resolve_anchor_reason_label(
    owner_facts: &OwnerRuntimeFacts,
    absorb_first_frame_acquisition_anchor_issue: bool,
) -> Option<String> {
    owner_facts
        .latest_video_timeline_observation
        .as_ref()
        .and_then(|timeline| {
            if absorb_first_frame_acquisition_anchor_issue {
                return None;
            }
            resolve_anchor_reason_label_from_timeline(owner_facts, timeline)
        })
}

fn resolve_anchor_reason_label_from_timeline(
    owner_facts: &OwnerRuntimeFacts,
    timeline: &XbxEngineVideoTimelineObservation,
) -> Option<String> {
    let current_clean_anchor_at_ms = current_clean_anchor_observed_at_ms(
        owner_facts.clean_anchor_epoch,
        owner_facts.clean_anchor_observed_at_ms,
        owner_facts.clean_anchor_source_event.as_deref(),
        owner_facts.recovery_epoch,
    );
    let current_transport_await_probe =
        is_transport_await_probe_source_event(Some(timeline.source_event.as_str()))
            && current_clean_anchor_at_ms
                .is_none_or(|clean_anchor_at_ms| timeline.observed_at_ms > clean_anchor_at_ms);
    let label = match (
        has_current_transport_await_issue_from_observation(timeline, current_clean_anchor_at_ms),
        current_transport_await_probe,
        timeline.chain.state.as_str(),
        timeline.chain.reason.as_deref(),
        timeline.source_event.as_str(),
    ) {
        (true, _, "broken", Some(reason), _) | (true, _, "recovering", Some(reason), _) => reason,
        (_, true, _, _, "frame-await-recovery-keyframe") => "transportAwaitRecoveryKeyframe",
        (_, true, _, _, "frame-inspection-rejected-await-keyframe") => {
            "transportAwaitRecoveryKeyframe"
        }
        _ => return None,
    };
    VideoEscalationReason::from_recovery_reason_label(label).map(|_| label.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_transport_await_timeline_after_clean_anchor_does_not_raise_anchor_reason() {
        let facts = OwnerRuntimeFacts {
            recovery_epoch: 7,
            clean_anchor_epoch: Some(7),
            clean_anchor_observed_at_ms: Some(120.0),
            clean_anchor_source_event: Some("chain-clean-keyframe-submitted".to_string()),
            latest_video_timeline_observation: Some(crate::XbxEngineVideoTimelineObservation {
                observation_id: 1,
                source_event: "frame-await-recovery-keyframe".to_string(),
                gap: None,
                frame: None,
                chain: crate::XbxEngineVideoTimelineChainSnapshot {
                    state: "recovering".to_string(),
                    reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                    observed_at_ms: 110.0,
                },
                observed_at_ms: 110.0,
            }),
            ..OwnerRuntimeFacts::default()
        };

        assert_eq!(resolve_anchor_reason_label(&facts, false), None);
    }

    #[test]
    fn fresh_transport_await_timeline_still_raises_anchor_reason() {
        let facts = OwnerRuntimeFacts {
            recovery_epoch: 7,
            clean_anchor_epoch: Some(7),
            clean_anchor_observed_at_ms: Some(120.0),
            clean_anchor_source_event: Some("chain-clean-keyframe-submitted".to_string()),
            latest_video_timeline_observation: Some(crate::XbxEngineVideoTimelineObservation {
                observation_id: 2,
                source_event: "frame-await-recovery-keyframe".to_string(),
                gap: None,
                frame: None,
                chain: crate::XbxEngineVideoTimelineChainSnapshot {
                    state: "recovering".to_string(),
                    reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                    observed_at_ms: 130.0,
                },
                observed_at_ms: 130.0,
            }),
            ..OwnerRuntimeFacts::default()
        };

        assert_eq!(
            resolve_anchor_reason_label(&facts, false),
            Some("transportAwaitRecoveryKeyframe".to_string())
        );
    }
}
