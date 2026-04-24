use super::{ChainState, FrameReceiveState, GapState, VideoTimelineState};
use crate::media::video::ingress::budget::FrameBudgetContext;
use crate::media::video::types::FrameRecoveryDisposition;
use crate::media::video::types::FrameValue;
use crate::{XbxEngineAnchorCandidateFailureReason, XbxEngineAnchorCandidateState};

#[test]
fn wait_anchor_gate_moves_chain_between_recovering_and_healthy() {
    let mut state = VideoTimelineState::new();
    assert!(!state.chain_requires_recovery_anchor());
    assert_eq!(state.chain_state(), ChainState::Healthy);
    state.apply_wait_keyframe_gate(true);
    assert!(state.chain_requires_recovery_anchor());
    assert_eq!(state.chain_state(), ChainState::Recovering);
}

#[test]
fn chain_broken_then_anchor_request_enters_recovering() {
    let mut state = VideoTimelineState::new();
    state.on_chain_broken();
    assert_eq!(state.chain_state(), ChainState::Broken);
    state.on_recovery_keyframe_requested();
    assert_eq!(state.chain_state(), ChainState::Recovering);
    assert!(state.chain_requires_recovery_anchor());
}

#[test]
fn sustaining_recovery_is_not_reported_as_waiting_anchor() {
    let mut state = VideoTimelineState::new();
    state.on_clean_anchor_ingress(95_050, 12.0);

    assert_eq!(state.chain_state(), ChainState::SustainingRecovery);
    assert!(!state.chain_requires_recovery_anchor());
}

#[test]
fn per_gap_lifecycle_is_tracked() {
    let mut state = VideoTimelineState::new();
    state.observe_gap(&[10, 11], 1.0, Some(90_000), "supply", "supply");
    state.mark_gap_reorder_pending(&[10, 11], 2.0, Some(90_000), "supply", "supply");
    state.mark_gap_nack_candidate(&[10], 3.0, Some(90_000), "supply", "supply");
    state.mark_gap_repair_in_flight(&[10], 4.0, Some(90_000), "supply", "supply");
    state.mark_gap_resolved(10, 5.0, Some(90_000), "supply", "supply");
    state.mark_gap_expired(
        &[11],
        6.0,
        Some(90_000),
        "supply",
        "supply",
        Some("deadline"),
    );
    assert_eq!(state.gap_state_of(10), Some(GapState::Resolved));
    assert_eq!(state.gap_state_of(11), Some(GapState::Expired));
    assert_eq!(
        state.frame_state_of(90_000),
        Some(FrameReceiveState::Closed)
    );
}

#[test]
fn anchor_candidate_ledger_tracks_rejected_candidate() {
    let mut state = VideoTimelineState::new();
    state.observe_anchor_candidate(
        3,
        Some(91_200),
        "frame-inspection-rejected-await-anchor",
        XbxEngineAnchorCandidateState::Rejected,
        Some(XbxEngineAnchorCandidateFailureReason::InspectionRejectedInvalidSliceHeader),
        3.0,
    );
    let ledger = state
        .latest_anchor_candidate_ledger()
        .expect("anchor candidate");
    assert_eq!(ledger.recovery_epoch, 3);
    assert_eq!(ledger.frame_rtp_timestamp, Some(91_200));
    assert_eq!(ledger.state, XbxEngineAnchorCandidateState::Rejected);
    assert_eq!(
        ledger.failure_reason,
        Some(XbxEngineAnchorCandidateFailureReason::InspectionRejectedInvalidSliceHeader)
    );
}

#[test]
fn anchor_candidate_ledger_tracks_clean_anchor_submission() {
    let mut state = VideoTimelineState::new();
    state.observe_anchor_candidate(
        7,
        Some(95_001),
        "frame-complete-candidate",
        XbxEngineAnchorCandidateState::Observed,
        None,
        10.0,
    );
    state.observe_anchor_candidate(
        7,
        Some(95_001),
        "chain-clean-anchor-submitted",
        XbxEngineAnchorCandidateState::SubmittedCleanAnchor,
        None,
        12.0,
    );
    let ledger = state
        .latest_anchor_candidate_ledger()
        .expect("anchor candidate");
    assert_eq!(ledger.recovery_epoch, 7);
    assert_eq!(
        ledger.state,
        XbxEngineAnchorCandidateState::SubmittedCleanAnchor
    );
    assert_eq!(ledger.source_event, "chain-clean-anchor-submitted");
    assert_eq!(ledger.failure_reason, None);
}

#[test]
fn anonymous_repair_candidate_inherits_latest_frame_in_same_epoch() {
    let mut state = VideoTimelineState::new();
    state.observe_anchor_candidate(
        9,
        Some(96_001),
        "frame-complete-candidate",
        XbxEngineAnchorCandidateState::Observed,
        None,
        20.0,
    );
    state.observe_anchor_candidate(
        9,
        None,
        "gap-repair-in-flight",
        XbxEngineAnchorCandidateState::Repaired,
        None,
        21.0,
    );
    let ledger = state
        .latest_anchor_candidate_ledger()
        .expect("anchor candidate");
    assert_eq!(ledger.recovery_epoch, 9);
    assert_eq!(ledger.frame_rtp_timestamp, Some(96_001));
    assert_eq!(ledger.state, XbxEngineAnchorCandidateState::Repaired);
    assert_eq!(ledger.source_event, "gap-repair-in-flight");
}

#[test]
fn anchor_candidate_ledger_tracks_observed_to_awaiting_transition() {
    let mut state = VideoTimelineState::new();
    state.observe_anchor_candidate(
        11,
        Some(96_001),
        "frame-complete-candidate",
        XbxEngineAnchorCandidateState::Observed,
        None,
        20.0,
    );
    state.observe_anchor_candidate(
        11,
        Some(96_001),
        "frame-await-recovery-anchor",
        XbxEngineAnchorCandidateState::AwaitingRecovery,
        Some(XbxEngineAnchorCandidateFailureReason::AwaitingRecoveryKeyframe),
        24.0,
    );
    let ledger = state
        .latest_anchor_candidate_ledger()
        .expect("anchor candidate");
    assert_eq!(ledger.recovery_epoch, 11);
    assert_eq!(ledger.frame_rtp_timestamp, Some(96_001));
    assert_eq!(
        ledger.state,
        XbxEngineAnchorCandidateState::AwaitingRecovery
    );
    assert_eq!(ledger.source_event, "frame-await-recovery-anchor");
    assert_eq!(
        ledger.failure_reason,
        Some(XbxEngineAnchorCandidateFailureReason::AwaitingRecoveryKeyframe)
    );
}

#[test]
fn anchor_candidate_ledger_tracks_observed_to_repaired_transition() {
    let mut state = VideoTimelineState::new();
    state.observe_anchor_candidate(
        12,
        Some(96_101),
        "frame-complete-candidate",
        XbxEngineAnchorCandidateState::Observed,
        None,
        30.0,
    );
    state.observe_anchor_candidate(
        12,
        Some(96_101),
        "gap-resolved",
        XbxEngineAnchorCandidateState::Repaired,
        None,
        36.0,
    );
    let ledger = state
        .latest_anchor_candidate_ledger()
        .expect("anchor candidate");
    assert_eq!(ledger.recovery_epoch, 12);
    assert_eq!(ledger.frame_rtp_timestamp, Some(96_101));
    assert_eq!(ledger.state, XbxEngineAnchorCandidateState::Repaired);
    assert_eq!(ledger.source_event, "gap-resolved");
    assert_eq!(ledger.failure_reason, None);
}

#[test]
fn anchor_candidate_ledger_tracks_observed_to_rejected_transition() {
    let mut state = VideoTimelineState::new();
    state.observe_anchor_candidate(
        13,
        Some(96_201),
        "frame-complete-candidate",
        XbxEngineAnchorCandidateState::Observed,
        None,
        40.0,
    );
    state.observe_anchor_candidate(
        13,
        Some(96_201),
        "gap-expired-skipped",
        XbxEngineAnchorCandidateState::Rejected,
        Some(XbxEngineAnchorCandidateFailureReason::GapExpiredDeadline),
        47.0,
    );
    let ledger = state
        .latest_anchor_candidate_ledger()
        .expect("anchor candidate");
    assert_eq!(ledger.recovery_epoch, 13);
    assert_eq!(ledger.frame_rtp_timestamp, Some(96_201));
    assert_eq!(ledger.state, XbxEngineAnchorCandidateState::Rejected);
    assert_eq!(ledger.source_event, "gap-expired-skipped");
    assert_eq!(
        ledger.failure_reason,
        Some(XbxEngineAnchorCandidateFailureReason::GapExpiredDeadline)
    );
}

#[test]
fn frame_recovery_ledger_prefers_supply_chain_failure() {
    let mut state = VideoTimelineState::new();
    state.record_frame_recovery(
        90_000,
        Some(100.0),
        FrameRecoveryDisposition::UnrecoverableLate,
        Some("late"),
        FrameBudgetContext::steady_for_value(FrameValue::new(false, false, 12 * 1024)),
    );
    state.record_frame_recovery(
        90_000,
        None,
        FrameRecoveryDisposition::UnrecoverableReferenceChain,
        Some("chain"),
        FrameBudgetContext::steady_for_value(FrameValue::new(false, true, 48 * 1024)),
    );
    let entry = state.take_frame_recovery(90_000).expect("entry");
    assert_eq!(
        entry.frame_recovery_disposition,
        FrameRecoveryDisposition::UnrecoverableReferenceChain
    );
    assert_eq!(entry.frame_unrecoverable_reason.as_deref(), Some("chain"));
    assert_eq!(entry.frame_playout_deadline_at_ms, Some(100.0));
    assert_eq!(entry.budget_context.recovery_value_tier(), "supply");
}

#[test]
fn timeout_reason_is_exposed_via_chain_reason_when_no_frame_close_reason() {
    let mut state = VideoTimelineState::new();
    state.record_timeout_reason("streamIdleTimeout");
    let observation = state.snapshot_for_observation(1, "timeout-stream-idle", None, None, 1.0);
    assert_eq!(
        observation.chain.reason.as_deref(),
        Some("streamIdleTimeout")
    );
}

#[test]
fn timeout_reason_is_cleared_after_new_frame_observed() {
    let mut state = VideoTimelineState::new();
    state.record_timeout_reason("streamThinStall");
    let before = state.snapshot_for_observation(1, "timeout-stream-thin-stall", None, None, 1.0);
    assert_eq!(before.chain.reason.as_deref(), Some("streamThinStall"));

    state.observe_frame(90_001, 2.0, Some(false), "disposable");
    let after = state.snapshot_for_observation(2, "frame-observed", None, Some(90_001), 2.0);
    assert_eq!(after.chain.reason.as_deref(), None);
}

#[test]
fn timeout_detected_sets_stalled_chain_state() {
    let mut state = VideoTimelineState::new();
    state.on_timeout_detected();
    assert_eq!(state.chain_state(), ChainState::Stalled);
    let observation = state.snapshot_for_observation(1, "timeout-stream-idle", None, None, 1.0);
    assert_eq!(observation.chain.state, "stalled");
}

#[test]
fn frame_observed_after_timeout_moves_stalled_to_repairing_then_healthy() {
    let mut state = VideoTimelineState::new();
    state.on_timeout_detected();
    state.observe_frame(90_001, 2.0, Some(false), "disposable");
    assert_eq!(state.chain_state(), ChainState::Repairing);
    state.mark_frame_complete_candidate(90_001, 3.0, Some(false), "disposable");
    assert_eq!(state.chain_state(), ChainState::Repairing);
    state.observe_frame(90_002, 130.0, Some(false), "disposable");
    state.mark_frame_complete_candidate(90_002, 140.0, Some(false), "disposable");
    assert_eq!(state.chain_state(), ChainState::Healthy);
}

#[test]
fn single_complete_candidate_does_not_whiten_recovering_chain_without_stable_window() {
    let mut state = VideoTimelineState::new();
    state.on_timeout_detected();
    state.observe_frame(92_001, 5.0, Some(false), "disposable");
    state.mark_frame_complete_candidate(92_001, 10.0, Some(false), "disposable");
    assert_eq!(state.chain_state(), ChainState::Repairing);
}

#[test]
fn recovering_chain_requires_stable_clean_frames_before_healthy() {
    let mut state = VideoTimelineState::new();
    state.on_timeout_detected();

    state.observe_frame(93_001, 10.0, Some(false), "disposable");
    state.mark_frame_complete_candidate(93_001, 20.0, Some(false), "disposable");
    assert_eq!(state.chain_state(), ChainState::Repairing);

    state.observe_frame(93_002, 121.0, Some(false), "disposable");
    state.mark_frame_complete_candidate(93_002, 132.0, Some(false), "disposable");
    assert_eq!(state.chain_state(), ChainState::Healthy);
}

#[test]
fn timeout_does_not_override_recovering_chain() {
    let mut state = VideoTimelineState::new();
    state.apply_wait_keyframe_gate(true);
    assert_eq!(state.chain_state(), ChainState::Recovering);
    state.on_timeout_detected();
    assert_eq!(state.chain_state(), ChainState::Recovering);
}

#[test]
fn expired_reference_gap_does_not_coexist_with_healthy_chain() {
    let mut state = VideoTimelineState::new();
    state.observe_gap(&[11], 1.0, Some(90_000), "supply", "supply");
    assert_eq!(state.chain_state(), ChainState::Repairing);
    state.mark_gap_expired(
        &[11],
        2.0,
        Some(90_000),
        "supply",
        "supply",
        Some("deadline"),
    );
    assert_eq!(state.gap_state_of(11), Some(GapState::Expired));
    assert_eq!(state.chain_state(), ChainState::Broken);

    state.mark_frame_complete_candidate(90_001, 3.0, Some(false), "disposable");
    assert_eq!(state.chain_state(), ChainState::Broken);
}

#[test]
fn gap_resolved_does_not_whiten_chain_without_stable_completion() {
    let mut state = VideoTimelineState::new();
    state.observe_gap(&[41], 1.0, Some(99_000), "supply", "supply");
    assert_eq!(state.chain_state(), ChainState::Repairing);

    state.mark_gap_repair_in_flight(&[41], 2.0, Some(99_000), "supply", "supply");
    assert_eq!(state.chain_state(), ChainState::Repairing);
    state.mark_gap_resolved(41, 3.0, Some(99_000), "supply", "supply");
    assert_eq!(state.chain_state(), ChainState::Repairing);

    state.observe_frame(99_001, 20.0, Some(false), "disposable");
    state.mark_frame_complete_candidate(99_001, 30.0, Some(false), "disposable");
    assert_eq!(state.chain_state(), ChainState::Repairing);

    state.observe_frame(99_002, 160.0, Some(false), "disposable");
    state.mark_frame_complete_candidate(99_002, 170.0, Some(false), "disposable");
    assert_eq!(state.chain_state(), ChainState::Healthy);
}

#[test]
fn broken_chain_is_not_whitened_by_disposable_until_clean_anchor() {
    let mut state = VideoTimelineState::new();
    state.observe_gap(&[21], 1.0, Some(91_000), "supply", "supply");
    state.mark_gap_expired(
        &[21],
        2.0,
        Some(91_000),
        "supply",
        "supply",
        Some("deadline"),
    );
    assert_eq!(state.chain_state(), ChainState::Broken);

    state.mark_frame_complete_candidate(91_001, 3.0, Some(false), "disposable");
    assert_eq!(state.chain_state(), ChainState::Broken);

    state.mark_frame_complete_candidate(91_002, 4.0, Some(true), "anchor");
    assert_eq!(state.chain_state(), ChainState::Broken);

    state.on_clean_anchor_ingress(91_002, 4.0);
    assert_eq!(state.chain_state(), ChainState::SustainingRecovery);
}

#[test]
fn estimated_arrival_near_deadline_low_value_disposable_gap_does_not_break_chain() {
    let mut state = VideoTimelineState::new();
    let chain_broken = state.mark_gap_expired(
        &[38024],
        2.0,
        None,
        "disposable",
        "unknown",
        Some("estimatedArrivalNearDeadlineLowValue"),
    );
    assert!(!chain_broken);
    assert_eq!(state.chain_state(), ChainState::Healthy);
}

#[test]
fn anonymous_cloud_low_value_disposable_gap_does_not_break_chain() {
    let mut state = VideoTimelineState::new();
    let chain_broken = state.mark_gap_expired(
        &[38022],
        2.0,
        None,
        "disposable",
        "unknown",
        Some("cloudHighRttLowValueAdmission"),
    );
    assert!(!chain_broken);
    assert_eq!(state.chain_state(), ChainState::Healthy);

    state.observe_frame(91_100, 3.0, Some(false), "disposable");
    state.mark_frame_complete_candidate(91_100, 4.0, Some(false), "disposable");
    assert_eq!(state.chain_state(), ChainState::Healthy);
    let observation = state.snapshot_for_observation(
        1,
        "frame-complete-candidate",
        Some(38022),
        Some(91_100),
        4.0,
    );
    assert_eq!(observation.chain.state, "healthy");
    assert_eq!(observation.chain.reason.as_deref(), None);

    state.mark_frame_complete_candidate(91_101, 5.0, Some(true), "anchor");
    assert_eq!(state.chain_state(), ChainState::Healthy);

    state.on_clean_anchor_ingress(91_101, 5.0);
    assert_eq!(state.chain_state(), ChainState::SustainingRecovery);
}

#[test]
fn anonymous_disposable_gap_low_value_admission_keeps_chain_healthy() {
    let mut state = VideoTimelineState::new();
    state.mark_gap_expired(
        &[38022],
        2.0,
        None,
        "disposable",
        "unknown",
        Some("cloudHighRttLowValueAdmission"),
    );

    state.observe_frame(91_100, 3.0, Some(false), "disposable");
    let observation =
        state.snapshot_for_observation(1, "frame-observed", Some(38022), Some(91_100), 3.0);
    assert_eq!(observation.chain.state, "healthy");
    assert_eq!(observation.chain.reason.as_deref(), None);
}

#[test]
fn inspection_reject_reason_projects_through_frame_and_chain_snapshot() {
    let mut state = VideoTimelineState::new();
    state.on_admission_await_recovery_keyframe(Some("inspectionRejectInvalidSliceHeader"));
    state.observe_frame(91_200, 3.0, None, "unknown");
    state.mark_frame_closed(
        91_200,
        3.0,
        None,
        "unknown",
        Some("inspectionRejectInvalidSliceHeader"),
    );

    let observation = state.snapshot_for_observation(
        1,
        "frame-inspection-rejected-await-anchor",
        None,
        Some(91_200),
        3.0,
    );
    assert_eq!(observation.chain.state, "recovering");
    assert_eq!(
        observation.chain.reason.as_deref(),
        Some("inspectionRejectInvalidSliceHeader")
    );
    assert_eq!(
        observation
            .frame
            .as_ref()
            .and_then(|frame| frame.close_reason.as_deref()),
        Some("inspectionRejectInvalidSliceHeader")
    );
    assert_eq!(
        observation.frame.as_ref().map(|frame| frame.state.as_str()),
        Some("closed")
    );
}

#[test]
fn wait_anchor_gate_creates_chain_debt_until_clean_anchor() {
    let mut state = VideoTimelineState::new();
    state.apply_wait_keyframe_gate(true);
    assert_eq!(state.chain_state(), ChainState::Recovering);

    state.observe_frame(90_010, 2.0, Some(false), "disposable");
    state.mark_frame_complete_candidate(90_010, 3.0, Some(false), "disposable");
    assert_eq!(state.chain_state(), ChainState::Recovering);

    state.apply_wait_keyframe_gate(false);
    assert_eq!(state.chain_state(), ChainState::Recovering);

    state.observe_frame(90_011, 4.0, Some(true), "anchor");
    state.mark_frame_complete_candidate(90_011, 5.0, Some(true), "anchor");
    assert_eq!(state.chain_state(), ChainState::Recovering);

    state.on_clean_anchor_ingress(90_011, 5.0);
    assert_eq!(state.chain_state(), ChainState::SustainingRecovery);
}

#[test]
fn recovering_chain_multiple_complete_candidates_cannot_whiten_without_clean_anchor() {
    let mut state = VideoTimelineState::new();
    state.on_admission_await_recovery_keyframe(Some("awaitingRecoveryAnchor"));
    assert_eq!(state.chain_state(), ChainState::Recovering);

    state.observe_frame(95_001, 2.0, Some(false), "disposable");
    state.mark_frame_complete_candidate(95_001, 3.0, Some(false), "disposable");
    assert_eq!(state.chain_state(), ChainState::Recovering);

    state.observe_frame(95_002, 130.0, Some(false), "disposable");
    state.mark_frame_complete_candidate(95_002, 132.0, Some(false), "disposable");
    assert_eq!(state.chain_state(), ChainState::Recovering);
}

#[test]
fn recovering_chain_only_clean_anchor_submission_can_return_healthy() {
    let mut state = VideoTimelineState::new();
    state.on_admission_await_recovery_keyframe(Some("awaitingRecoveryAnchor"));
    state.observe_frame(96_001, 2.0, Some(true), "anchor");
    state.mark_frame_complete_candidate(96_001, 3.0, Some(true), "anchor");
    assert_eq!(state.chain_state(), ChainState::Recovering);

    state.on_clean_anchor_ingress(96_001, 3.0);
    assert_eq!(state.chain_state(), ChainState::SustainingRecovery);
}

#[test]
fn expired_and_awaiting_debt_are_not_cleared_by_complete_candidate() {
    let mut broken = VideoTimelineState::new();
    broken.mark_gap_expired(
        &[31],
        1.0,
        Some(97_000),
        "supply",
        "supply",
        Some("deadline"),
    );
    assert_eq!(broken.chain_state(), ChainState::Broken);
    broken.observe_frame(97_001, 2.0, Some(false), "disposable");
    broken.mark_frame_complete_candidate(97_001, 3.0, Some(false), "disposable");
    assert_eq!(broken.chain_state(), ChainState::Broken);

    let mut awaiting = VideoTimelineState::new();
    awaiting.on_admission_await_recovery_keyframe(Some("awaitingRecoveryAnchor"));
    assert_eq!(awaiting.chain_state(), ChainState::Recovering);
    awaiting.observe_frame(98_001, 2.0, Some(false), "disposable");
    awaiting.mark_frame_complete_candidate(98_001, 3.0, Some(false), "disposable");
    assert_eq!(awaiting.chain_state(), ChainState::Recovering);
}

#[test]
fn clean_anchor_short_window_softens_disposable_reorder_reentry() {
    let mut state = VideoTimelineState::new();
    state.on_clean_anchor_ingress(110_001, 10.0);
    assert_eq!(state.chain_state(), ChainState::SustainingRecovery);

    state.mark_gap_reorder_pending(&[501], 10.5, None, "disposable", "disposable");
    assert_eq!(state.chain_state(), ChainState::SustainingRecovery);

    let chain_broken = state.mark_gap_expired(
        &[501],
        10.6,
        None,
        "disposable",
        "unknown",
        Some("awaitingRecoveryAnchor"),
    );
    assert!(!chain_broken);
    assert_eq!(state.chain_state(), ChainState::SustainingRecovery);
}

#[test]
fn clean_anchor_building_window_softens_disposable_expiry_without_frame_budget() {
    let mut state = VideoTimelineState::new();
    state.on_clean_anchor_ingress(120_001, 20.0);
    assert_eq!(state.chain_state(), ChainState::SustainingRecovery);

    for sequence in [601u16, 602, 603, 604, 605, 606] {
        let chain_broken = state.mark_gap_expired(
            &[sequence],
            20.1,
            None,
            "disposable",
            "unknown",
            Some("awaitingRecoveryAnchor"),
        );
        assert!(!chain_broken);
        assert_eq!(state.chain_state(), ChainState::SustainingRecovery);
    }
}

#[test]
fn clean_anchor_building_phase_does_not_relax_supply_break() {
    let mut state = VideoTimelineState::new();
    state.on_clean_anchor_ingress(130_001, 30.0);
    assert_eq!(state.chain_state(), ChainState::SustainingRecovery);

    let chain_broken = state.mark_gap_expired(
        &[701],
        30.1,
        Some(130_050),
        "supply",
        "supply",
        Some("awaitingRecoveryAnchor"),
    );
    assert!(chain_broken);
    assert_eq!(state.chain_state(), ChainState::Broken);
}

#[test]
fn observation_only_building_phase_softening_is_time_based() {
    let mut state = VideoTimelineState::new();
    state.on_clean_anchor_ingress(130_001, 30.0);
    assert_eq!(state.chain_state(), ChainState::SustainingRecovery);

    state.mark_gap_reorder_pending(&[741], 30.1, None, "disposable", "disposable");
    state.mark_gap_nack_candidate(&[741], 30.2, None, "disposable", "disposable");
    state.mark_gap_repair_in_flight(&[741], 30.3, None, "disposable", "disposable");
    state.mark_gap_resolved(741, 30.4, None, "disposable", "disposable");

    assert!(state.recovery_chain_building_phase_active(30.5, "disposable"));
    assert!(state.recovery_chain_building_phase_active(30.6, "disposable"));
    assert!(state.recovery_chain_building_phase_active(30.7, "disposable"));
    assert!(state.recovery_chain_building_phase_active(31.0, "disposable"));
}

#[test]
fn submit_side_building_phase_probe_is_time_based() {
    let mut state = VideoTimelineState::new();
    state.on_clean_anchor_ingress(130_101, 40.0);

    assert!(state.recovery_chain_building_phase_active(40.1, "disposable"));
    assert!(state.recovery_chain_building_phase_active(40.2, "disposable"));
    assert!(state.recovery_chain_building_phase_active(40.3, "disposable"));
    assert!(state.recovery_chain_building_phase_active(41.1, "disposable"));
}

#[test]
fn stale_wait_anchor_debt_reopens_to_sustaining_when_clean_anchor_allows_disposable_continuation() {
    let mut state = VideoTimelineState::new();
    state.on_clean_anchor_ingress(130_301, 60.0);
    state.on_admission_await_recovery_keyframe(Some("awaitingRecoveryAnchor"));
    assert_eq!(state.chain_state(), ChainState::Recovering);

    assert!(state.reopen_delta_continuation_after_clean_anchor(60.2));
    assert_eq!(state.chain_state(), ChainState::SustainingRecovery);
    assert!(!state.chain_requires_recovery_anchor());
    assert!(!state.has_hard_recovery_risk_for_test());
}

#[test]
fn repair_in_flight_hard_gap_does_not_block_clean_anchor_disposable_reopen() {
    let mut state = VideoTimelineState::new();
    state.on_clean_anchor_ingress(130_401, 70.0);
    state.on_admission_await_recovery_keyframe(Some("awaitingRecoveryAnchor"));
    state.mark_gap_repair_in_flight(&[841], 70.1, Some(130_450), "anchor", "anchor");
    assert_eq!(state.chain_state(), ChainState::Recovering);
    assert!(state.has_hard_recovery_risk_for_test());

    assert!(state.reopen_delta_continuation_after_clean_anchor(70.2));
    assert_eq!(state.chain_state(), ChainState::SustainingRecovery);
    assert!(!state.chain_requires_recovery_anchor());
}

#[test]
fn sustaining_recovery_timeout_falls_back_to_recovering() {
    let mut state = VideoTimelineState::new();
    state.on_clean_anchor_ingress(130_201, 50.0);
    assert_eq!(state.chain_state(), ChainState::SustainingRecovery);

    state.on_timeout_detected();

    assert_eq!(state.chain_state(), ChainState::Recovering);
    assert!(state.chain_requires_recovery_anchor());
    let observation =
        state.snapshot_for_observation(1, "timeout-stream-thin-stall", None, None, 51.0);
    assert_eq!(
        observation.chain.reason.as_deref(),
        Some("awaitRecoveryAnchor")
    );
}

#[test]
fn clean_anchor_submission_retires_older_reorder_debt() {
    let mut state = VideoTimelineState::new();
    state.observe_gap(&[901], 1.0, Some(100_001), "supply", "supply");
    state.mark_gap_reorder_pending(&[901], 2.0, Some(100_001), "supply", "supply");
    assert_eq!(state.chain_state(), ChainState::Repairing);

    state.on_clean_anchor_ingress(100_100, 10.0);

    assert_eq!(state.chain_state(), ChainState::SustainingRecovery);
    assert_eq!(state.gap_state_of(901), None);
}

#[test]
fn stable_continuation_can_retire_aged_older_reorder_debt() {
    let mut state = VideoTimelineState::new();
    state.observe_gap(&[911], 1.0, Some(100_001), "supply", "supply");
    state.mark_gap_reorder_pending(&[911], 2.0, Some(100_001), "supply", "supply");
    assert_eq!(state.chain_state(), ChainState::Repairing);

    state.observe_frame(100_100, 10.0, Some(false), "disposable");
    state.mark_frame_complete_candidate(100_100, 20.0, Some(false), "disposable");
    assert_eq!(state.chain_state(), ChainState::Repairing);
    assert_eq!(state.gap_state_of(911), Some(GapState::ReorderPending));

    state.observe_frame(100_101, 300.0, Some(false), "disposable");
    state.mark_frame_complete_candidate(100_101, 320.0, Some(false), "disposable");

    assert_eq!(state.gap_state_of(911), None);
    assert_eq!(state.chain_state(), ChainState::Healthy);
}

#[test]
fn stable_continuation_does_not_retire_newer_reorder_debt() {
    let mut state = VideoTimelineState::new();
    state.observe_gap(&[921], 1.0, Some(100_200), "supply", "supply");
    state.mark_gap_reorder_pending(&[921], 2.0, Some(100_200), "supply", "supply");
    assert_eq!(state.chain_state(), ChainState::Repairing);

    state.observe_frame(100_100, 10.0, Some(false), "disposable");
    state.mark_frame_complete_candidate(100_100, 20.0, Some(false), "disposable");
    state.observe_frame(100_101, 300.0, Some(false), "disposable");
    state.mark_frame_complete_candidate(100_101, 320.0, Some(false), "disposable");

    assert_eq!(state.gap_state_of(921), Some(GapState::ReorderPending));
    assert_eq!(state.chain_state(), ChainState::Repairing);
}

#[test]
fn clean_anchor_candidate_peek_does_not_consume_pending_until_ack() {
    let mut state = VideoTimelineState::new();
    state.on_clean_anchor_ingress(150_001, 10.0);

    assert_eq!(
        state.peek_clean_anchor_stats_commit_candidate_if_stable(150_001, 12.0),
        Some(150_001)
    );
    // 第二次 peek 仍能命中，说明 source 阶段不会提前消费 pending。
    assert_eq!(
        state.peek_clean_anchor_stats_commit_candidate_if_stable(150_001, 13.0),
        Some(150_001)
    );

    assert!(state.ack_clean_anchor_stats_committed(150_001));
    assert_eq!(
        state.peek_clean_anchor_stats_commit_candidate_if_stable(150_001, 14.0),
        None
    );
}

#[test]
fn local_backpressure_disposable_gap_does_not_break_chain_or_mark_hard_risk() {
    let mut state = VideoTimelineState::new();

    let chain_broken = state.mark_gap_expired(
        &[801],
        40.0,
        None,
        "disposable",
        "unknown",
        Some("localBackpressureDeltaGap"),
    );

    assert!(!chain_broken);
    assert_eq!(state.chain_state(), ChainState::Healthy);
    assert!(!state.has_hard_recovery_risk_for_test());
}

#[test]
fn display_starved_low_value_disposable_gap_does_not_break_chain_or_mark_hard_risk() {
    let mut state = VideoTimelineState::new();

    let chain_broken = state.mark_gap_expired(
        &[811],
        40.0,
        None,
        "disposable",
        "unknown",
        Some("displayStarvedLowValueAdmission"),
    );

    assert!(!chain_broken);
    assert_eq!(state.chain_state(), ChainState::Healthy);
    assert!(!state.has_hard_recovery_risk_for_test());
}

#[test]
fn anonymous_disposable_wait_reason_without_local_provenance_remains_hard_risk() {
    let mut state = VideoTimelineState::new();

    let chain_broken = state.mark_gap_expired(
        &[802],
        41.0,
        None,
        "disposable",
        "unknown",
        Some("awaitingRecoveryAnchor"),
    );

    assert!(chain_broken);
    assert_eq!(state.chain_state(), ChainState::Broken);
    assert!(state.has_hard_recovery_risk_for_test());
}

#[test]
fn anonymous_budget_supply_gap_expire_does_not_break_chain() {
    let mut state = VideoTimelineState::new();
    assert!(!state.mark_gap_expired(
        &[33_221],
        100.0,
        None,
        "supply",
        "unknown",
        Some("deadline"),
    ));
    assert_eq!(state.chain_state(), ChainState::Healthy);
}

#[test]
fn gap_snapshot_splits_budget_and_evidence_for_anonymous_transport() {
    let mut state = VideoTimelineState::new();
    state.observe_gap(&[5], 1.0, None, "supply", "unknown");
    let obs = state.snapshot_for_observation(1, "t", Some(5), None, 2.0);
    let gap = obs.gap.expect("gap");
    assert_eq!(gap.budget_importance.as_deref(), Some("supply"));
    assert_eq!(gap.evidence_importance.as_deref(), Some("unknown"));
    assert_eq!(gap.gap_dependency_confidence.as_deref(), Some("anonymous"));
}

#[test]
fn later_budget_update_can_downgrade_gap_importance_lane() {
    let mut state = VideoTimelineState::new();
    state.observe_gap(&[6], 1.0, None, "supply", "unknown");
    state.mark_gap_nack_candidate(&[6], 2.0, None, "disposable", "unknown");
    let obs = state.snapshot_for_observation(1, "t", Some(6), None, 3.0);
    assert_eq!(
        obs.gap.expect("gap").budget_importance.as_deref(),
        Some("disposable")
    );
}
