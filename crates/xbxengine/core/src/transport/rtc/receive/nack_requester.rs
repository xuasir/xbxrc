use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use super::keyframe_escalation_queue::KeyframeEscalationQueue;
use super::packet_buffer::PacketBuffer;
use super::timing::{NackSchedulingParams, ReceiveTimingProfile};

/// 单 seq 累计发送次数超过该阈值仍无恢复时，强制关键帧升级（不再重开 NACK 周期）。
const STUCK_SEQUENCE_SEND_THRESHOLD: u8 = 12;

#[derive(Debug)]
struct PendingNack {
    first_seen: Instant,
    first_sent: Option<Instant>,
    last_sent_at: Option<Instant>,
    retry_count: u8,
    total_send_count: u8,
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

/// receiver-local NACK：seq gap + RTT 感知时序；RTX/FEC 恢复 seq 不再进入 NACK。
pub struct NackRequester {
    _timing: ReceiveTimingProfile,
    pending: BTreeMap<u16, PendingNack>,
    recovered: BTreeSet<u16>,
    /// 已耗尽重试的 seq；在关键帧升级前禁止重新入队，避免对同一洞无限 NACK。
    exhausted_sequences: BTreeSet<u16>,
    keyframe_escalation: KeyframeEscalationQueue,
}

impl NackRequester {
    pub fn new(timing: ReceiveTimingProfile) -> Self {
        Self {
            _timing: timing,
            pending: BTreeMap::new(),
            recovered: BTreeSet::new(),
            exhausted_sequences: BTreeSet::new(),
            keyframe_escalation: KeyframeEscalationQueue::default(),
        }
    }

    pub fn register_gaps(&mut self, gaps: impl IntoIterator<Item = u16>) {
        let now = Instant::now();
        for seq in gaps {
            if self.recovered.contains(&seq) {
                continue;
            }
            if self.exhausted_sequences.contains(&seq) {
                self.keyframe_escalation.arm_immediate(now);
                continue;
            }
            self.pending.entry(seq).or_insert(PendingNack {
                first_seen: now,
                first_sent: None,
                last_sent_at: None,
                retry_count: 0,
                total_send_count: 0,
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
        self.exhausted_sequences.clear();
        for entry in self.pending.values_mut() {
            if entry.exhausted {
                entry.exhausted = false;
                entry.first_sent = None;
                entry.last_sent_at = None;
                entry.retry_count = 0;
                entry.total_send_count = 0;
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
                    entry.total_send_count = entry.total_send_count.saturating_add(1);
                    result.sequences.push(*seq);
                    result.retry_counts.push(entry.retry_count);
                    if entry.total_send_count >= STUCK_SEQUENCE_SEND_THRESHOLD {
                        should_arm_escalation = true;
                    }
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
                entry.total_send_count = entry.total_send_count.saturating_add(1);
                result.sequences.push(*seq);
                result.retry_counts.push(entry.retry_count);
                if entry.total_send_count >= STUCK_SEQUENCE_SEND_THRESHOLD {
                    should_arm_escalation = true;
                }
                continue;
            }

            let timed_out = age >= params.nack_timeout;
            let retries_exhausted = entry.retry_count >= params.max_retries;
            if timed_out || retries_exhausted {
                entry.exhausted = true;
                self.exhausted_sequences.insert(*seq);
                should_arm_escalation = true;
            }
        }

        if should_arm_escalation {
            if self
                .pending
                .values()
                .any(|entry| entry.total_send_count >= STUCK_SEQUENCE_SEND_THRESHOLD)
                || !self.exhausted_sequences.is_empty()
            {
                self.keyframe_escalation.arm_immediate(now);
            } else {
                self.keyframe_escalation
                    .arm(params.keyframe_escalation_dwell, now);
            }
        }

        result.keyframe_escalation_due = self.keyframe_escalation.poll_due(now);
        result
    }

    pub fn sync_from_buffer(&mut self, buffer: &PacketBuffer) {
        self.register_gaps(buffer.all_missing());
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
    fn exhausted_sequence_reregistration_arms_immediate_keyframe() {
        let mut requester = NackRequester::new(cloud_timing());
        requester.register_gaps([42_u16]);
        let start = Instant::now();
        let mut params = cloud_timing().nack_scheduling_params(10.0);
        params.first_nack = Duration::from_millis(1);
        params.retry_interval = Duration::from_millis(1);
        params.max_retries = 0;
        params.nack_timeout = Duration::from_millis(5);
        params.reorder_wait = Duration::from_millis(1);

        let t1 = start + Duration::from_millis(3);
        let _ = requester.poll(&params, t1);
        let _ = requester.poll(&params, t1 + Duration::from_millis(2));
        assert!(requester.has_exhausted_gaps());

        requester.register_gaps([42_u16]);
        assert!(requester.keyframe_escalation_armed());
        let r = requester.poll(&params, t1 + Duration::from_millis(3));
        assert!(r.keyframe_escalation_due);
    }

    #[test]
    fn repeated_sends_on_same_sequence_force_immediate_keyframe() {
        let mut requester = NackRequester::new(cloud_timing());
        requester.register_gaps([99_u16]);
        let start = Instant::now();
        let mut params = cloud_timing().nack_scheduling_params(10.0);
        params.first_nack = Duration::from_millis(1);
        params.retry_interval = Duration::from_millis(1);
        params.max_retries = 20;
        params.nack_timeout = Duration::from_secs(60);
        params.reorder_wait = Duration::from_millis(1);

        let mut armed = false;
        for step in 0..20 {
            let now = start + Duration::from_millis(2 + step * 3);
            let result = requester.poll(&params, now);
            if result.keyframe_escalation_due {
                armed = true;
                break;
            }
        }
        assert!(armed, "stuck sequence should escalate to keyframe");
    }

    #[test]
    fn exhaustion_arms_keyframe_escalation_when_gap_times_out() {
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
        assert!(!requester.keyframe_escalation_armed());

        let t_exhaust = t1 + params.nack_timeout + Duration::from_millis(2);
        let r2 = requester.poll(&params, t_exhaust);
        assert!(r2.sequences.is_empty());
        assert!(r2.keyframe_escalation_due);
        assert!(!requester.keyframe_escalation_armed());
    }
}
