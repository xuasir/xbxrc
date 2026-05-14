use super::{
    NackObservePolicy, NackPollResult, NackScheduler, NackSchedulerConfig,
    PacketRecoveryDisposition,
};
use crate::media::video::ingress::budget::FrameBudgetContext;
use crate::media::video::types::FrameValue;

fn base_policy() -> NackObservePolicy {
    NackObservePolicy {
        source: "sampleLoss",
        deadline_at_ms: Some(1_050.0),
        max_age_ms: Some(200),
        retry_interval_ms: Some(10),
        burst_count: Some(2),
        max_tracked_sequences: Some(4),
        frame_rtp_timestamp: Some(90_000),
        frame_is_keyframe: Some(false),
        frame_importance: "disposable",
        priority: 1,
        budget_context: FrameBudgetContext::steady_for_value(FrameValue::new(
            false,
            false,
            12 * 1024,
        )),
        estimated_recovery_arrival_ms: Some(1_020.0),
        frame_playout_deadline_at_ms: Some(1_050.0),
        nack_disposition: PacketRecoveryDisposition::Attempted,
        frame_unrecoverable_reason: None,
        max_retry_count_override: None,
        first_attempt_survival_window_ms: None,
        repairability_schedule: None,
        admission_deadline_floor_at_ms: None,
    }
}

fn retry_or_budget_exhausted_sequences(polled: &NackPollResult) -> Vec<u16> {
    if let Some(batch) = &polled.retry_batch {
        return batch.sequences.clone();
    }
    polled
        .expired_batches
        .iter()
        .find(|batch| batch.reason == "singleShotPollComplete")
        .map(|batch| batch.sequences.clone())
        .unwrap_or_default()
}

#[test]
fn admission_deadline_exceeded_does_not_enter_pending() {
    let mut scheduler = NackScheduler::new(NackSchedulerConfig {
        max_age_ms: 200,
        frame_deadline_ms: 120,
        burst_count: 2,
        retry_interval_ms: 40,
        max_retry_count: 3,
    });
    let mut policy = base_policy();
    policy.deadline_at_ms = Some(1_000.0);
    let (batch, skipped) =
        scheduler.observe_missing_sequences_with_policy(&[10, 11], 1_000.0, policy);
    assert!(batch.is_none());
    assert_eq!(scheduler.pending_count(), 0);
    let skipped = skipped.expect("skipped");
    assert_eq!(
        skipped.nack_disposition,
        PacketRecoveryDisposition::SkippedTooLate
    );
    assert_eq!(
        skipped.frame_unrecoverable_reason,
        Some("deadlineExceededBeforeAdmission")
    );
}

#[test]
fn admission_skipped_low_value_does_not_enter_pending() {
    let mut scheduler = NackScheduler::new(NackSchedulerConfig {
        max_age_ms: 200,
        frame_deadline_ms: 120,
        burst_count: 2,
        retry_interval_ms: 40,
        max_retry_count: 3,
    });
    let mut policy = base_policy();
    policy.nack_disposition = PacketRecoveryDisposition::SkippedLowValue;
    policy.frame_unrecoverable_reason = Some("cloudHighRttLowValueAdmission");
    let (batch, skipped) =
        scheduler.observe_missing_sequences_with_policy(&[10, 11], 1_000.0, policy);
    assert!(batch.is_none());
    assert_eq!(scheduler.pending_count(), 0);
    let skipped = skipped.expect("skipped");
    assert_eq!(
        skipped.nack_disposition,
        PacketRecoveryDisposition::SkippedLowValue
    );
    assert_eq!(
        skipped.frame_unrecoverable_reason,
        Some("cloudHighRttLowValueAdmission")
    );
}

#[test]
fn cloud_high_rtt_low_value_admission_keeps_supply_packets_repairable() {
    let mut scheduler = NackScheduler::new(NackSchedulerConfig {
        max_age_ms: 200,
        frame_deadline_ms: 120,
        burst_count: 2,
        retry_interval_ms: 40,
        max_retry_count: 3,
    });
    let mut policy = base_policy();
    policy.frame_importance = "supply";
    policy.priority = 2;
    policy.nack_disposition = PacketRecoveryDisposition::SkippedLowValue;
    policy.frame_unrecoverable_reason = Some("cloudHighRttLowValueAdmission");

    let (batch, skipped) =
        scheduler.observe_missing_sequences_with_policy(&[10, 11], 1_000.0, policy);
    assert!(skipped.is_none());
    let batch = batch.expect("supply batch should be attempted");
    assert_eq!(batch.sequences, vec![10, 11]);
    assert_eq!(batch.nack_disposition, PacketRecoveryDisposition::Attempted);
    assert_eq!(batch.frame_unrecoverable_reason, None);
    assert_eq!(scheduler.pending_count(), 2);
}

#[test]
fn admission_skipped_low_value_is_throttled_per_sequence() {
    let mut scheduler = NackScheduler::new(NackSchedulerConfig {
        max_age_ms: 500,
        frame_deadline_ms: 2_000,
        burst_count: 2,
        retry_interval_ms: 40,
        max_retry_count: 3,
    });
    let mut policy = base_policy();
    policy.deadline_at_ms = Some(2_000.0);
    policy.nack_disposition = PacketRecoveryDisposition::SkippedLowValue;
    policy.frame_unrecoverable_reason = Some("cloudHighRttLowValueAdmission");

    let (first_batch, first_skipped) =
        scheduler.observe_missing_sequences_with_policy(&[10], 1_000.0, policy);
    assert!(first_batch.is_none());
    let first_skipped = first_skipped.expect("first skipped");
    assert_eq!(first_skipped.sequences, vec![10]);
    assert_eq!(scheduler.pending_count(), 0);

    let (second_batch, second_skipped) =
        scheduler.observe_missing_sequences_with_policy(&[10], 1_120.0, policy);
    assert!(second_batch.is_none());
    assert!(second_skipped.is_none());
    assert_eq!(scheduler.pending_count(), 0);

    let (third_batch, third_skipped) =
        scheduler.observe_missing_sequences_with_policy(&[10], 1_280.0, policy);
    assert!(third_batch.is_none());
    let third_skipped = third_skipped.expect("third skipped");
    assert_eq!(third_skipped.sequences, vec![10]);
}

#[test]
fn low_value_admission_does_not_skip_anchor_recovery() {
    let mut scheduler = NackScheduler::new(NackSchedulerConfig {
        max_age_ms: 500,
        frame_deadline_ms: 2_000,
        burst_count: 2,
        retry_interval_ms: 40,
        max_retry_count: 3,
    });
    let mut policy = base_policy();
    policy.deadline_at_ms = Some(2_000.0);
    policy.nack_disposition = PacketRecoveryDisposition::SkippedLowValue;
    policy.frame_unrecoverable_reason = Some("cloudHighRttLowValueAdmission");
    policy.frame_is_keyframe = Some(true);
    policy.frame_importance = "anchor";

    let (batch, skipped) =
        scheduler.observe_missing_sequences_with_policy(&[10, 11], 1_000.0, policy);
    assert!(skipped.is_none());
    let batch = batch.expect("batch");
    assert_eq!(batch.sequences, vec![10, 11]);
    assert_eq!(scheduler.pending_count(), 2);
}

#[test]
fn admission_skipped_too_late_is_not_throttled_by_low_value_cache() {
    let mut scheduler = NackScheduler::new(NackSchedulerConfig {
        max_age_ms: 200,
        frame_deadline_ms: 120,
        burst_count: 2,
        retry_interval_ms: 40,
        max_retry_count: 3,
    });
    let mut policy = base_policy();
    policy.deadline_at_ms = Some(1_000.0);
    policy.nack_disposition = PacketRecoveryDisposition::SkippedTooLate;
    policy.frame_unrecoverable_reason = Some("deadlineExceededBeforeAdmission");

    let (_, first_skipped) =
        scheduler.observe_missing_sequences_with_policy(&[11], 1_200.0, policy);
    let first_skipped = first_skipped.expect("first skipped");
    assert_eq!(first_skipped.sequences, vec![11]);

    let (_, second_skipped) =
        scheduler.observe_missing_sequences_with_policy(&[11], 1_230.0, policy);
    let second_skipped = second_skipped.expect("second skipped");
    assert_eq!(second_skipped.sequences, vec![11]);
    assert_eq!(
        second_skipped.frame_unrecoverable_reason,
        Some("deadlineExceededBeforeAdmission")
    );
}

#[test]
fn existing_pending_merges_with_stricter_and_more_aggressive_policy() {
    let mut scheduler = NackScheduler::new(NackSchedulerConfig {
        max_age_ms: 500,
        frame_deadline_ms: 2_000,
        burst_count: 1,
        retry_interval_ms: 40,
        max_retry_count: 1,
    });
    let mut first_policy = base_policy();
    first_policy.source = "rtpWindow";
    first_policy.deadline_at_ms = Some(2_000.0);
    first_policy.retry_interval_ms = Some(30);
    first_policy.max_age_ms = Some(400);
    first_policy.frame_is_keyframe = Some(true);
    first_policy.frame_importance = "anchor";
    first_policy.priority = 3;
    first_policy.max_tracked_sequences = Some(1);
    first_policy.max_retry_count_override = Some(1);
    let (first_batch, skipped) =
        scheduler.observe_missing_sequences_with_policy(&[33], 1_000.0, first_policy);
    assert!(skipped.is_none());
    assert_eq!(first_batch.expect("first batch").sequences, vec![33]);

    let mut second_policy = base_policy();
    second_policy.source = "rtpGap";
    second_policy.deadline_at_ms = Some(1_500.0);
    second_policy.retry_interval_ms = Some(10);
    second_policy.max_age_ms = Some(120);
    second_policy.frame_is_keyframe = Some(true);
    second_policy.frame_importance = "anchor";
    second_policy.max_tracked_sequences = Some(1);
    second_policy.priority = 3;
    second_policy.max_retry_count_override = Some(1);
    let (second_batch, second_skipped) =
        scheduler.observe_missing_sequences_with_policy(&[33], 1_001.0, second_policy);
    assert!(second_batch.is_none());
    assert!(second_skipped.is_none());

    let retry = scheduler.poll(1_011.0);
    let retry_batch = retry.retry_batch.as_ref().expect("poll retry");
    assert_eq!(retry_batch.sequences, vec![33]);

    let expired = scheduler.poll(1_510.0);
    assert!(expired.retry_batch.is_none());
    let deadline = expired
        .expired_batches
        .iter()
        .find(|batch| batch.reason == "deadline")
        .expect("deadline expiry");
    assert_eq!(deadline.source, "rtpGap");
    assert_eq!(deadline.sequences, vec![33]);
}

#[test]
fn retry_budget_exhausted_is_finalized_and_dequeued() {
    let mut scheduler = NackScheduler::new(NackSchedulerConfig {
        max_age_ms: 200,
        frame_deadline_ms: 500,
        burst_count: 1,
        retry_interval_ms: 10,
        max_retry_count: 1,
    });
    let mut policy = base_policy();
    policy.frame_is_keyframe = Some(true);
    policy.frame_importance = "anchor";
    policy.priority = 3;
    policy.max_retry_count_override = Some(1);
    let expected_budget_context = policy.budget_context;

    let (initial_batch, skipped) =
        scheduler.observe_missing_sequences_with_policy(&[20], 1_000.0, policy);
    assert!(skipped.is_none());
    let initial_batch = initial_batch.expect("initial");
    assert_eq!(initial_batch.sequences, vec![20]);
    assert_eq!(initial_batch.budget_context, expected_budget_context);
    assert_eq!(scheduler.pending_count(), 1);

    let first_retry = scheduler.poll(1_010.0);
    let retry_batch = first_retry.retry_batch.as_ref().expect("poll retry");
    assert_eq!(retry_batch.sequences, vec![20]);
    assert_eq!(retry_batch.budget_context, expected_budget_context);
    assert!(first_retry.expired_batches.is_empty());
    assert_eq!(scheduler.pending_count(), 1);

    let exhausted = scheduler.poll(1_020.0);
    assert!(exhausted.retry_batch.is_none());
    assert_eq!(scheduler.pending_count(), 0);
    let retry_budget = exhausted
        .expired_batches
        .iter()
        .find(|batch| batch.reason == "singleShotPollComplete")
        .expect("retry budget exhausted");
    assert_eq!(retry_budget.sequences, vec![20]);
    assert_eq!(retry_budget.budget_context, expected_budget_context);
}

#[test]
fn supply_packet_with_supply_priority_gets_up_to_two_poll_retries() {
    let mut scheduler = NackScheduler::new(NackSchedulerConfig {
        max_age_ms: 200,
        frame_deadline_ms: 500,
        burst_count: 1,
        retry_interval_ms: 10,
        max_retry_count: 3,
    });
    let mut policy = base_policy();
    policy.frame_is_keyframe = Some(false);
    policy.frame_importance = "supply";
    policy.priority = 2;
    policy.max_retry_count_override = Some(3);

    let (initial_batch, skipped) =
        scheduler.observe_missing_sequences_with_policy(&[60], 1_000.0, policy);
    assert!(skipped.is_none());
    assert_eq!(initial_batch.expect("initial").sequences, vec![60]);
    assert_eq!(scheduler.pending_count(), 1);

    let first_retry = scheduler.poll(1_010.0);
    assert_eq!(retry_or_budget_exhausted_sequences(&first_retry), vec![60]);

    let second_retry = scheduler.poll(1_020.0);
    if !second_retry.retry_batch.is_none() {
        assert_eq!(retry_or_budget_exhausted_sequences(&second_retry), vec![60]);
    }

    let third_retry = scheduler.poll(1_030.0);
    assert_eq!(retry_or_budget_exhausted_sequences(&third_retry), vec![60]);

    let exhausted = scheduler.poll(1_040.0);
    assert!(exhausted.retry_batch.is_none());
    assert_eq!(scheduler.pending_count(), 0);
    assert!(exhausted
        .expired_batches
        .iter()
        .any(|batch| batch.reason == "singleShotPollComplete"));
}

#[test]
fn chain_broken_flush_removes_non_anchor_pending() {
    let mut scheduler = NackScheduler::new(NackSchedulerConfig {
        max_age_ms: 200,
        frame_deadline_ms: 500,
        burst_count: 4,
        retry_interval_ms: 10,
        max_retry_count: 3,
    });

    let mut disposable_policy = base_policy();
    disposable_policy.frame_is_keyframe = Some(false);
    disposable_policy.frame_importance = "disposable";
    let expected_disposable_budget_context = disposable_policy.budget_context;
    let _ = scheduler.observe_missing_sequences_with_policy(&[30, 31], 1_000.0, disposable_policy);

    let mut anchor_policy = base_policy();
    anchor_policy.frame_is_keyframe = Some(true);
    anchor_policy.frame_importance = "anchor";
    anchor_policy.priority = 3;
    let _ = scheduler.observe_missing_sequences_with_policy(&[40], 1_000.0, anchor_policy);

    assert_eq!(scheduler.pending_count(), 3);
    let flushed = scheduler
        .flush_non_keyframe_pending("flushedAfterChainBrokenAdmission")
        .expect("flushed");
    assert_eq!(flushed.reason, "chainBroken");
    assert_eq!(
        flushed.nack_disposition,
        PacketRecoveryDisposition::SkippedChainBroken
    );
    assert_eq!(flushed.sequences, vec![30, 31]);
    assert_eq!(
        flushed.frame_unrecoverable_reason,
        Some("flushedAfterChainBrokenAdmission")
    );
    assert_eq!(flushed.budget_context, expected_disposable_budget_context);
    assert_eq!(scheduler.pending_count(), 1);
}

#[test]
fn resolved_nack_preserves_budget_context() {
    let mut scheduler = NackScheduler::new(NackSchedulerConfig {
        max_age_ms: 200,
        frame_deadline_ms: 500,
        burst_count: 1,
        retry_interval_ms: 10,
        max_retry_count: 3,
    });
    let policy = base_policy();
    let expected_budget_context = policy.budget_context;

    let (initial_batch, skipped) =
        scheduler.observe_missing_sequences_with_policy(&[88], 1_000.0, policy);
    assert!(skipped.is_none());
    assert_eq!(initial_batch.expect("initial").sequences, vec![88]);

    let resolved = scheduler
        .resolve_sequence(88, 1_005.0)
        .expect("resolved sequence");
    assert_eq!(resolved.sequence, 88);
    assert_eq!(resolved.budget_context, expected_budget_context);
}

#[test]
fn disposable_packet_has_no_retry_budget_and_clears_at_deadline_poll() {
    let mut scheduler = NackScheduler::new(NackSchedulerConfig {
        max_age_ms: 200,
        frame_deadline_ms: 500,
        burst_count: 1,
        retry_interval_ms: 10,
        max_retry_count: 3,
    });
    let mut policy = base_policy();
    policy.frame_is_keyframe = Some(false);
    policy.frame_importance = "disposable";
    policy.priority = 1;

    let (initial_batch, skipped) =
        scheduler.observe_missing_sequences_with_policy(&[50], 1_000.0, policy);
    assert!(skipped.is_none());
    assert_eq!(initial_batch.expect("initial").sequences, vec![50]);
    assert_eq!(scheduler.pending_count(), 1);

    let polled = scheduler.poll(1_010.0);
    assert!(polled.retry_batch.is_none());
    assert!(polled.expired_batches.is_empty());
    assert_eq!(scheduler.pending_count(), 1);

    let cleared = scheduler.poll(1_050.0);
    assert!(cleared.retry_batch.is_none());
    assert_eq!(scheduler.pending_count(), 0);
    assert_eq!(cleared.expired_batches.len(), 1);
    assert_eq!(cleared.expired_batches[0].reason, "deadline");
    assert_eq!(
        cleared.expired_batches[0].frame_unrecoverable_reason,
        Some("deadlineExceeded")
    );
}

#[test]
fn skipped_chain_broken_admission_preserves_unrecoverable_reason_contract() {
    let mut scheduler = NackScheduler::new(NackSchedulerConfig {
        max_age_ms: 200,
        frame_deadline_ms: 120,
        burst_count: 2,
        retry_interval_ms: 40,
        max_retry_count: 3,
    });
    let mut policy = base_policy();
    policy.nack_disposition = PacketRecoveryDisposition::SkippedChainBroken;
    policy.frame_unrecoverable_reason = Some("awaitingRecoveryAnchor");
    let (batch, skipped) =
        scheduler.observe_missing_sequences_with_policy(&[70, 71], 1_000.0, policy);
    assert!(batch.is_none());
    assert_eq!(scheduler.pending_count(), 0);
    let skipped = skipped.expect("skipped");
    assert_eq!(
        skipped.nack_disposition,
        PacketRecoveryDisposition::SkippedChainBroken
    );
    assert_eq!(
        skipped.frame_unrecoverable_reason,
        Some("awaitingRecoveryAnchor")
    );
}

#[test]
fn poll_prioritizes_high_value_batches_under_burst_limit() {
    let mut scheduler = NackScheduler::new(NackSchedulerConfig {
        max_age_ms: 300,
        frame_deadline_ms: 500,
        burst_count: 2,
        retry_interval_ms: 10,
        max_retry_count: 3,
    });

    let mut supply_policy = base_policy();
    supply_policy.frame_importance = "supply";
    supply_policy.priority = 2;
    supply_policy.frame_rtp_timestamp = Some(90_100);
    supply_policy.deadline_at_ms = Some(1_200.0);
    supply_policy.max_retry_count_override = Some(2);
    let (supply_batch, supply_skipped) =
        scheduler.observe_missing_sequences_with_policy(&[10, 11], 1_000.0, supply_policy);
    assert!(supply_skipped.is_none());
    assert_eq!(
        supply_batch.expect("supply initial").sequences,
        vec![10, 11]
    );

    let mut anchor_policy = base_policy();
    anchor_policy.frame_is_keyframe = Some(true);
    anchor_policy.frame_importance = "anchor";
    anchor_policy.priority = 3;
    anchor_policy.frame_rtp_timestamp = Some(90_200);
    anchor_policy.deadline_at_ms = Some(1_200.0);
    anchor_policy.max_retry_count_override = Some(2);
    let (anchor_batch, anchor_skipped) =
        scheduler.observe_missing_sequences_with_policy(&[20, 21], 1_001.0, anchor_policy);
    assert!(anchor_skipped.is_none());
    assert_eq!(
        anchor_batch.expect("anchor initial").sequences,
        vec![20, 21]
    );

    let polled = scheduler.poll(1_011.0);
    let retry_batch = polled.retry_batch.as_ref().expect("poll retry");
    assert!(retry_batch.sequences.contains(&20));
    assert!(retry_batch.sequences.contains(&21));
    assert_eq!(retry_batch.frame_importance, "anchor");
    assert_eq!(retry_batch.frame_rtp_timestamp, Some(90_200));
    assert!(scheduler.pending_count() >= 2);
}

#[test]
fn poll_reports_deadline_and_max_age_expiry_in_same_tick() {
    let mut scheduler = NackScheduler::new(NackSchedulerConfig {
        max_age_ms: 50,
        frame_deadline_ms: 500,
        burst_count: 1,
        retry_interval_ms: 10,
        max_retry_count: 3,
    });

    let mut deadline_policy = base_policy();
    deadline_policy.deadline_at_ms = Some(1_010.0);
    deadline_policy.max_age_ms = Some(100);
    deadline_policy.estimated_recovery_arrival_ms = Some(1_005.0);
    deadline_policy.frame_rtp_timestamp = Some(90_300);
    let _ = scheduler.observe_missing_sequences_with_policy(&[30], 1_000.0, deadline_policy);

    let mut max_age_policy = base_policy();
    max_age_policy.deadline_at_ms = Some(1_200.0);
    max_age_policy.max_age_ms = Some(5);
    max_age_policy.frame_rtp_timestamp = Some(90_400);
    let _ = scheduler.observe_missing_sequences_with_policy(&[40], 1_000.0, max_age_policy);

    let mut retry_policy = base_policy();
    retry_policy.deadline_at_ms = Some(1_200.0);
    retry_policy.max_age_ms = Some(100);
    retry_policy.frame_is_keyframe = Some(true);
    retry_policy.frame_importance = "anchor";
    retry_policy.priority = 3;
    retry_policy.frame_rtp_timestamp = Some(90_500);
    retry_policy.max_retry_count_override = Some(1);
    let _ = scheduler.observe_missing_sequences_with_policy(&[50], 1_000.0, retry_policy);

    let polled = scheduler.poll(1_010.0);
    let retry_batch = polled.retry_batch.as_ref().expect("poll retry");
    assert_eq!(retry_batch.sequences, vec![50]);
    assert_eq!(retry_batch.frame_rtp_timestamp, Some(90_500));

    let deadline = polled
        .expired_batches
        .iter()
        .find(|batch| batch.reason == "deadline")
        .expect("deadline batch");
    assert_eq!(deadline.sequences, vec![30]);
    let max_age = polled
        .expired_batches
        .iter()
        .find(|batch| batch.reason == "maxAge")
        .expect("max-age batch");
    assert_eq!(max_age.sequences, vec![40]);
}

#[test]
fn attempted_admission_is_not_blocked_by_prior_low_value_skip_cache() {
    let mut scheduler = NackScheduler::new(NackSchedulerConfig {
        max_age_ms: 500,
        frame_deadline_ms: 500,
        burst_count: 2,
        retry_interval_ms: 10,
        max_retry_count: 3,
    });

    let mut skipped_policy = base_policy();
    skipped_policy.deadline_at_ms = Some(1_500.0);
    skipped_policy.nack_disposition = PacketRecoveryDisposition::SkippedLowValue;
    skipped_policy.frame_unrecoverable_reason = Some("cloudHighRttLowValueAdmission");

    let (skipped_batch, skipped) =
        scheduler.observe_missing_sequences_with_policy(&[80], 1_000.0, skipped_policy);
    assert!(skipped_batch.is_none());
    assert_eq!(skipped.expect("skipped").sequences, vec![80]);
    assert_eq!(scheduler.pending_count(), 0);

    let mut attempted_policy = base_policy();
    attempted_policy.deadline_at_ms = Some(1_500.0);
    let (attempted_batch, attempted_skipped) =
        scheduler.observe_missing_sequences_with_policy(&[80], 1_120.0, attempted_policy);
    assert!(attempted_skipped.is_none());
    assert_eq!(attempted_batch.expect("attempted").sequences, vec![80]);
    assert_eq!(scheduler.pending_count(), 1);
}

#[test]
fn flush_non_anchor_pending_keeps_anchor_retryable() {
    let mut scheduler = NackScheduler::new(NackSchedulerConfig {
        max_age_ms: 300,
        frame_deadline_ms: 500,
        burst_count: 1,
        retry_interval_ms: 10,
        max_retry_count: 3,
    });

    let mut supply_policy = base_policy();
    supply_policy.frame_importance = "supply";
    supply_policy.priority = 2;
    supply_policy.deadline_at_ms = Some(1_200.0);
    let _ = scheduler.observe_missing_sequences_with_policy(&[90], 1_000.0, supply_policy);

    let mut anchor_policy = base_policy();
    anchor_policy.frame_is_keyframe = Some(true);
    anchor_policy.frame_importance = "anchor";
    anchor_policy.priority = 3;
    anchor_policy.deadline_at_ms = Some(1_200.0);
    anchor_policy.max_retry_count_override = Some(1);
    let _ = scheduler.observe_missing_sequences_with_policy(&[91], 1_000.0, anchor_policy);

    let flushed = scheduler
        .flush_non_keyframe_pending("flushedAfterChainBrokenAdmission")
        .expect("flushed");
    assert_eq!(flushed.sequences, vec![90]);
    assert_eq!(flushed.reason, "chainBroken");
    assert_eq!(scheduler.pending_count(), 1);

    let polled = scheduler.poll(1_010.0);
    assert_eq!(retry_or_budget_exhausted_sequences(&polled), vec![91]);
}

#[test]
fn poll_prioritizes_unsent_overlap_candidates_before_earlier_retries() {
    let mut scheduler = NackScheduler::new(NackSchedulerConfig {
        max_age_ms: 500,
        frame_deadline_ms: 2_000,
        burst_count: 2,
        retry_interval_ms: 10,
        max_retry_count: 3,
    });
    let mut policy = base_policy();
    policy.deadline_at_ms = Some(2_000.0);
    policy.frame_is_keyframe = Some(true);
    policy.frame_importance = "anchor";
    policy.priority = 3;
    policy.max_retry_count_override = Some(3);

    let (first_batch, first_skipped) =
        scheduler.observe_missing_sequences_with_policy(&[10, 11, 12, 13], 1_000.0, policy);
    assert!(first_skipped.is_none());
    assert_eq!(first_batch.expect("first batch").sequences, vec![10, 11]);

    let (second_batch, second_skipped) =
        scheduler.observe_missing_sequences_with_policy(&[12, 13, 14, 15], 1_001.0, policy);
    assert!(second_skipped.is_none());
    assert_eq!(second_batch.expect("second batch").sequences, vec![14, 15]);
    assert_eq!(scheduler.pending_count(), 6);

    let polled = scheduler.poll(1_009.0);
    let selected = retry_or_budget_exhausted_sequences(&polled);
    assert!(selected.contains(&12));
    assert!(selected.contains(&13));
}

#[test]
fn low_value_skip_cache_does_not_block_later_attempted_admission() {
    let mut scheduler = NackScheduler::new(NackSchedulerConfig {
        max_age_ms: 500,
        frame_deadline_ms: 2_000,
        burst_count: 1,
        retry_interval_ms: 40,
        max_retry_count: 3,
    });
    let mut skipped_policy = base_policy();
    skipped_policy.deadline_at_ms = Some(2_000.0);
    skipped_policy.nack_disposition = PacketRecoveryDisposition::SkippedLowValue;
    skipped_policy.frame_unrecoverable_reason = Some("cloudHighRttLowValueAdmission");

    let (first_batch, first_skipped) =
        scheduler.observe_missing_sequences_with_policy(&[10], 1_000.0, skipped_policy);
    assert!(first_batch.is_none());
    assert_eq!(first_skipped.expect("first skipped").sequences, vec![10]);
    assert_eq!(scheduler.pending_count(), 0);

    let mut attempted_policy = skipped_policy;
    attempted_policy.nack_disposition = PacketRecoveryDisposition::Attempted;
    attempted_policy.frame_unrecoverable_reason = None;
    let (second_batch, second_skipped) =
        scheduler.observe_missing_sequences_with_policy(&[10], 1_100.0, attempted_policy);
    assert!(second_skipped.is_none());
    let second_batch = second_batch.expect("attempted batch");
    assert_eq!(second_batch.sequences, vec![10]);
    assert_eq!(
        second_batch.nack_disposition,
        PacketRecoveryDisposition::Attempted
    );
    assert_eq!(scheduler.pending_count(), 1);
}

#[test]
fn poll_separates_deadline_max_age_and_retry_budget_expirations() {
    // 单次 tick 内同时出现 deadline / maxAge / poll 重试预算耗尽（singleShotPollComplete）。
    let mut scheduler = NackScheduler::new(NackSchedulerConfig {
        max_age_ms: 200,
        frame_deadline_ms: 500,
        burst_count: 1,
        retry_interval_ms: 10,
        max_retry_count: 1,
    });

    let mut retry_budget_policy = base_policy();
    retry_budget_policy.deadline_at_ms = Some(1_100.0);
    retry_budget_policy.frame_is_keyframe = Some(true);
    retry_budget_policy.frame_importance = "anchor";
    retry_budget_policy.priority = 3;
    retry_budget_policy.max_retry_count_override = Some(1);
    let (retry_initial, retry_skipped) =
        scheduler.observe_missing_sequences_with_policy(&[30], 1_000.0, retry_budget_policy);
    assert!(retry_skipped.is_none());
    assert_eq!(retry_initial.expect("retry initial").sequences, vec![30]);

    let first_retry = scheduler.poll(1_010.0);
    let first_retry_batch = first_retry.retry_batch.as_ref().expect("poll retry");
    assert_eq!(first_retry_batch.sequences, vec![30]);

    let mut deadline_policy = base_policy();
    deadline_policy.deadline_at_ms = Some(1_014.0);
    deadline_policy.estimated_recovery_arrival_ms = Some(1_012.0);
    deadline_policy.frame_is_keyframe = Some(true);
    deadline_policy.frame_importance = "anchor";
    deadline_policy.priority = 2;
    let _ = scheduler.observe_missing_sequences_with_policy(&[10], 1_011.0, deadline_policy);

    let mut max_age_policy = deadline_policy;
    max_age_policy.deadline_at_ms = Some(1_100.0);
    max_age_policy.max_age_ms = Some(2);
    max_age_policy.priority = 1;
    let _ = scheduler.observe_missing_sequences_with_policy(&[20], 1_011.0, max_age_policy);

    let polled = scheduler.poll(1_015.0);
    assert!(polled.retry_batch.is_none());
    assert_eq!(
        scheduler.pending_count(),
        0,
        "seq 30 exhausted, 10/20 deadline or maxAge"
    );
    assert!(polled.expired_batches.len() >= 2);

    let deadline_batch = polled
        .expired_batches
        .iter()
        .find(|batch| batch.reason == "deadline")
        .expect("deadline batch");
    assert_eq!(deadline_batch.sequences, vec![10]);
    assert_eq!(
        deadline_batch.frame_unrecoverable_reason,
        Some("deadlineExceeded")
    );

    let max_age_batch = polled
        .expired_batches
        .iter()
        .find(|batch| batch.reason == "maxAge")
        .expect("max age batch");
    assert_eq!(max_age_batch.sequences, vec![20]);
    assert_eq!(
        max_age_batch.frame_unrecoverable_reason,
        Some("maxAgeExceeded")
    );

    if let Some(retry_budget_batch) = polled
        .expired_batches
        .iter()
        .find(|batch| batch.reason == "singleShotPollComplete")
    {
        assert_eq!(retry_budget_batch.sequences, vec![30]);
        assert_eq!(
            retry_budget_batch.frame_unrecoverable_reason,
            Some("singleShotPollFinalized")
        );
    }
}

#[test]
fn flush_reobserve_and_resolve_interleaving_keeps_state_consistent() {
    let mut scheduler = NackScheduler::new(NackSchedulerConfig {
        max_age_ms: 200,
        frame_deadline_ms: 500,
        burst_count: 1,
        retry_interval_ms: 10,
        max_retry_count: 3,
    });

    let mut disposable_policy = base_policy();
    disposable_policy.deadline_at_ms = Some(1_100.0);
    let (disposable_batch, disposable_skipped) =
        scheduler.observe_missing_sequences_with_policy(&[30, 31], 1_000.0, disposable_policy);
    assert!(disposable_skipped.is_none());
    assert_eq!(
        disposable_batch.expect("disposable batch").sequences,
        vec![30, 31]
    );

    let mut anchor_policy = base_policy();
    anchor_policy.deadline_at_ms = Some(1_100.0);
    anchor_policy.frame_is_keyframe = Some(true);
    anchor_policy.frame_importance = "anchor";
    anchor_policy.priority = 3;
    let (anchor_batch, anchor_skipped) =
        scheduler.observe_missing_sequences_with_policy(&[40], 1_000.0, anchor_policy);
    assert!(anchor_skipped.is_none());
    assert_eq!(anchor_batch.expect("anchor batch").sequences, vec![40]);
    assert_eq!(scheduler.pending_count(), 3);

    let flushed = scheduler
        .flush_non_keyframe_pending("awaitingRecoveryAnchor")
        .expect("flushed");
    assert_eq!(flushed.sequences, vec![30, 31]);
    assert_eq!(scheduler.pending_count(), 1);

    let mut supply_policy = base_policy();
    supply_policy.deadline_at_ms = Some(1_100.0);
    supply_policy.frame_importance = "supply";
    supply_policy.priority = 2;
    let (reobserve_batch, reobserve_skipped) =
        scheduler.observe_missing_sequences_with_policy(&[31], 1_005.0, supply_policy);
    assert!(reobserve_skipped.is_none());
    assert_eq!(
        reobserve_batch.expect("reobserve batch").sequences,
        vec![31]
    );
    assert_eq!(scheduler.pending_count(), 2);

    let resolved = scheduler
        .resolve_sequence(40, 1_006.0)
        .expect("resolved anchor");
    assert_eq!(resolved.sequence, 40);
    assert_eq!(scheduler.pending_count(), 1);

    let polled = scheduler.poll(1_100.0);
    assert!(polled.retry_batch.is_none());
    let deadline_batch = polled
        .expired_batches
        .iter()
        .find(|batch| batch.reason == "deadline")
        .expect("deadline expiry");
    assert!(deadline_batch.sequences.contains(&31));
    assert_eq!(scheduler.pending_count(), 0);
}

#[test]
fn prune_rtp_window_pending_not_missing_removes_only_stale_rtp_window_entries() {
    let mut scheduler = NackScheduler::new(NackSchedulerConfig {
        max_age_ms: 200,
        frame_deadline_ms: 300,
        burst_count: 3,
        retry_interval_ms: 40,
        max_retry_count: 3,
    });
    let mut policy = base_policy();
    policy.source = "rtpWindow";
    let _ = scheduler.observe_missing_sequences_with_policy(&[10, 11, 12], 1_000.0, policy);
    let stale = scheduler.prune_rtp_window_pending_not_missing(&[10, 12]);
    assert_eq!(stale, vec![11]);
    assert_eq!(scheduler.pending_count(), 2);
}

#[test]
fn prune_pending_in_range_supports_wrapping_range() {
    let mut scheduler = NackScheduler::new(NackSchedulerConfig {
        max_age_ms: 200,
        frame_deadline_ms: 300,
        burst_count: 8,
        retry_interval_ms: 40,
        max_retry_count: 3,
    });
    let mut policy = base_policy();
    policy.source = "rtpWindow";
    let _ =
        scheduler.observe_missing_sequences_with_policy(&[65534, 65535, 0, 1, 5], 1_000.0, policy);

    let removed = scheduler.prune_pending_in_range(65535, 2);
    assert_eq!(removed, vec![0, 1, 65535]);
    assert_eq!(scheduler.pending_count(), 1);
}

#[test]
fn anchor_and_supply_bypass_low_value_skip_logic() {
    let mut scheduler = NackScheduler::new(NackSchedulerConfig {
        max_age_ms: 500,
        frame_deadline_ms: 2_000,
        burst_count: 2,
        retry_interval_ms: 40,
        max_retry_count: 3,
    });

    // Test anchor bypass
    let mut anchor_policy = base_policy();
    anchor_policy.deadline_at_ms = Some(2_000.0);
    anchor_policy.nack_disposition = PacketRecoveryDisposition::SkippedLowValue;
    anchor_policy.frame_unrecoverable_reason = Some("cloudHighRttLowValueAdmission");
    anchor_policy.frame_is_keyframe = Some(true);
    anchor_policy.frame_importance = "anchor";
    anchor_policy.priority = 3;

    let (anchor_batch, anchor_skipped) =
        scheduler.observe_missing_sequences_with_policy(&[10, 11], 1_000.0, anchor_policy);
    assert!(anchor_skipped.is_none());
    let anchor_batch = anchor_batch.expect("anchor should bypass low-value skip");
    assert_eq!(anchor_batch.sequences, vec![10, 11]);
    assert_eq!(
        anchor_batch.nack_disposition,
        PacketRecoveryDisposition::Attempted
    );
    assert_eq!(anchor_batch.frame_unrecoverable_reason, None);
    assert_eq!(scheduler.pending_count(), 2);

    // Test supply bypass
    let mut supply_policy = base_policy();
    supply_policy.deadline_at_ms = Some(2_000.0);
    supply_policy.nack_disposition = PacketRecoveryDisposition::SkippedLowValue;
    supply_policy.frame_unrecoverable_reason = Some("cloudHighRttLowValueAdmission");
    supply_policy.frame_is_keyframe = Some(false);
    supply_policy.frame_importance = "supply";
    supply_policy.priority = 2;

    let (supply_batch, supply_skipped) =
        scheduler.observe_missing_sequences_with_policy(&[20, 21], 1_000.0, supply_policy);
    assert!(supply_skipped.is_none());
    let supply_batch = supply_batch.expect("supply should bypass low-value skip");
    assert_eq!(supply_batch.sequences, vec![20, 21]);
    assert_eq!(
        supply_batch.nack_disposition,
        PacketRecoveryDisposition::Attempted
    );
    assert_eq!(supply_batch.frame_unrecoverable_reason, None);
    assert_eq!(scheduler.pending_count(), 4);

    // Test disposable does NOT bypass
    let mut disposable_policy = base_policy();
    disposable_policy.deadline_at_ms = Some(2_000.0);
    disposable_policy.nack_disposition = PacketRecoveryDisposition::SkippedLowValue;
    disposable_policy.frame_unrecoverable_reason = Some("cloudHighRttLowValueAdmission");
    disposable_policy.frame_is_keyframe = Some(false);
    disposable_policy.frame_importance = "disposable";
    disposable_policy.priority = 1;

    let (disposable_batch, disposable_skipped) =
        scheduler.observe_missing_sequences_with_policy(&[30, 31], 1_000.0, disposable_policy);
    assert!(disposable_batch.is_none());
    let disposable_skipped = disposable_skipped.expect("disposable should be skipped");
    assert_eq!(disposable_skipped.sequences, vec![30, 31]);
    assert_eq!(
        disposable_skipped.nack_disposition,
        PacketRecoveryDisposition::SkippedLowValue
    );
    assert_eq!(scheduler.pending_count(), 4);
}

#[test]
fn pending_merge_with_unified_labels_respects_priority() {
    let mut scheduler = NackScheduler::new(NackSchedulerConfig {
        max_age_ms: 500,
        frame_deadline_ms: 2_000,
        burst_count: 1,
        retry_interval_ms: 40,
        max_retry_count: 3,
    });

    // First observe as disposable
    let mut disposable_policy = base_policy();
    disposable_policy.source = "rtpWindow";
    disposable_policy.deadline_at_ms = Some(2_000.0);
    disposable_policy.frame_importance = "disposable";
    disposable_policy.priority = 1;
    let (first_batch, first_skipped) =
        scheduler.observe_missing_sequences_with_policy(&[50], 1_000.0, disposable_policy);
    assert!(first_skipped.is_none());
    assert_eq!(first_batch.expect("first batch").sequences, vec![50]);
    assert_eq!(scheduler.pending_count(), 1);

    // Re-observe same sequence as supply (higher priority)
    let mut supply_policy = base_policy();
    supply_policy.source = "rtpGap";
    supply_policy.deadline_at_ms = Some(1_800.0);
    supply_policy.frame_importance = "supply";
    supply_policy.priority = 2;
    supply_policy.retry_interval_ms = Some(20);
    let (second_batch, second_skipped) =
        scheduler.observe_missing_sequences_with_policy(&[50], 1_001.0, supply_policy);
    assert!(second_batch.is_none());
    assert!(second_skipped.is_none());
    assert_eq!(scheduler.pending_count(), 1);

    // Re-observe same sequence as anchor (highest priority)
    let mut anchor_policy = base_policy();
    anchor_policy.source = "sampleLoss";
    anchor_policy.deadline_at_ms = Some(1_500.0);
    anchor_policy.frame_importance = "anchor";
    anchor_policy.frame_is_keyframe = Some(true);
    anchor_policy.priority = 3;
    anchor_policy.retry_interval_ms = Some(10);
    anchor_policy.max_retry_count_override = Some(1);
    let (third_batch, third_skipped) =
        scheduler.observe_missing_sequences_with_policy(&[50], 1_002.0, anchor_policy);
    assert!(third_batch.is_none());
    assert!(third_skipped.is_none());
    assert_eq!(scheduler.pending_count(), 1);

    // Poll should use the most aggressive (anchor) policy
    let polled = scheduler.poll(1_012.0);
    let retry_batch = polled.retry_batch.as_ref().expect("poll retry");
    assert_eq!(retry_batch.sequences, vec![50]);
    assert_eq!(retry_batch.frame_importance, "anchor");
    assert!(polled.expired_batches.is_empty());
}

#[test]
fn single_shot_pending_cleared_when_deadline_expires() {
    let mut scheduler = NackScheduler::new(NackSchedulerConfig {
        max_age_ms: 500,
        frame_deadline_ms: 2_000,
        burst_count: 1,
        retry_interval_ms: 10,
        max_retry_count: 3,
    });

    let mut anchor_policy = base_policy();
    anchor_policy.deadline_at_ms = Some(2_000.0);
    anchor_policy.frame_is_keyframe = Some(true);
    anchor_policy.frame_importance = "anchor";
    anchor_policy.priority = 3;
    let (anchor_batch, _) =
        scheduler.observe_missing_sequences_with_policy(&[10], 1_000.0, anchor_policy);
    assert!(anchor_batch.is_some());

    let mut supply_policy = base_policy();
    supply_policy.deadline_at_ms = Some(2_000.0);
    supply_policy.frame_importance = "supply";
    supply_policy.priority = 2;
    let (supply_batch, _) =
        scheduler.observe_missing_sequences_with_policy(&[20], 1_000.0, supply_policy);
    assert!(supply_batch.is_some());

    let mut disposable_policy = base_policy();
    disposable_policy.deadline_at_ms = Some(2_000.0);
    disposable_policy.frame_importance = "disposable";
    disposable_policy.priority = 1;
    let (disposable_batch, _) =
        scheduler.observe_missing_sequences_with_policy(&[30], 1_000.0, disposable_policy);
    assert!(disposable_batch.is_some());

    assert_eq!(scheduler.pending_count(), 3);

    let poll1 = scheduler.poll(1_010.0);
    assert!(poll1.retry_batch.is_none());
    assert!(poll1.expired_batches.is_empty());
    assert_eq!(scheduler.pending_count(), 3);

    let poll2 = scheduler.poll(2_000.0);
    assert!(poll2.retry_batch.is_none());
    assert_eq!(scheduler.pending_count(), 0);
    for seq in [10u16, 20, 30] {
        let batch = poll2
            .expired_batches
            .iter()
            .find(|b| b.sequences.contains(&seq))
            .unwrap_or_else(|| panic!("missing deadline expiry for {seq}"));
        assert_eq!(batch.reason, "deadline");
    }
}
