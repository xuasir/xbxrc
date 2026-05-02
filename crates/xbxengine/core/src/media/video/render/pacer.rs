use std::time::{Duration, Instant};

const DEFAULT_CATCH_UP_THRESHOLD_MS: u64 = 500;
const DEFAULT_LONG_SLEEP_GUARD_MS: u64 = 20;
const MIN_REFRESH_INTERVAL_MS: u64 = 6;
const MAX_REFRESH_INTERVAL_MS: u64 = 34;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FramePacingAction {
    Drop,
    Ready,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FramePacingDecision {
    pub action: FramePacingAction,
    pub enter_catch_up_mode: bool,
    pub exit_catch_up_mode: bool,
}

impl FramePacingDecision {
    fn drop(enter_catch_up_mode: bool) -> Self {
        Self {
            action: FramePacingAction::Drop,
            enter_catch_up_mode,
            exit_catch_up_mode: false,
        }
    }

    fn ready(exit_catch_up_mode: bool) -> Self {
        Self {
            action: FramePacingAction::Ready,
            enter_catch_up_mode: false,
            exit_catch_up_mode,
        }
    }
}

/**
 * pacing v1 先收敛成纯策略对象：
 * - 不直接依赖线程或渲染器，便于 actor/未来 display-link 复用
 * - 统一处理 catch-up、长睡眠保护和 deadline 判定
 */
#[derive(Clone, Copy, Debug)]
pub(crate) struct FramePacingPolicy {
    catch_up_threshold: Duration,
    long_sleep_guard: Duration,
}

impl FramePacingPolicy {
    #[allow(dead_code)]
    pub(crate) fn new(refresh_interval_ms: u64) -> Self {
        Self::with_dynamic_budget(refresh_interval_ms, None, None, None, None)
    }

    pub(crate) fn with_dynamic_budget(
        refresh_interval_ms: u64,
        catch_up_threshold_ms: Option<u64>,
        long_sleep_guard_ms: Option<u64>,
        video_rtt_ms: Option<f64>,
        video_nack_recovery_rtt_ms: Option<f64>,
    ) -> Self {
        let normalized_refresh_interval_ms =
            refresh_interval_ms.clamp(MIN_REFRESH_INTERVAL_MS, MAX_REFRESH_INTERVAL_MS);

        // RTT 感知的 catch-up 阈值：max(500ms, 2 × RTT + jitter_buffer_max_delay)
        // 优先使用 NACK recovery RTT（更准确），回退到 video RTT
        let rtt_aware_threshold_ms = video_nack_recovery_rtt_ms.or(video_rtt_ms).map(|rtt_ms| {
            // 2 × RTT + 保守的 jitter buffer 估计（30ms）
            let threshold = (2.0 * rtt_ms + 30.0).round() as u64;
            threshold.max(DEFAULT_CATCH_UP_THRESHOLD_MS)
        });

        Self {
            catch_up_threshold: Duration::from_millis(
                catch_up_threshold_ms
                    .or(rtt_aware_threshold_ms)
                    .unwrap_or(DEFAULT_CATCH_UP_THRESHOLD_MS)
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
        catch_up_mode: bool,
        _host_release_wait: Option<Duration>,
    ) -> FramePacingDecision {
        if catch_up_mode {
            if now > deadline + self.catch_up_threshold {
                return FramePacingDecision::drop(true);
            }
            FramePacingDecision::ready(true)
        } else if now > deadline + self.catch_up_threshold {
            FramePacingDecision::drop(true)
        } else {
            let _ = self.long_sleep_guard;
            FramePacingDecision::ready(false)
        }
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
        let decision = policy.decide(now, deadline, false, None);
        assert_eq!(decision.action, FramePacingAction::Drop);
        assert!(decision.enter_catch_up_mode);
    }

    #[test]
    fn pacing_marks_short_early_gap_as_ready() {
        let policy = FramePacingPolicy::new(16);
        let now = Instant::now();
        let deadline = now + Duration::from_millis(10);
        let decision = policy.decide(now, deadline, false, None);
        assert_eq!(decision.action, FramePacingAction::Ready);
    }

    #[test]
    fn pacing_exits_catch_up_when_deadline_returns() {
        let policy = FramePacingPolicy::new(16);
        let now = Instant::now();
        let deadline = now - Duration::from_millis(100);
        let decision = policy.decide(now, deadline, true, None);
        assert_eq!(decision.action, FramePacingAction::Ready);
        assert!(decision.exit_catch_up_mode);
    }

    #[test]
    fn pacing_uses_zero_sleep_guard_override_to_submit_now_for_short_gap() {
        let policy = FramePacingPolicy::with_dynamic_budget(16, None, Some(0), None, None);
        let now = Instant::now();
        let deadline = now + Duration::from_millis(10);
        let decision = policy.decide(now, deadline, false, None);
        assert_eq!(decision.action, FramePacingAction::Ready);
    }

    #[test]
    fn pacing_submits_due_frame_without_host_release_gate() {
        let policy = FramePacingPolicy::new(16);
        let now = Instant::now();
        let deadline = now - Duration::from_millis(1);
        let decision = policy.decide(now, deadline, false, Some(Duration::from_millis(5)));
        assert_eq!(decision.action, FramePacingAction::Ready);
    }

    #[test]
    fn pacing_keeps_deadline_sleep_without_host_release_gate_override() {
        let policy = FramePacingPolicy::new(16);
        let now = Instant::now();
        let deadline = now + Duration::from_millis(10);
        let decision = policy.decide(now, deadline, false, Some(Duration::from_millis(4)));
        assert_eq!(decision.action, FramePacingAction::Ready);
    }
}
