use std::time::{Duration, Instant};

const DEFAULT_CATCH_UP_THRESHOLD_MS: u64 = 500;
const DEFAULT_LONG_SLEEP_GUARD_MS: u64 = 20;
const MIN_REFRESH_INTERVAL_MS: u64 = 6;
const MAX_REFRESH_INTERVAL_MS: u64 = 34;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FramePacingAction {
    Drop,
    SubmitNow,
    Sleep(Duration),
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

    fn submit_now(exit_catch_up_mode: bool) -> Self {
        Self {
            action: FramePacingAction::SubmitNow,
            enter_catch_up_mode: false,
            exit_catch_up_mode,
        }
    }

    fn sleep(duration: Duration, exit_catch_up_mode: bool) -> Self {
        Self {
            action: FramePacingAction::Sleep(duration),
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
    pub(crate) fn new(refresh_interval_ms: u64) -> Self {
        Self::with_dynamic_budget(refresh_interval_ms, None)
    }

    pub(crate) fn with_dynamic_budget(
        refresh_interval_ms: u64,
        catch_up_threshold_ms: Option<u64>,
    ) -> Self {
        let normalized_refresh_interval_ms =
            refresh_interval_ms.clamp(MIN_REFRESH_INTERVAL_MS, MAX_REFRESH_INTERVAL_MS);
        Self {
            catch_up_threshold: Duration::from_millis(
                catch_up_threshold_ms
                    .unwrap_or(DEFAULT_CATCH_UP_THRESHOLD_MS)
                    .max(1),
            ),
            long_sleep_guard: Duration::from_millis(
                normalized_refresh_interval_ms.min(DEFAULT_LONG_SLEEP_GUARD_MS),
            ),
        }
    }

    pub(crate) fn decide(
        &self,
        now: Instant,
        deadline: Instant,
        catch_up_mode: bool,
    ) -> FramePacingDecision {
        if catch_up_mode {
            if now > deadline + self.catch_up_threshold {
                return FramePacingDecision::drop(true);
            }
            return FramePacingDecision::submit_now(true);
        }

        if now > deadline + self.catch_up_threshold {
            return FramePacingDecision::drop(true);
        }

        if now >= deadline {
            return FramePacingDecision::submit_now(false);
        }

        let sleep_time = deadline.duration_since(now);
        if sleep_time <= self.long_sleep_guard {
            return FramePacingDecision::sleep(sleep_time, false);
        }

        // target playout 异常偏大时，优先快速追帧，不长时间阻塞线程。
        FramePacingDecision::submit_now(false)
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
        let decision = policy.decide(now, deadline, false);
        assert_eq!(decision.action, FramePacingAction::Drop);
        assert!(decision.enter_catch_up_mode);
    }

    #[test]
    fn pacing_sleeps_for_short_early_gap() {
        let policy = FramePacingPolicy::new(16);
        let now = Instant::now();
        let deadline = now + Duration::from_millis(10);
        let decision = policy.decide(now, deadline, false);
        assert_eq!(
            decision.action,
            FramePacingAction::Sleep(Duration::from_millis(10))
        );
    }

    #[test]
    fn pacing_exits_catch_up_when_deadline_returns() {
        let policy = FramePacingPolicy::new(16);
        let now = Instant::now();
        let deadline = now - Duration::from_millis(100);
        let decision = policy.decide(now, deadline, true);
        assert_eq!(decision.action, FramePacingAction::SubmitNow);
        assert!(decision.exit_catch_up_mode);
    }
}
