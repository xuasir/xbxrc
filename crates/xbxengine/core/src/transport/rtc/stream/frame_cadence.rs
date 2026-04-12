use crate::media::video::ingress::budget::{FrameBudgetContext, FrameBudgetWindowSource};
use crate::media::video::types::FrameValue;

/// transport 侧只关心“帧到达节奏”和“还能给 NACK 多久恢复窗口”，
/// 不负责 playout target 的最终定义。
pub(super) struct TransportFrameDeadlineTracker {
    fallback_deadline_ms: u64,
    estimated_frame_interval_ms: f64,
    last_frame_arrival_at_ms: Option<f64>,
}

impl TransportFrameDeadlineTracker {
    pub(super) fn new(fallback_deadline_ms: u64) -> Self {
        Self {
            fallback_deadline_ms,
            estimated_frame_interval_ms: 33.0,
            last_frame_arrival_at_ms: None,
        }
    }

    pub(super) fn record_frame_arrival(&mut self, frame_arrival_at_ms: f64) {
        if let Some(previous_arrival_at_ms) = self.last_frame_arrival_at_ms {
            let observed_interval_ms =
                (frame_arrival_at_ms - previous_arrival_at_ms).clamp(16.0, 100.0);
            self.estimated_frame_interval_ms =
                (self.estimated_frame_interval_ms * 0.7) + (observed_interval_ms * 0.3);
        }
        self.last_frame_arrival_at_ms = Some(frame_arrival_at_ms);
    }

    #[allow(dead_code)]
    pub(super) fn next_transport_deadline_for_value_at_ms(
        &self,
        now_ms: f64,
        value: FrameValue,
    ) -> f64 {
        self.next_transport_deadline_with_context_at_ms(
            now_ms,
            value,
            FrameBudgetContext {
                window_source: FrameBudgetWindowSource::Transport,
                ..FrameBudgetContext::steady_for_value(value)
            },
        )
    }

    pub(super) fn next_transport_deadline_with_context_at_ms(
        &self,
        now_ms: f64,
        value: FrameValue,
        context: FrameBudgetContext,
    ) -> f64 {
        let next_arrival_at_ms = self
            .last_frame_arrival_at_ms
            .map(|arrival| arrival + self.estimated_frame_interval_ms)
            .unwrap_or(now_ms);
        let raw_deadline_ms = (self.fallback_deadline_ms as f64)
            * (context.deadline_budget_ratio_per_mille(value) as f64 / 1_000.0);
        let value_deadline_ms = match context.frame_importance() {
            "keyframe" => raw_deadline_ms.max(self.estimated_frame_interval_ms),
            "reference" => raw_deadline_ms
                .min((self.estimated_frame_interval_ms * 3.0).max(36.0))
                .max(36.0),
            _ => raw_deadline_ms
                .min((self.estimated_frame_interval_ms * 2.0).max(24.0))
                .max(24.0),
        };
        next_arrival_at_ms + value_deadline_ms
    }
}

#[cfg(test)]
mod tests {
    use super::TransportFrameDeadlineTracker;
    use crate::media::video::ingress::budget::{FrameBudgetContext, FrameBudgetWindowSource};
    use crate::media::video::types::FrameValue;

    #[test]
    fn delta_deadline_is_tighter_than_keyframe_deadline() {
        let mut tracker = TransportFrameDeadlineTracker::new(250);
        tracker.record_frame_arrival(1_000.0);
        tracker.record_frame_arrival(1_033.0);

        let delta_deadline = tracker.next_transport_deadline_for_value_at_ms(
            1_040.0,
            FrameValue::new(false, false, 8 * 1024),
        );
        let keyframe_deadline = tracker.next_transport_deadline_for_value_at_ms(
            1_040.0,
            FrameValue::new(true, true, 64 * 1024),
        );

        assert!(delta_deadline < keyframe_deadline);
    }

    #[test]
    fn recovery_context_gives_supply_frame_wider_deadline_than_steady_delta() {
        let mut tracker = TransportFrameDeadlineTracker::new(250);
        tracker.record_frame_arrival(1_000.0);
        tracker.record_frame_arrival(1_033.0);
        let value = FrameValue::new(false, false, 8 * 1024);

        let steady_deadline = tracker.next_transport_deadline_for_value_at_ms(1_040.0, value);
        let recovery_deadline = tracker.next_transport_deadline_with_context_at_ms(
            1_040.0,
            value,
            FrameBudgetContext::for_transport(
                value,
                true,
                Some(120.0),
                Some(1_060.0),
                Some(1_090.0),
                false,
                FrameBudgetWindowSource::Recovery,
            ),
        );

        assert!(recovery_deadline > steady_deadline);
    }
}
