use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct NackSchedulerConfig {
    pub max_age_ms: u64,
    pub frame_deadline_ms: u64,
    pub burst_count: u16,
    pub retry_interval_ms: u64,
    pub max_retry_count: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NackBatch {
    pub sequences: Vec<u16>,
    pub retry_count: u8,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedNack {
    pub sequence: u16,
    pub recovery_time_ms: f64,
    pub retry_count: u8,
    pub was_late: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpiredNackBatch {
    pub sequences: Vec<u16>,
    pub reason: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
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

    pub fn observe_gap(
        &mut self,
        expected_sequence: u16,
        received_sequence: u16,
        now_ms: f64,
        deadline_at_ms: Option<f64>,
    ) -> Option<NackBatch> {
        if received_sequence <= expected_sequence {
            let wrapping_diff = received_sequence.wrapping_sub(expected_sequence);
            if wrapping_diff >= (1 << 15) || wrapping_diff == 0 {
                return None;
            }
        }
        let sequences = sequence_range(expected_sequence, received_sequence);
        if sequences.is_empty() {
            return None;
        }

        let burst_count = usize::from(self.config.burst_count.max(1));
        for (index, sequence) in sequences.iter().enumerate() {
            let last_sent_at_ms = if index < burst_count {
                now_ms
            } else {
                now_ms - self.config.retry_interval_ms as f64
            };
            self.pending.entry(*sequence).or_insert(PendingNack {
                first_seen_at_ms: now_ms,
                last_sent_at_ms,
                deadline_at_ms: deadline_at_ms
                    .unwrap_or(now_ms + self.config.frame_deadline_ms as f64),
                retry_count: 0,
            });
        }

        Some(NackBatch {
            sequences: sequences.into_iter().take(burst_count).collect(),
            retry_count: 0,
        })
    }

    pub fn observe_missing_sequences(
        &mut self,
        sequences: &[u16],
        now_ms: f64,
        deadline_at_ms: Option<f64>,
    ) -> Option<NackBatch> {
        if sequences.is_empty() {
            return None;
        }

        let mut inserted = Vec::new();
        let burst_count = usize::from(self.config.burst_count.max(1));
        for (index, sequence) in sequences.iter().enumerate() {
            if self.pending.contains_key(sequence) {
                continue;
            }
            let last_sent_at_ms = if inserted.len() < burst_count && index < burst_count {
                now_ms
            } else {
                now_ms - self.config.retry_interval_ms as f64
            };
            self.pending.insert(
                *sequence,
                PendingNack {
                    first_seen_at_ms: now_ms,
                    last_sent_at_ms,
                    deadline_at_ms: deadline_at_ms
                        .unwrap_or(now_ms + self.config.frame_deadline_ms as f64),
                    retry_count: 0,
                },
            );
            inserted.push(*sequence);
        }

        if inserted.is_empty() {
            None
        } else {
            Some(NackBatch {
                sequences: inserted.into_iter().take(burst_count).collect(),
                retry_count: 0,
            })
        }
    }

    pub fn poll(&mut self, now_ms: f64) -> NackPollResult {
        let mut expired_deadline_sequences = Vec::new();
        let mut expired_max_age_sequences = Vec::new();
        self.pending.retain(|sequence, pending| {
            let age_ms = (now_ms - pending.first_seen_at_ms).max(0.0);
            if now_ms >= pending.deadline_at_ms {
                expired_deadline_sequences.push(*sequence);
                return false;
            }
            if age_ms >= self.config.max_age_ms as f64 {
                expired_max_age_sequences.push(*sequence);
                return false;
            }
            true
        });

        let mut retry_sequences = Vec::new();
        let mut next_retry_count = 0u8;
        let burst_count = usize::from(self.config.burst_count.max(1));
        for (sequence, pending) in &mut self.pending {
            let since_last_sent_ms = (now_ms - pending.last_sent_at_ms).max(0.0);
            if pending.retry_count >= self.config.max_retry_count {
                continue;
            }
            if since_last_sent_ms < self.config.retry_interval_ms as f64 {
                continue;
            }
            pending.retry_count = pending.retry_count.saturating_add(1);
            pending.last_sent_at_ms = now_ms;
            next_retry_count = pending.retry_count;
            retry_sequences.push(*sequence);
            if retry_sequences.len() >= burst_count {
                break;
            }
        }

        NackPollResult {
            retry_batch: if retry_sequences.is_empty() {
                None
            } else {
                Some(NackBatch {
                    sequences: retry_sequences,
                    retry_count: next_retry_count,
                })
            },
            expired_batches: vec![
                ExpiredNackBatch {
                    sequences: expired_deadline_sequences,
                    reason: "deadline".to_string(),
                },
                ExpiredNackBatch {
                    sequences: expired_max_age_sequences,
                    reason: "maxAge".to_string(),
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
        })
    }

    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }
}

fn sequence_range(start: u16, end_exclusive: u16) -> Vec<u16> {
    let mut sequences = Vec::new();
    let mut cursor = start;
    while cursor != end_exclusive {
        sequences.push(cursor);
        cursor = cursor.wrapping_add(1);
    }
    sequences
}

#[cfg(test)]
mod tests {
    use super::{NackScheduler, NackSchedulerConfig};

    #[test]
    fn observe_gap_caps_initial_batch_and_releases_overflow_on_poll() {
        let mut scheduler = NackScheduler::new(NackSchedulerConfig {
            max_age_ms: 200,
            frame_deadline_ms: 120,
            burst_count: 2,
            retry_interval_ms: 40,
            max_retry_count: 3,
        });

        let initial = scheduler
            .observe_gap(10, 15, 1_000.0, None)
            .expect("initial batch");
        assert_eq!(initial.sequences, vec![10, 11]);

        let retry = scheduler.poll(1_000.0).retry_batch.expect("overflow batch");
        assert_eq!(retry.sequences, vec![12, 13]);
    }

    #[test]
    fn observe_gap_supports_sequence_wrap() {
        let mut scheduler = NackScheduler::new(NackSchedulerConfig {
            max_age_ms: 200,
            frame_deadline_ms: 120,
            burst_count: 4,
            retry_interval_ms: 40,
            max_retry_count: 3,
        });

        let initial = scheduler
            .observe_gap(u16::MAX, 2, 1_000.0, None)
            .expect("wrapped batch");
        assert_eq!(initial.sequences, vec![u16::MAX, 0, 1]);
    }
}
