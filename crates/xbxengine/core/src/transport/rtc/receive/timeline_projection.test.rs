use super::receiver_state::ReceiverState;
use super::timeline_projection::project_latest_video_timeline_observation;
use super::trace_ledger::ReceiverTraceLedger;

#[test]
fn projection_chain_state_follows_receiver_state() {
    let mut ledger = ReceiverTraceLedger::new();
    ledger.mark_gap_reorder_pending(&[100], 1.0, Some(90_000), "supply", "supply");

    let observation = project_latest_video_timeline_observation(
        ReceiverState::Repairing,
        &ledger,
        1,
        "gap-repair-in-flight",
        Some(100),
        Some(90_000),
        2.0,
    );

    assert_eq!(observation.chain.state, "repairing");
    assert_eq!(
        observation.chain.reason.as_deref(),
        Some("gapRepairInFlight")
    );
    assert_eq!(observation.source_event, "gap-repair-in-flight");
    assert!(observation.gap.is_some());
}

#[test]
fn waiting_keyframe_receiver_state_overrides_ledger_gap_facts() {
    let mut ledger = ReceiverTraceLedger::new();
    ledger.mark_gap_reorder_pending(&[200], 1.0, Some(91_000), "supply", "supply");

    let observation = project_latest_video_timeline_observation(
        ReceiverState::WaitingKeyframe,
        &ledger,
        2,
        "await-recovery-keyframe",
        Some(200),
        Some(91_000),
        3.0,
    );

    assert_eq!(observation.chain.state, "waiting-keyframe");
    assert_eq!(
        observation.chain.reason.as_deref(),
        Some("receiverWaitingKeyframe")
    );
}
