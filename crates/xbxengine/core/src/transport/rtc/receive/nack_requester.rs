use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use super::packet_buffer::PacketBuffer;
use super::timing::ReceiveTimingProfile;

#[derive(Debug)]
struct PendingNack {
    first_seen: Instant,
    first_sent: Option<Instant>,
    retry_sent: bool,
}

/// receiver-local NACK：仅 seq gap + RTT 感知时序，无 host/display admission。
pub struct NackRequester {
    timing: ReceiveTimingProfile,
    pending: BTreeMap<u16, PendingNack>,
}

impl NackRequester {
    pub fn new(timing: ReceiveTimingProfile) -> Self {
        Self {
            timing,
            pending: BTreeMap::new(),
        }
    }

    pub fn register_gaps(&mut self, gaps: impl IntoIterator<Item = u16>) {
        let now = Instant::now();
        for seq in gaps {
            self.pending.entry(seq).or_insert(PendingNack {
                first_seen: now,
                first_sent: None,
                retry_sent: false,
            });
        }
    }

    pub fn resolve(&mut self, sequence: u16) -> bool {
        self.pending.remove(&sequence).is_some()
    }

    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    pub fn poll_ready_sequences(&mut self, now: Instant) -> (Vec<u16>, bool) {
        let reorder_wait = Duration::from_millis(self.timing.reorder_wait_ms);
        let first_nack = Duration::from_millis(self.timing.first_nack_ms);
        let retry_wait = Duration::from_millis(self.timing.nack_retry_ms);
        let fallback = Duration::from_millis(self.timing.keyframe_fallback_ms);

        let mut batch = Vec::new();
        let mut keyframe_fallback = false;

        for (seq, entry) in self.pending.iter_mut() {
            let age = now.saturating_duration_since(entry.first_seen);
            if age < reorder_wait {
                continue;
            }
            if entry.first_sent.is_none() && age >= first_nack {
                entry.first_sent = Some(now);
                batch.push(*seq);
                continue;
            }
            if let Some(first_sent) = entry.first_sent {
                if !entry.retry_sent && now.saturating_duration_since(first_sent) >= retry_wait {
                    entry.retry_sent = true;
                    batch.push(*seq);
                } else if now.saturating_duration_since(entry.first_seen) >= fallback {
                    keyframe_fallback = true;
                }
            }
        }

        if keyframe_fallback {
            self.pending.clear();
        }

        (batch, keyframe_fallback)
    }

    pub fn sync_from_buffer(&mut self, buffer: &PacketBuffer) {
        self.register_gaps(buffer.all_missing());
    }
}
