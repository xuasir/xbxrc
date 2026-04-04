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
