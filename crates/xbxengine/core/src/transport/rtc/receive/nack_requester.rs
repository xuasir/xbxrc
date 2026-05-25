use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use super::keyframe_escalation_queue::KeyframeEscalationQueue;
use super::packet_buffer::PacketBuffer;
use super::timing::{NackSchedulingParams, ReceiveTimingProfile};

#[derive(Debug)]
struct PendingNack {
    first_seen: Instant,
    first_sent: Option<Instant>,
    last_sent_at: Option<Instant>,
    retry_count: u8,
    exhausted: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoveredPacketSource {
    Rtx,
    #[allow(dead_code)]
    Fec,
}

/// 单次 poll 结果：待发 NACK 列表、每条 seq 的重试计数、是否应触发排队后的关键帧升级。
#[derive(Debug, Default)]
pub struct NackPollResult {
    pub sequences: Vec<u16>,
    pub retry_counts: Vec<u8>,
    pub keyframe_escalation_due: bool,
}

impl NackPollResult {
    pub fn is_empty(&self) -> bool {
        self.sequences.is_empty() && !self.keyframe_escalation_due
    }
}

/// receiver-local NACK：seq gap + RTT 感知时序；RTX/FEC 恢复 seq 不再进入 NACK。
pub struct NackRequester {
    _timing: ReceiveTimingProfile,
    pending: BTreeMap<u16, PendingNack>,
    recovered: BTreeSet<u16>,
    keyframe_escalation: KeyframeEscalationQueue,
}

impl NackRequester {
    pub fn new(timing: ReceiveTimingProfile) -> Self {
        Self {
            _timing: timing,
            pending: BTreeMap::new(),
            recovered: BTreeSet::new(),
            keyframe_escalation: KeyframeEscalationQueue::default(),
        }
    }

    pub fn register_gaps(&mut self, gaps: impl IntoIterator<Item = u16>) {
        let now = Instant::now();
        for seq in gaps {
            if self.recovered.contains(&seq) {
                continue;
            }
            self.pending.entry(seq).or_insert(PendingNack {
                first_seen: now,
                first_sent: None,
                last_sent_at: None,
                retry_count: 0,
                exhausted: false,
            });
        }
    }

    pub fn mark_recovered(&mut self, sequence: u16, _source: RecoveredPacketSource) -> bool {
        self.recovered.insert(sequence);
        self.pending.remove(&sequence).is_some()
    }

    pub fn resolve(&mut self, sequence: u16) -> bool {
        self.pending.remove(&sequence).is_some()
    }

    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    pub fn clear_keyframe_escalation(&mut self) {
        self.keyframe_escalation.clear();
    }

    pub fn nack_escalation_pending(&self) -> bool {
        self.keyframe_escalation.is_armed()
    }

    pub fn has_exhausted_gaps(&self) -> bool {
        self.pending.values().any(|entry| entry.exhausted)
    }

    #[cfg(test)]
    pub(crate) fn keyframe_escalation_armed(&self) -> bool {
        self.keyframe_escalation.is_armed()
    }

    pub fn on_keyframe_escalation_sent(&mut self) {
        self.keyframe_escalation.clear();
        for entry in self.pending.values_mut() {
            if entry.exhausted {
                entry.exhausted = false;
                entry.first_sent = None;
                entry.last_sent_at = None;
                entry.retry_count = 0;
            }
        }
    }

    pub fn poll(&mut self, params: &NackSchedulingParams, now: Instant) -> NackPollResult {
        let mut result = NackPollResult::default();
        let mut should_arm_escalation = false;

        for (seq, entry) in self.pending.iter_mut() {
            if self.recovered.contains(seq) {
                continue;
            }
            let age = now.saturating_duration_since(entry.first_seen);
            if age < params.reorder_wait {
                continue;
            }
            if entry.exhausted {
                continue;
            }

            if entry.first_sent.is_none() {
                if age >= params.first_nack {
                    entry.first_sent = Some(now);
                    entry.last_sent_at = Some(now);
                    result.sequences.push(*seq);
                    result.retry_counts.push(entry.retry_count);
                }
                continue;
            }

            let since_last = entry
                .last_sent_at
                .map(|t| now.saturating_duration_since(t))
                .unwrap_or(Duration::ZERO);

            if entry.retry_count < params.max_retries && since_last >= params.retry_interval {
                entry.retry_count = entry.retry_count.saturating_add(1);
                entry.last_sent_at = Some(now);
                result.sequences.push(*seq);
                result.retry_counts.push(entry.retry_count);
                continue;
            }

            let timed_out = age >= params.nack_timeout;
            let retries_exhausted = entry.retry_count >= params.max_retries;
            if timed_out || retries_exhausted {
                entry.exhausted = true;
                should_arm_escalation = true;
            }
        }

        if should_arm_escalation {
            self.keyframe_escalation
                .arm(params.keyframe_escalation_dwell, now);
        }

        result.keyframe_escalation_due = self.keyframe_escalation.poll_due(now);
        result
    }

    pub fn sync_from_buffer(&mut self, buffer: &PacketBuffer) {
        self.register_gaps(buffer.all_missing());
    }

    /// 兼容旧调用：返回序列列表与是否应关键帧升级（无 per-seq retry 时取 0）。
    pub fn poll_ready_sequences(&mut self, now: Instant) -> (Vec<u16>, bool) {
        let params = self._timing.nack_scheduling_params(100.0);
        let result = self.poll(&params, now);
        (result.sequences, result.keyframe_escalation_due)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cloud_timing() -> ReceiveTimingProfile {
        ReceiveTimingProfile::for_target(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud))
    }

    #[test]
    fn rtt_increases_first_nack_delay() {
        let profile = cloud_timing();
        let low = profile.nack_scheduling_params(20.0);
        let high = profile.nack_scheduling_params(200.0);
        assert!(high.first_nack > low.first_nack);
        assert!(high.retry_interval >= low.retry_interval);
    }

    #[test]
    fn recovered_sequence_is_not_nacked() {
        let mut requester = NackRequester::new(cloud_timing());
        requester.register_gaps([10_u16, 11]);
        requester.mark_recovered(10, RecoveredPacketSource::Rtx);
        let start = Instant::now();
        let params = cloud_timing().nack_scheduling_params(50.0);
        std::thread::sleep(Duration::from_millis(25));
        let result = requester.poll(&params, start + Duration::from_millis(25));
        assert!(!result.sequences.contains(&10));
    }

    #[test]
    fn exhaustion_arms_keyframe_queue_before_due() {
        let mut requester = NackRequester::new(cloud_timing());
        requester.register_gaps([42_u16]);
        let start = Instant::now();
        let mut params = cloud_timing().nack_scheduling_params(10.0);
        params.first_nack = Duration::from_millis(1);
        params.retry_interval = Duration::from_millis(1);
        params.max_retries = 0;
        params.nack_timeout = Duration::from_millis(5);
        params.keyframe_escalation_dwell = Duration::from_millis(50);
        params.reorder_wait = Duration::from_millis(1);

        let t1 = start + Duration::from_millis(3);
        let r1 = requester.poll(&params, t1);
        assert!(!r1.sequences.is_empty());
        let r2 = requester.poll(&params, t1 + Duration::from_millis(2));
        assert!(r2.sequences.is_empty());
        assert!(!r2.keyframe_escalation_due);
        assert!(requester.keyframe_escalation_armed());

        let r3 = requester.poll(&params, t1 + Duration::from_millis(60));
        assert!(r3.keyframe_escalation_due);
    }
}
