#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FrameReleasePolicy {
    min_delay_ms: f64,
    max_delay_ms: f64,
}

impl FrameReleasePolicy {
    pub(crate) fn new(min_delay_ms: u64, max_delay_ms: u64) -> Self {
        let bounded_min = min_delay_ms.min(max_delay_ms.max(1));
        let bounded_max = max_delay_ms.max(bounded_min).max(1);
        Self {
            min_delay_ms: bounded_min as f64,
            max_delay_ms: bounded_max as f64,
        }
    }

    // 单帧时最多等到 max；多帧时达到 min 就尽快释放，保持低延迟。
    pub(crate) fn should_release(&self, queue_delay_ms: f64, buffered_frames: usize) -> bool {
        queue_delay_ms >= self.max_delay_ms
            || (queue_delay_ms >= self.min_delay_ms && buffered_frames > 1)
    }
}

#[cfg(test)]
mod tests {
    use super::FrameReleasePolicy;

    #[test]
    fn frame_release_policy_respects_min_and_max_window() {
        let policy = FrameReleasePolicy::new(20, 30);
        assert!(!policy.should_release(19.0, 2));
        assert!(policy.should_release(20.0, 2));
        assert!(!policy.should_release(20.0, 1));
        assert!(policy.should_release(30.0, 1));
    }
}
