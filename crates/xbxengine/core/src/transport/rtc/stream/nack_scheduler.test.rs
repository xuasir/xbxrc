use super::{NackObservePolicy, NackScheduler, NackSchedulerConfig, PacketRecoveryDisposition};
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
        frame_importance: "delta",
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
    }
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
fn cloud_high_rtt_low_value_admission_keeps_reference_packets_repairable() {
    let mut scheduler = NackScheduler::new(NackSchedulerConfig {
        max_age_ms: 200,
        frame_deadline_ms: 120,
        burst_count: 2,
        retry_interval_ms: 40,
        max_retry_count: 3,
    });
    let mut policy = base_policy();
    policy.frame_importance = "reference";
    policy.priority = 2;
    policy.nack_disposition = PacketRecoveryDisposition::SkippedLowValue;
    policy.frame_unrecoverable_reason = Some("cloudHighRttLowValueAdmission");

    let (batch, skipped) =
        scheduler.observe_missing_sequences_with_policy(&[10, 11], 1_000.0, policy);
    assert!(skipped.is_none());
    let batch = batch.expect("reference batch should be attempted");
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
fn low_value_admission_does_not_skip_keyframe_recovery() {
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
    policy.frame_importance = "keyframe";

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
fn retry_budget_exhausted_is_finalized_and_dequeued() {
    let mut scheduler = NackScheduler::new(NackSchedulerConfig {
        max_age_ms: 200,
        frame_deadline_ms: 500,
        burst_count: 1,
        retry_interval_ms: 10,
        max_retry_count: 3,
    });
    let mut policy = base_policy();
    policy.frame_is_keyframe = Some(true);
    policy.frame_importance = "keyframe";
    policy.priority = 3;
    let expected_budget_context = policy.budget_context;

    let (initial_batch, skipped) =
        scheduler.observe_missing_sequences_with_policy(&[20], 1_000.0, policy);
    assert!(skipped.is_none());
    let initial_batch = initial_batch.expect("initial");
    assert_eq!(initial_batch.sequences, vec![20]);
    assert_eq!(initial_batch.budget_context, expected_budget_context);
    assert_eq!(scheduler.pending_count(), 1);

    let first_retry = scheduler.poll(1_010.0);
    let retry_batch = first_retry.retry_batch.expect("retry");
    assert_eq!(retry_batch.sequences, vec![20]);
    assert_eq!(retry_batch.budget_context, expected_budget_context);
    assert!(first_retry.expired_batches.is_empty());
    assert_eq!(scheduler.pending_count(), 1);

    let exhausted = scheduler.poll(1_020.0);
    assert!(exhausted.retry_batch.is_none());
    assert_eq!(scheduler.pending_count(), 0);
    assert_eq!(exhausted.expired_batches.len(), 1);
    assert_eq!(exhausted.expired_batches[0].reason, "retryBudget");
    assert_eq!(exhausted.expired_batches[0].sequences, vec![20]);
    assert_eq!(
        exhausted.expired_batches[0].budget_context,
        expected_budget_context
    );
}

#[test]
fn reference_packet_with_supply_priority_gets_single_retry_budget() {
    let mut scheduler = NackScheduler::new(NackSchedulerConfig {
        max_age_ms: 200,
        frame_deadline_ms: 500,
        burst_count: 1,
        retry_interval_ms: 10,
        max_retry_count: 3,
    });
    let mut policy = base_policy();
    policy.frame_is_keyframe = Some(false);
    policy.frame_importance = "reference";
    policy.priority = 2;

    let (initial_batch, skipped) =
        scheduler.observe_missing_sequences_with_policy(&[60], 1_000.0, policy);
    assert!(skipped.is_none());
    assert_eq!(initial_batch.expect("initial").sequences, vec![60]);
    assert_eq!(scheduler.pending_count(), 1);

    let first_retry = scheduler.poll(1_010.0);
    assert_eq!(first_retry.retry_batch.expect("retry").sequences, vec![60]);
    assert!(first_retry.expired_batches.is_empty());

    let exhausted = scheduler.poll(1_020.0);
    assert!(exhausted.retry_batch.is_none());
    assert_eq!(exhausted.expired_batches.len(), 1);
    assert_eq!(exhausted.expired_batches[0].reason, "retryBudget");
}

#[test]
fn chain_broken_flush_removes_non_keyframe_pending() {
    let mut scheduler = NackScheduler::new(NackSchedulerConfig {
        max_age_ms: 200,
        frame_deadline_ms: 500,
        burst_count: 4,
        retry_interval_ms: 10,
        max_retry_count: 3,
    });

    let mut delta_policy = base_policy();
    delta_policy.frame_is_keyframe = Some(false);
    delta_policy.frame_importance = "delta";
    let expected_delta_budget_context = delta_policy.budget_context;
    let _ = scheduler.observe_missing_sequences_with_policy(&[30, 31], 1_000.0, delta_policy);

    let mut keyframe_policy = base_policy();
    keyframe_policy.frame_is_keyframe = Some(true);
    keyframe_policy.frame_importance = "keyframe";
    keyframe_policy.priority = 3;
    let _ = scheduler.observe_missing_sequences_with_policy(&[40], 1_000.0, keyframe_policy);

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
    assert_eq!(flushed.budget_context, expected_delta_budget_context);
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
fn delta_packet_has_no_retry_budget_and_finalizes_on_first_poll() {
    let mut scheduler = NackScheduler::new(NackSchedulerConfig {
        max_age_ms: 200,
        frame_deadline_ms: 500,
        burst_count: 1,
        retry_interval_ms: 10,
        max_retry_count: 3,
    });
    let mut policy = base_policy();
    policy.frame_is_keyframe = Some(false);
    policy.frame_importance = "delta";
    policy.priority = 1;

    let (initial_batch, skipped) =
        scheduler.observe_missing_sequences_with_policy(&[50], 1_000.0, policy);
    assert!(skipped.is_none());
    assert_eq!(initial_batch.expect("initial").sequences, vec![50]);
    assert_eq!(scheduler.pending_count(), 1);

    let polled = scheduler.poll(1_010.0);
    assert!(polled.retry_batch.is_none());
    assert_eq!(scheduler.pending_count(), 0);
    assert_eq!(polled.expired_batches.len(), 1);
    assert_eq!(polled.expired_batches[0].reason, "retryBudget");
    assert_eq!(
        polled.expired_batches[0].frame_unrecoverable_reason,
        Some("retryBudgetExhausted")
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
    policy.frame_unrecoverable_reason = Some("awaitingRecoveryKeyframe");
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
        Some("awaitingRecoveryKeyframe")
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

    let mut reference_policy = base_policy();
    reference_policy.frame_importance = "reference";
    reference_policy.priority = 2;
    reference_policy.frame_rtp_timestamp = Some(90_100);
    reference_policy.deadline_at_ms = Some(1_200.0);
    let (reference_batch, reference_skipped) =
        scheduler.observe_missing_sequences_with_policy(&[10, 11], 1_000.0, reference_policy);
    assert!(reference_skipped.is_none());
    assert_eq!(
        reference_batch.expect("reference initial").sequences,
        vec![10, 11]
    );

    let mut keyframe_policy = base_policy();
    keyframe_policy.frame_is_keyframe = Some(true);
    keyframe_policy.frame_importance = "keyframe";
    keyframe_policy.priority = 3;
    keyframe_policy.frame_rtp_timestamp = Some(90_200);
    keyframe_policy.deadline_at_ms = Some(1_200.0);
    let (keyframe_batch, keyframe_skipped) =
        scheduler.observe_missing_sequences_with_policy(&[20, 21], 1_001.0, keyframe_policy);
    assert!(keyframe_skipped.is_none());
    assert_eq!(
        keyframe_batch.expect("keyframe initial").sequences,
        vec![20, 21]
    );

    let polled = scheduler.poll(1_011.0);
    let retry_batch = polled.retry_batch.expect("retry batch");
    assert_eq!(retry_batch.sequences, vec![20, 21]);
    assert_eq!(retry_batch.frame_importance, "keyframe");
    assert_eq!(retry_batch.frame_rtp_timestamp, Some(90_200));
    assert!(polled.expired_batches.is_empty());
    assert_eq!(scheduler.pending_count(), 4);
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
    retry_policy.frame_importance = "keyframe";
    retry_policy.priority = 3;
    retry_policy.frame_rtp_timestamp = Some(90_500);
    let _ = scheduler.observe_missing_sequences_with_policy(&[50], 1_000.0, retry_policy);

    let polled = scheduler.poll(1_010.0);
    let retry_batch = polled.retry_batch.expect("retry batch");
    assert_eq!(retry_batch.sequences, vec![50]);
    assert_eq!(retry_batch.frame_rtp_timestamp, Some(90_500));

    assert_eq!(polled.expired_batches.len(), 2);
    assert_eq!(polled.expired_batches[0].reason, "deadline");
    assert_eq!(polled.expired_batches[0].sequences, vec![30]);
    assert_eq!(
        polled.expired_batches[0].frame_unrecoverable_reason,
        Some("deadlineExceeded")
    );
    assert_eq!(polled.expired_batches[1].reason, "maxAge");
    assert_eq!(polled.expired_batches[1].sequences, vec![40]);
    assert_eq!(
        polled.expired_batches[1].frame_unrecoverable_reason,
        Some("maxAgeExceeded")
    );
    assert_eq!(scheduler.pending_count(), 1);
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
fn flush_non_keyframe_pending_keeps_keyframe_retryable() {
    let mut scheduler = NackScheduler::new(NackSchedulerConfig {
        max_age_ms: 300,
        frame_deadline_ms: 500,
        burst_count: 1,
        retry_interval_ms: 10,
        max_retry_count: 3,
    });

    let mut reference_policy = base_policy();
    reference_policy.frame_importance = "reference";
    reference_policy.priority = 2;
    reference_policy.deadline_at_ms = Some(1_200.0);
    let _ = scheduler.observe_missing_sequences_with_policy(&[90], 1_000.0, reference_policy);

    let mut keyframe_policy = base_policy();
    keyframe_policy.frame_is_keyframe = Some(true);
    keyframe_policy.frame_importance = "keyframe";
    keyframe_policy.priority = 3;
    keyframe_policy.deadline_at_ms = Some(1_200.0);
    let _ = scheduler.observe_missing_sequences_with_policy(&[91], 1_000.0, keyframe_policy);

    let flushed = scheduler
        .flush_non_keyframe_pending("flushedAfterChainBrokenAdmission")
        .expect("flushed");
    assert_eq!(flushed.sequences, vec![90]);
    assert_eq!(flushed.reason, "chainBroken");
    assert_eq!(scheduler.pending_count(), 1);

    let polled = scheduler.poll(1_010.0);
    let retry_batch = polled.retry_batch.expect("retry batch");
    assert_eq!(retry_batch.sequences, vec![91]);
    assert_eq!(retry_batch.frame_importance, "keyframe");
    assert!(polled.expired_batches.is_empty());
    assert_eq!(scheduler.pending_count(), 1);
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
    policy.frame_importance = "keyframe";
    policy.priority = 3;

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
    let retry_batch = polled.retry_batch.expect("retry batch");
    assert_eq!(retry_batch.sequences, vec![12, 13]);
    assert_eq!(retry_batch.retry_count, 1);
    assert!(polled.expired_batches.is_empty());
    assert_eq!(scheduler.pending_count(), 6);
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
    let mut scheduler = NackScheduler::new(NackSchedulerConfig {
        max_age_ms: 200,
        frame_deadline_ms: 500,
        burst_count: 1,
        retry_interval_ms: 10,
        max_retry_count: 3,
    });

    let mut retry_budget_policy = base_policy();
    retry_budget_policy.deadline_at_ms = Some(1_100.0);
    retry_budget_policy.frame_is_keyframe = Some(true);
    retry_budget_policy.frame_importance = "keyframe";
    retry_budget_policy.priority = 3;
    let (retry_initial, retry_skipped) =
        scheduler.observe_missing_sequences_with_policy(&[30], 1_000.0, retry_budget_policy);
    assert!(retry_skipped.is_none());
    assert_eq!(retry_initial.expect("retry initial").sequences, vec![30]);

    let first_retry = scheduler.poll(1_010.0);
    assert_eq!(
        first_retry.retry_batch.expect("first retry").sequences,
        vec![30]
    );
    assert!(first_retry.expired_batches.is_empty());
    assert_eq!(scheduler.pending_count(), 1);

    let mut deadline_policy = base_policy();
    deadline_policy.deadline_at_ms = Some(1_014.0);
    deadline_policy.estimated_recovery_arrival_ms = Some(1_012.0);
    deadline_policy.frame_is_keyframe = Some(true);
    deadline_policy.frame_importance = "keyframe";
    deadline_policy.priority = 2;
    let _ = scheduler.observe_missing_sequences_with_policy(&[10], 1_011.0, deadline_policy);

    let mut max_age_policy = deadline_policy;
    max_age_policy.deadline_at_ms = Some(1_100.0);
    max_age_policy.max_age_ms = Some(2);
    max_age_policy.priority = 1;
    let _ = scheduler.observe_missing_sequences_with_policy(&[20], 1_011.0, max_age_policy);

    let polled = scheduler.poll(1_015.0);
    assert!(polled.retry_batch.is_none());
    assert_eq!(scheduler.pending_count(), 0);
    assert_eq!(polled.expired_batches.len(), 3);

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

    let retry_budget_batch = polled
        .expired_batches
        .iter()
        .find(|batch| batch.reason == "retryBudget")
        .expect("retry budget batch");
    assert_eq!(retry_budget_batch.sequences, vec![30]);
    assert_eq!(
        retry_budget_batch.frame_unrecoverable_reason,
        Some("retryBudgetExhausted")
    );
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

    let mut delta_policy = base_policy();
    delta_policy.deadline_at_ms = Some(1_100.0);
    let (delta_batch, delta_skipped) =
        scheduler.observe_missing_sequences_with_policy(&[30, 31], 1_000.0, delta_policy);
    assert!(delta_skipped.is_none());
    assert_eq!(delta_batch.expect("delta batch").sequences, vec![30, 31]);

    let mut keyframe_policy = base_policy();
    keyframe_policy.deadline_at_ms = Some(1_100.0);
    keyframe_policy.frame_is_keyframe = Some(true);
    keyframe_policy.frame_importance = "keyframe";
    keyframe_policy.priority = 3;
    let (keyframe_batch, keyframe_skipped) =
        scheduler.observe_missing_sequences_with_policy(&[40], 1_000.0, keyframe_policy);
    assert!(keyframe_skipped.is_none());
    assert_eq!(keyframe_batch.expect("keyframe batch").sequences, vec![40]);
    assert_eq!(scheduler.pending_count(), 3);

    let flushed = scheduler
        .flush_non_keyframe_pending("awaitingRecoveryKeyframe")
        .expect("flushed");
    assert_eq!(flushed.sequences, vec![30, 31]);
    assert_eq!(scheduler.pending_count(), 1);

    let mut reference_policy = base_policy();
    reference_policy.deadline_at_ms = Some(1_100.0);
    reference_policy.frame_importance = "reference";
    reference_policy.priority = 2;
    let (reobserve_batch, reobserve_skipped) =
        scheduler.observe_missing_sequences_with_policy(&[31], 1_005.0, reference_policy);
    assert!(reobserve_skipped.is_none());
    assert_eq!(
        reobserve_batch.expect("reobserve batch").sequences,
        vec![31]
    );
    assert_eq!(scheduler.pending_count(), 2);

    let resolved = scheduler
        .resolve_sequence(40, 1_006.0)
        .expect("resolved keyframe");
    assert_eq!(resolved.sequence, 40);
    assert_eq!(scheduler.pending_count(), 1);

    let polled = scheduler.poll(1_015.0);
    assert!(polled.expired_batches.is_empty());
    assert_eq!(polled.retry_batch.expect("retry batch").sequences, vec![31]);
    assert_eq!(scheduler.pending_count(), 1);
}
