//! NACK 耗尽后的关键帧升级排队：对齐 libwebrtc「先修洞、dwell 后再 PLI」，避免立即 hard fallback。

use std::time::{Duration, Instant};

/// receiver-local：单次修洞周期内合并为一次 PLI 升级。
#[derive(Debug, Default)]
pub struct KeyframeEscalationQueue {
    due_at: Option<Instant>,
}

impl KeyframeEscalationQueue {
    pub fn arm(&mut self, dwell: Duration, now: Instant) {
        if self.due_at.is_none() {
            self.due_at = Some(now + dwell);
        }
    }

    /// 同一 seq 反复 NACK 仍无解码进展时，跳过 dwell 立即升级关键帧。
    pub fn arm_immediate(&mut self, now: Instant) {
        self.due_at = Some(now);
    }

    pub fn is_armed(&self) -> bool {
        self.due_at.is_some()
    }

    pub fn poll_due(&mut self, now: Instant) -> bool {
        let Some(due) = self.due_at else {
            return false;
        };
        if now < due {
            return false;
        }
        self.due_at = None;
        true
    }

    pub fn clear(&mut self) {
        self.due_at = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arm_immediate_makes_keyframe_due_on_next_poll() {
        let mut queue = KeyframeEscalationQueue::default();
        let start = Instant::now();
        queue.arm_immediate(start);
        assert!(queue.poll_due(start));
    }

    #[test]
    fn dwell_delays_keyframe_until_due() {
        let mut queue = KeyframeEscalationQueue::default();
        let start = Instant::now();
        queue.arm(Duration::from_millis(80), start);
        assert!(!queue.poll_due(start + Duration::from_millis(40)));
        assert!(queue.poll_due(start + Duration::from_millis(80)));
        assert!(!queue.is_armed());
    }
}
