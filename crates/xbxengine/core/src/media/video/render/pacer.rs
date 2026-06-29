use std::time::{Duration, Instant};

const DEFAULT_LATE_DROP_THRESHOLD_MS: u64 = 500;
const DEFAULT_LONG_SLEEP_GUARD_MS: u64 = 20;
const MIN_REFRESH_INTERVAL_MS: u64 = 6;
const MAX_REFRESH_INTERVAL_MS: u64 = 34;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FramePacingAction {
    Drop,
    Hold,
    Ready,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FramePacingDecision {
    pub action: FramePacingAction,
    pub wait_duration: Duration,
}

impl FramePacingDecision {
    fn drop() -> Self {
        Self {
            action: FramePacingAction::Drop,
            wait_duration: Duration::ZERO,
        }
    }

    fn hold(wait_duration: Duration) -> Self {
        Self {
            action: FramePacingAction::Hold,
            wait_duration,
        }
    }

    fn ready() -> Self {
        Self {
            action: FramePacingAction::Ready,
            wait_duration: Duration::ZERO,
        }
    }
}

/**
 * pacing v1 先收敛成纯策略对象：
 * - 不直接依赖线程或渲染器，便于 actor/未来 display-link 复用
 * - 统一处理低延迟丢帧、长睡眠保护和 deadline 判定
 */
#[derive(Clone, Copy, Debug)]
pub(crate) struct FramePacingPolicy {
    late_drop_threshold: Duration,
    long_sleep_guard: Duration,
}

impl FramePacingPolicy {
    #[allow(dead_code)]
    pub(crate) fn new(refresh_interval_ms: u64) -> Self {
        Self::with_dynamic_budget(refresh_interval_ms, None, None, None, None)
    }

    pub(crate) fn with_dynamic_budget(
        refresh_interval_ms: u64,
        late_drop_threshold_ms: Option<u64>,
        long_sleep_guard_ms: Option<u64>,
        video_rtt_ms: Option<f64>,
        video_nack_recovery_rtt_ms: Option<f64>,
    ) -> Self {
        let normalized_refresh_interval_ms =
            refresh_interval_ms.clamp(MIN_REFRESH_INTERVAL_MS, MAX_REFRESH_INTERVAL_MS);

        // RTT 感知的 late-drop 阈值：max(500ms, 2 × RTT + jitter_buffer_max_delay)
        // 优先使用 NACK recovery RTT（更准确），回退到 video RTT
        let rtt_aware_threshold_ms = video_nack_recovery_rtt_ms.or(video_rtt_ms).map(|rtt_ms| {
            // 2 × RTT + 保守的 jitter buffer 估计（30ms）
            let threshold = (2.0 * rtt_ms + 30.0).round() as u64;
            threshold.max(DEFAULT_LATE_DROP_THRESHOLD_MS)
        });

        Self {
            late_drop_threshold: Duration::from_millis(
                late_drop_threshold_ms
                    .or(rtt_aware_threshold_ms)
                    .unwrap_or(DEFAULT_LATE_DROP_THRESHOLD_MS)
                    .max(1),
            ),
            long_sleep_guard: Duration::from_millis(
                long_sleep_guard_ms
                    .unwrap_or(normalized_refresh_interval_ms.min(DEFAULT_LONG_SLEEP_GUARD_MS)),
            ),
        }
    }

    pub(crate) fn decide(
        &self,
        now: Instant,
        deadline: Instant,
        _host_release_wait: Option<Duration>,
    ) -> FramePacingDecision {
        if now > deadline + self.late_drop_threshold {
            return FramePacingDecision::drop();
        }
        if now >= deadline || self.long_sleep_guard.is_zero() {
            return FramePacingDecision::ready();
        }
        let wait_duration = deadline
            .saturating_duration_since(now)
            .min(self.long_sleep_guard);
        FramePacingDecision::hold(wait_duration)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct HostPacingPressure {
    pub cadence_phase: HostCadencePhaseHint,
    pub no_pending_pressure_level: Option<String>,
    pub no_pending_streak: u32,
    pub host_mailbox_overwrite_count_total: u64,
    pub host_mailbox_enqueue_count_total: u64,
    pub present_fps: Option<f64>,
    pub display_fps: Option<f64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum HostCadencePhaseHint {
    Idle,
    Priming,
    Steady,
    Starved,
    #[default]
    Unknown,
}

impl HostCadencePhaseHint {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Priming => "priming",
            Self::Steady => "steady",
            Self::Starved => "starved",
            Self::Unknown => "unknown",
        }
    }

    pub(crate) fn from_stats(value: Option<&str>) -> Self {
        match value {
            Some("idle") => Self::Idle,
            Some("priming") => Self::Priming,
            Some("steady") => Self::Steady,
            Some("starved") => Self::Starved,
            Some(_) => Self::Unknown,
            None => Self::Unknown,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{FramePacingAction, FramePacingPolicy};
    use std::time::{Duration, Instant};

    #[test]
    fn pacing_drops_massively_late_frames() {
        let policy = FramePacingPolicy::new(16);
        let now = Instant::now();
        let deadline = now - Duration::from_millis(600);
        let decision = policy.decide(now, deadline, None);
        assert_eq!(decision.action, FramePacingAction::Drop);
    }

    #[test]
    fn pacing_holds_short_early_gap_until_deadline() {
        let policy = FramePacingPolicy::new(16);
        let now = Instant::now();
        let deadline = now + Duration::from_millis(10);
        let decision = policy.decide(now, deadline, None);
        assert_eq!(decision.action, FramePacingAction::Hold);
        assert!(decision.wait_duration <= Duration::from_millis(10));
    }

    #[test]
    fn pacing_ready_when_deadline_arrives() {
        let policy = FramePacingPolicy::new(16);
        let now = Instant::now();
        let deadline = now - Duration::from_millis(100);
        let decision = policy.decide(now, deadline, None);
        assert_eq!(decision.action, FramePacingAction::Ready);
    }

    #[test]
    fn pacing_uses_zero_sleep_guard_override_to_submit_now_for_short_gap() {
        let policy = FramePacingPolicy::with_dynamic_budget(16, None, Some(0), None, None);
        let now = Instant::now();
        let deadline = now + Duration::from_millis(10);
        let decision = policy.decide(now, deadline, None);
        assert_eq!(decision.action, FramePacingAction::Ready);
    }

    #[test]
    fn pacing_submits_due_frame_without_host_release_gate() {
        let policy = FramePacingPolicy::new(16);
        let now = Instant::now();
        let deadline = now - Duration::from_millis(1);
        let decision = policy.decide(now, deadline, Some(Duration::from_millis(5)));
        assert_eq!(decision.action, FramePacingAction::Ready);
    }

    #[test]
    fn pacing_keeps_deadline_sleep_without_host_release_gate_override() {
        let policy = FramePacingPolicy::new(16);
        let now = Instant::now();
        let deadline = now + Duration::from_millis(10);
        let decision = policy.decide(now, deadline, Some(Duration::from_millis(4)));
        assert_eq!(decision.action, FramePacingAction::Hold);
    }
}
