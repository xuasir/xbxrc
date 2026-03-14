use crate::media::video::types::FrameValue;

pub struct FrameDeadlineTracker {
    fallback_deadline_ms: u64,
    estimated_frame_interval_ms: f64,
    last_target_playout_at_ms: Option<f64>,
}

impl FrameDeadlineTracker {
    pub fn new(fallback_deadline_ms: u64) -> Self {
        Self {
            fallback_deadline_ms,
            estimated_frame_interval_ms: 33.0,
            last_target_playout_at_ms: None,
        }
    }

    pub fn record_frame_target(&mut self, target_playout_at_ms: f64) {
        if let Some(previous_target_playout_at_ms) = self.last_target_playout_at_ms {
            let observed_interval_ms =
                (target_playout_at_ms - previous_target_playout_at_ms).clamp(16.0, 100.0);
            self.estimated_frame_interval_ms =
                (self.estimated_frame_interval_ms * 0.7) + (observed_interval_ms * 0.3);
        }
        self.last_target_playout_at_ms = Some(target_playout_at_ms);
    }

    pub fn next_deadline_at_ms(&self, now_ms: f64) -> f64 {
        self.next_deadline_for_value_at_ms(now_ms, FrameValue::Delta)
    }

    pub fn next_deadline_for_value_at_ms(&self, now_ms: f64, value: FrameValue) -> f64 {
        let next_target_at_ms = self
            .last_target_playout_at_ms
            .map(|target| target + self.estimated_frame_interval_ms)
            .unwrap_or(now_ms);
        let value_deadline_ms = match value {
            // 关键帧更值得等待，给满窗口。
            FrameValue::Keyframe => self.fallback_deadline_ms as f64,
            // delta 帧更强调时效，过期就不要继续拖累恢复链。
            FrameValue::Delta => (self.fallback_deadline_ms as f64)
                .min((self.estimated_frame_interval_ms * 2.0).max(24.0)),
        };
        next_target_at_ms + value_deadline_ms
    }
}

#[cfg(test)]
mod tests {
    use super::FrameDeadlineTracker;
    use crate::media::video::types::FrameValue;

    #[test]
    fn delta_deadline_is_tighter_than_keyframe_deadline() {
        let mut tracker = FrameDeadlineTracker::new(250);
        tracker.record_frame_target(1_000.0);
        tracker.record_frame_target(1_033.0);

        let delta_deadline = tracker.next_deadline_for_value_at_ms(1_040.0, FrameValue::Delta);
        let keyframe_deadline =
            tracker.next_deadline_for_value_at_ms(1_040.0, FrameValue::Keyframe);

        assert!(delta_deadline < keyframe_deadline);
    }
}
