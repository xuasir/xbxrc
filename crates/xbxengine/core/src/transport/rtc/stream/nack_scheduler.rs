use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PacketRecoveryDisposition {
    Attempted,
    SkippedTooLate,
    SkippedLowValue,
    SkippedChainBroken,
}

impl PacketRecoveryDisposition {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Attempted => "attempted",
            Self::SkippedTooLate => "skippedTooLate",
            Self::SkippedLowValue => "skippedLowValue",
            Self::SkippedChainBroken => "skippedChainBroken",
        }
    }
}

#[derive(Clone, Debug)]
pub struct NackSchedulerConfig {
    pub max_age_ms: u64,
    pub frame_deadline_ms: u64,
    pub burst_count: u16,
    pub retry_interval_ms: u64,
    pub max_retry_count: u8,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NackBatch {
    pub sequences: Vec<u16>,
    pub retry_count: u8,
    pub source: &'static str,
    pub frame_rtp_timestamp: Option<u32>,
    pub frame_is_keyframe: Option<bool>,
    pub frame_importance: &'static str,
    pub deadline_at_ms: Option<f64>,
    pub estimated_recovery_arrival_ms: Option<f64>,
    pub frame_playout_deadline_at_ms: Option<f64>,
    pub nack_disposition: PacketRecoveryDisposition,
    pub frame_unrecoverable_reason: Option<&'static str>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedNack {
    pub sequence: u16,
    pub recovery_time_ms: f64,
    pub retry_count: u8,
    pub was_late: bool,
    pub source: &'static str,
    pub frame_rtp_timestamp: Option<u32>,
    pub frame_is_keyframe: Option<bool>,
    pub frame_importance: &'static str,
    pub deadline_at_ms: Option<f64>,
    pub estimated_recovery_arrival_ms: Option<f64>,
    pub frame_playout_deadline_at_ms: Option<f64>,
    pub nack_disposition: PacketRecoveryDisposition,
    pub frame_unrecoverable_reason: Option<&'static str>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExpiredNackBatch {
    pub sequences: Vec<u16>,
    pub reason: String,
    pub source: &'static str,
    pub frame_rtp_timestamp: Option<u32>,
    pub frame_is_keyframe: Option<bool>,
    pub frame_importance: &'static str,
    pub deadline_at_ms: Option<f64>,
    pub estimated_recovery_arrival_ms: Option<f64>,
    pub frame_playout_deadline_at_ms: Option<f64>,
    pub nack_disposition: PacketRecoveryDisposition,
    pub frame_unrecoverable_reason: Option<&'static str>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SkippedNackBatch {
    pub sequences: Vec<u16>,
    pub source: &'static str,
    pub frame_rtp_timestamp: Option<u32>,
    pub frame_is_keyframe: Option<bool>,
    pub frame_importance: &'static str,
    pub deadline_at_ms: Option<f64>,
    pub estimated_recovery_arrival_ms: Option<f64>,
    pub frame_playout_deadline_at_ms: Option<f64>,
    pub nack_disposition: PacketRecoveryDisposition,
    pub frame_unrecoverable_reason: Option<&'static str>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct NackPollResult {
    pub retry_batch: Option<NackBatch>,
    pub expired_batches: Vec<ExpiredNackBatch>,
}

#[derive(Clone, Debug)]
struct PendingNack {
    first_seen_at_ms: f64,
    last_sent_at_ms: f64,
    deadline_at_ms: f64,
    retry_count: u8,
    max_retry_count: u8,
    max_age_ms: u64,
    retry_interval_ms: u64,
    source: &'static str,
    frame_rtp_timestamp: Option<u32>,
    frame_is_keyframe: Option<bool>,
    frame_importance: &'static str,
    priority: u8,
    estimated_recovery_arrival_ms: Option<f64>,
    frame_playout_deadline_at_ms: Option<f64>,
    frame_unrecoverable_reason: Option<&'static str>,
}

#[derive(Clone, Copy, Debug)]
pub struct NackObservePolicy {
    pub source: &'static str,
    pub deadline_at_ms: Option<f64>,
    pub max_age_ms: Option<u64>,
    pub retry_interval_ms: Option<u64>,
    pub burst_count: Option<u16>,
    pub max_tracked_sequences: Option<u16>,
    pub frame_rtp_timestamp: Option<u32>,
    pub frame_is_keyframe: Option<bool>,
    pub frame_importance: &'static str,
    pub priority: u8,
    pub estimated_recovery_arrival_ms: Option<f64>,
    pub frame_playout_deadline_at_ms: Option<f64>,
    pub nack_disposition: PacketRecoveryDisposition,
    pub frame_unrecoverable_reason: Option<&'static str>,
}

pub struct NackScheduler {
    config: NackSchedulerConfig,
    pending: BTreeMap<u16, PendingNack>,
}

impl NackScheduler {
    pub fn new(config: NackSchedulerConfig) -> Self {
        Self {
            config,
            pending: BTreeMap::new(),
        }
    }

    pub fn observe_missing_sequences_with_policy(
        &mut self,
        sequences: &[u16],
        now_ms: f64,
        policy: NackObservePolicy,
    ) -> (Option<NackBatch>, Option<SkippedNackBatch>) {
        if sequences.is_empty() {
            return (None, None);
        }

        let burst_count = usize::from(policy.burst_count.unwrap_or(self.config.burst_count).max(1));
        let max_tracked_sequences = usize::from(
            policy
                .max_tracked_sequences
                .unwrap_or((burst_count.saturating_mul(2)).min(u16::MAX as usize) as u16)
                .max(1),
        );
        let retry_interval_ms = policy
            .retry_interval_ms
            .unwrap_or(self.config.retry_interval_ms);
        let max_age_ms = policy.max_age_ms.unwrap_or(self.config.max_age_ms);
        let deadline_at_ms = policy
            .deadline_at_ms
            .unwrap_or(now_ms + self.config.frame_deadline_ms as f64);
        let estimated_recovery_arrival_ms = policy.estimated_recovery_arrival_ms;
        let max_retry_count = if estimated_recovery_arrival_ms.is_some() {
            frame_importance_retry_budget(policy.frame_importance, self.config.max_retry_count)
        } else {
            self.config.max_retry_count
        };

        if matches!(
            policy.nack_disposition,
            PacketRecoveryDisposition::Attempted
        ) && estimated_recovery_arrival_ms.is_some_and(|arrival| arrival > deadline_at_ms)
        {
            let skipped = SkippedNackBatch {
                sequences: sequences
                    .iter()
                    .copied()
                    .take(max_tracked_sequences)
                    .collect(),
                source: policy.source,
                frame_rtp_timestamp: policy.frame_rtp_timestamp,
                frame_is_keyframe: policy.frame_is_keyframe,
                frame_importance: policy.frame_importance,
                deadline_at_ms: Some(deadline_at_ms),
                estimated_recovery_arrival_ms,
                frame_playout_deadline_at_ms: policy
                    .frame_playout_deadline_at_ms
                    .or(Some(deadline_at_ms)),
                nack_disposition: PacketRecoveryDisposition::SkippedTooLate,
                frame_unrecoverable_reason: Some("estimatedArrivalPastDeadline"),
            };
            return (None, Some(skipped));
        }

        if matches!(
            policy.nack_disposition,
            PacketRecoveryDisposition::SkippedTooLate
                | PacketRecoveryDisposition::SkippedLowValue
                | PacketRecoveryDisposition::SkippedChainBroken
        ) {
            let skipped = SkippedNackBatch {
                sequences: sequences
                    .iter()
                    .copied()
                    .take(max_tracked_sequences)
                    .collect(),
                source: policy.source,
                frame_rtp_timestamp: policy.frame_rtp_timestamp,
                frame_is_keyframe: policy.frame_is_keyframe,
                frame_importance: policy.frame_importance,
                deadline_at_ms: Some(deadline_at_ms),
                estimated_recovery_arrival_ms,
                frame_playout_deadline_at_ms: policy.frame_playout_deadline_at_ms,
                nack_disposition: policy.nack_disposition,
                frame_unrecoverable_reason: policy.frame_unrecoverable_reason,
            };
            return (None, Some(skipped));
        }

        let mut inserted = Vec::new();
        for (index, sequence) in sequences.iter().take(max_tracked_sequences).enumerate() {
            if self.pending.contains_key(sequence) {
                continue;
            }
            let last_sent_at_ms = if inserted.len() < burst_count && index < burst_count {
                now_ms
            } else {
                now_ms - retry_interval_ms as f64
            };
            self.pending.insert(
                *sequence,
                PendingNack {
                    first_seen_at_ms: now_ms,
                    last_sent_at_ms,
                    deadline_at_ms,
                    retry_count: 0,
                    max_retry_count,
                    max_age_ms,
                    retry_interval_ms,
                    source: policy.source,
                    frame_rtp_timestamp: policy.frame_rtp_timestamp,
                    frame_is_keyframe: policy.frame_is_keyframe,
                    frame_importance: policy.frame_importance,
                    priority: policy.priority,
                    estimated_recovery_arrival_ms,
                    frame_playout_deadline_at_ms: policy
                        .frame_playout_deadline_at_ms
                        .or(Some(deadline_at_ms)),
                    frame_unrecoverable_reason: policy.frame_unrecoverable_reason,
                },
            );
            inserted.push(*sequence);
        }

        let batch = if inserted.is_empty() {
            None
        } else {
            Some(NackBatch {
                sequences: inserted.into_iter().take(burst_count).collect(),
                retry_count: 0,
                source: policy.source,
                frame_rtp_timestamp: policy.frame_rtp_timestamp,
                frame_is_keyframe: policy.frame_is_keyframe,
                frame_importance: policy.frame_importance,
                deadline_at_ms: Some(deadline_at_ms),
                estimated_recovery_arrival_ms,
                frame_playout_deadline_at_ms: policy
                    .frame_playout_deadline_at_ms
                    .or(Some(deadline_at_ms)),
                nack_disposition: PacketRecoveryDisposition::Attempted,
                frame_unrecoverable_reason: policy.frame_unrecoverable_reason,
            })
        };
        (batch, None)
    }

    pub fn poll(&mut self, now_ms: f64) -> NackPollResult {
        let mut expired_deadline_sequences = Vec::new();
        let mut expired_max_age_sequences = Vec::new();
        let mut expired_deadline_meta = None;
        let mut expired_max_age_meta = None;
        self.pending.retain(|sequence, pending| {
            let age_ms = (now_ms - pending.first_seen_at_ms).max(0.0);
            if now_ms >= pending.deadline_at_ms {
                expired_deadline_sequences.push(*sequence);
                expired_deadline_meta.get_or_insert((
                    pending.source,
                    pending.frame_rtp_timestamp,
                    pending.frame_is_keyframe,
                    pending.frame_importance,
                    Some(pending.deadline_at_ms),
                    pending.estimated_recovery_arrival_ms,
                    pending.frame_playout_deadline_at_ms,
                    pending.frame_unrecoverable_reason,
                ));
                return false;
            }
            if age_ms >= pending.max_age_ms as f64 {
                expired_max_age_sequences.push(*sequence);
                expired_max_age_meta.get_or_insert((
                    pending.source,
                    pending.frame_rtp_timestamp,
                    pending.frame_is_keyframe,
                    pending.frame_importance,
                    Some(pending.deadline_at_ms),
                    pending.estimated_recovery_arrival_ms,
                    pending.frame_playout_deadline_at_ms,
                    pending.frame_unrecoverable_reason,
                ));
                return false;
            }
            true
        });

        let mut retry_candidates = Vec::new();
        for (sequence, pending) in &mut self.pending {
            let since_last_sent_ms = (now_ms - pending.last_sent_at_ms).max(0.0);
            if pending.retry_count >= pending.max_retry_count {
                continue;
            }
            if since_last_sent_ms < pending.retry_interval_ms as f64 {
                continue;
            }
            retry_candidates.push((
                *sequence,
                pending.priority,
                pending.first_seen_at_ms,
                pending.source,
                pending.frame_rtp_timestamp,
                pending.frame_is_keyframe,
                pending.frame_importance,
                pending.deadline_at_ms,
                pending.estimated_recovery_arrival_ms,
                pending.frame_playout_deadline_at_ms,
                pending.frame_unrecoverable_reason,
            ));
        }
        retry_candidates.sort_by(|left, right| {
            right.1.cmp(&left.1).then_with(|| {
                left.2
                    .partial_cmp(&right.2)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
        });
        let mut retry_sequences = Vec::new();
        let mut next_retry_count = 0u8;
        let mut retry_meta = None;
        let burst_count = usize::from(self.config.burst_count.max(1));
        for (
            sequence,
            _,
            _,
            source,
            frame_rtp_timestamp,
            frame_is_keyframe,
            frame_importance,
            deadline_at_ms,
            estimated_recovery_arrival_ms,
            frame_playout_deadline_at_ms,
            frame_unrecoverable_reason,
        ) in retry_candidates.into_iter().take(burst_count)
        {
            if let Some(pending) = self.pending.get_mut(&sequence) {
                pending.retry_count = pending.retry_count.saturating_add(1);
                pending.last_sent_at_ms = now_ms;
                next_retry_count = pending.retry_count;
                retry_sequences.push(sequence);
                retry_meta.get_or_insert((
                    source,
                    frame_rtp_timestamp,
                    frame_is_keyframe,
                    frame_importance,
                    Some(deadline_at_ms),
                    estimated_recovery_arrival_ms,
                    frame_playout_deadline_at_ms,
                    frame_unrecoverable_reason,
                ));
            }
        }

        NackPollResult {
            retry_batch: retry_sequences
                .is_empty()
                .then_some(())
                .and(None)
                .or_else(|| {
                    let (
                        source,
                        frame_rtp_timestamp,
                        frame_is_keyframe,
                        frame_importance,
                        deadline_at_ms,
                        estimated_recovery_arrival_ms,
                        frame_playout_deadline_at_ms,
                        frame_unrecoverable_reason,
                    ) = retry_meta.unwrap_or((
                        "rtpWindow",
                        None,
                        None,
                        "unknown",
                        None,
                        None,
                        None,
                        None,
                    ));
                    Some(NackBatch {
                        sequences: retry_sequences,
                        retry_count: next_retry_count,
                        source,
                        frame_rtp_timestamp,
                        frame_is_keyframe,
                        frame_importance,
                        deadline_at_ms,
                        estimated_recovery_arrival_ms,
                        frame_playout_deadline_at_ms: frame_playout_deadline_at_ms
                            .or(deadline_at_ms),
                        nack_disposition: PacketRecoveryDisposition::Attempted,
                        frame_unrecoverable_reason,
                    })
                }),
            expired_batches: vec![
                ExpiredNackBatch {
                    sequences: expired_deadline_sequences,
                    reason: "deadline".to_string(),
                    source: expired_deadline_meta
                        .map(|meta| meta.0)
                        .unwrap_or("rtpWindow"),
                    frame_rtp_timestamp: expired_deadline_meta.and_then(|meta| meta.1),
                    frame_is_keyframe: expired_deadline_meta.and_then(|meta| meta.2),
                    frame_importance: expired_deadline_meta
                        .map(|meta| meta.3)
                        .unwrap_or("unknown"),
                    deadline_at_ms: expired_deadline_meta.and_then(|meta| meta.4),
                    estimated_recovery_arrival_ms: expired_deadline_meta.and_then(|meta| meta.5),
                    frame_playout_deadline_at_ms: expired_deadline_meta
                        .and_then(|meta| meta.6)
                        .or(expired_deadline_meta.and_then(|meta| meta.4)),
                    nack_disposition: PacketRecoveryDisposition::SkippedTooLate,
                    frame_unrecoverable_reason: expired_deadline_meta
                        .and_then(|meta| meta.7)
                        .or(Some("deadlineExceeded")),
                },
                ExpiredNackBatch {
                    sequences: expired_max_age_sequences,
                    reason: "maxAge".to_string(),
                    source: expired_max_age_meta
                        .map(|meta| meta.0)
                        .unwrap_or("rtpWindow"),
                    frame_rtp_timestamp: expired_max_age_meta.and_then(|meta| meta.1),
                    frame_is_keyframe: expired_max_age_meta.and_then(|meta| meta.2),
                    frame_importance: expired_max_age_meta.map(|meta| meta.3).unwrap_or("unknown"),
                    deadline_at_ms: expired_max_age_meta.and_then(|meta| meta.4),
                    estimated_recovery_arrival_ms: expired_max_age_meta.and_then(|meta| meta.5),
                    frame_playout_deadline_at_ms: expired_max_age_meta
                        .and_then(|meta| meta.6)
                        .or(expired_max_age_meta.and_then(|meta| meta.4)),
                    nack_disposition: PacketRecoveryDisposition::SkippedTooLate,
                    frame_unrecoverable_reason: expired_max_age_meta
                        .and_then(|meta| meta.7)
                        .or(Some("maxAgeExceeded")),
                },
            ]
            .into_iter()
            .filter(|batch| !batch.sequences.is_empty())
            .collect(),
        }
    }

    pub fn resolve_sequence(&mut self, sequence: u16, now_ms: f64) -> Option<ResolvedNack> {
        self.pending.remove(&sequence).map(|pending| ResolvedNack {
            sequence,
            recovery_time_ms: (now_ms - pending.first_seen_at_ms).max(0.0),
            retry_count: pending.retry_count,
            was_late: now_ms >= pending.deadline_at_ms,
            source: pending.source,
            frame_rtp_timestamp: pending.frame_rtp_timestamp,
            frame_is_keyframe: pending.frame_is_keyframe,
            frame_importance: pending.frame_importance,
            deadline_at_ms: Some(pending.deadline_at_ms),
            estimated_recovery_arrival_ms: pending.estimated_recovery_arrival_ms,
            frame_playout_deadline_at_ms: pending
                .frame_playout_deadline_at_ms
                .or(Some(pending.deadline_at_ms)),
            nack_disposition: if now_ms >= pending.deadline_at_ms {
                PacketRecoveryDisposition::SkippedTooLate
            } else {
                PacketRecoveryDisposition::Attempted
            },
            frame_unrecoverable_reason: if now_ms >= pending.deadline_at_ms {
                Some("deadlineExceededBeforeRecovery")
            } else {
                pending.frame_unrecoverable_reason
            },
        })
    }

    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }
}

fn frame_importance_retry_budget(frame_importance: &str, default_max_retry_count: u8) -> u8 {
    match frame_importance {
        "keyframe" => default_max_retry_count,
        "reference" => default_max_retry_count.min(1),
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::{NackObservePolicy, NackScheduler, NackSchedulerConfig, PacketRecoveryDisposition};

    #[test]
    fn skipped_too_late_does_not_enter_pending() {
        let mut scheduler = NackScheduler::new(NackSchedulerConfig {
            max_age_ms: 200,
            frame_deadline_ms: 120,
            burst_count: 2,
            retry_interval_ms: 40,
            max_retry_count: 3,
        });
        let (batch, skipped) = scheduler.observe_missing_sequences_with_policy(
            &[10, 11],
            1_000.0,
            NackObservePolicy {
                source: "sampleLoss",
                deadline_at_ms: Some(1_050.0),
                max_age_ms: None,
                retry_interval_ms: Some(40),
                burst_count: None,
                max_tracked_sequences: None,
                frame_rtp_timestamp: Some(90_000),
                frame_is_keyframe: Some(false),
                frame_importance: "delta",
                priority: 1,
                estimated_recovery_arrival_ms: Some(1_120.0),
                frame_playout_deadline_at_ms: Some(1_050.0),
                nack_disposition: PacketRecoveryDisposition::SkippedTooLate,
                frame_unrecoverable_reason: Some("deadline"),
            },
        );
        assert!(batch.is_none());
        assert_eq!(scheduler.pending_count(), 0);
        assert_eq!(
            skipped.expect("skipped").nack_disposition,
            PacketRecoveryDisposition::SkippedTooLate
        );
    }
}
