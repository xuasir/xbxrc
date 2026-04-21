use std::collections::VecDeque;
use std::time::{Duration, Instant};

const DEFAULT_CATCH_UP_THRESHOLD_MS: u64 = 500;
const DEFAULT_LONG_SLEEP_GUARD_MS: u64 = 20;
const MIN_REFRESH_INTERVAL_MS: u64 = 6;
const MAX_REFRESH_INTERVAL_MS: u64 = 34;
const DEFAULT_QUEUE_DROP_TARGET_RELAXED: usize = 3;
const DEFAULT_QUEUE_DROP_TARGET_TIGHT: usize = 1;
const DEFAULT_QUEUE_HISTORY_WINDOW: usize = 8;
const DEFAULT_QUEUE_HISTORY_WINDOW_MS: u64 = 500;
const HOST_NO_PENDING_HIGH_STREAK: u32 = 60;
const HOST_NO_PENDING_CRITICAL_STREAK: u32 = 120;
const HOST_PRESENT_OVERWRITE_DEGRADED_RATIO: f64 = 0.05;
const HOST_PRESENT_OVERWRITE_CRITICAL_RATIO: f64 = 0.12;

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
            FramePacingDecision::submit_now(true)
        } else if now > deadline + self.catch_up_threshold {
            FramePacingDecision::drop(true)
        } else if now >= deadline {
            FramePacingDecision::submit_now(false)
        } else {
            let sleep_time = deadline.duration_since(now);
            if sleep_time <= self.long_sleep_guard {
                FramePacingDecision::sleep(sleep_time, false)
            } else {
                // target playout 异常偏大时，优先快速追帧，不长时间阻塞线程。
                FramePacingDecision::submit_now(false)
            }
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct HostPacingPressure {
    pub cadence_phase: HostCadencePhaseHint,
    pub no_pending_pressure_level: Option<String>,
    pub no_pending_streak: u32,
    pub present_overwrite_count_total: u64,
    pub present_submit_count_total: u64,
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

impl HostPacingPressure {
    pub(crate) fn present_overwrite_ratio(&self) -> Option<f64> {
        if self.present_submit_count_total == 0 {
            return None;
        }
        Some(self.present_overwrite_count_total as f64 / self.present_submit_count_total as f64)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct QueueHistoryConfig {
    pub max_history_len: usize,
    pub max_history_age_ms: u64,
    pub relaxed_drop_target: usize,
    pub tight_drop_target: usize,
}

impl Default for QueueHistoryConfig {
    fn default() -> Self {
        Self {
            max_history_len: DEFAULT_QUEUE_HISTORY_WINDOW,
            max_history_age_ms: DEFAULT_QUEUE_HISTORY_WINDOW_MS,
            relaxed_drop_target: DEFAULT_QUEUE_DROP_TARGET_RELAXED,
            tight_drop_target: DEFAULT_QUEUE_DROP_TARGET_TIGHT,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct QueuePressureDecision {
    pub drop_target: usize,
    pub aggressive: bool,
}

pub(crate) struct QueueHistoryController {
    config: QueueHistoryConfig,
    history: VecDeque<(Instant, usize)>,
}

impl QueueHistoryController {
    pub(crate) fn new(config: QueueHistoryConfig) -> Self {
        Self {
            config,
            history: VecDeque::with_capacity(config.max_history_len),
        }
    }

    pub(crate) fn record_depth(&mut self, queue_depth: usize) {
        self.history.push_back((Instant::now(), queue_depth));
        while self.history.len() > self.config.max_history_len {
            self.history.pop_front();
        }
    }

    pub(crate) fn decide_drop_target(
        &self,
        pressure: &HostPacingPressure,
    ) -> QueuePressureDecision {
        let now = Instant::now();
        let history_window = Duration::from_millis(self.config.max_history_age_ms.max(1));
        let recent_history: Vec<usize> = self
            .history
            .iter()
            .filter(|(observed_at, _)| {
                now.saturating_duration_since(*observed_at) <= history_window
            })
            .map(|(_, depth)| *depth)
            .collect();
        let overwrite_ratio = pressure.present_overwrite_ratio().unwrap_or(0.0);
        let pressure_level = pressure.no_pending_pressure_level.as_deref();
        let phase_priming = matches!(pressure.cadence_phase, HostCadencePhaseHint::Priming);
        let phase_starved = matches!(pressure.cadence_phase, HostCadencePhaseHint::Starved);
        let host_critical = matches!(pressure_level, Some("critical"))
            && pressure.no_pending_streak >= HOST_NO_PENDING_CRITICAL_STREAK;
        let host_degraded = matches!(pressure_level, Some("high" | "critical"))
            && pressure.no_pending_streak >= HOST_NO_PENDING_HIGH_STREAK;
        let overwrite_critical = overwrite_ratio >= HOST_PRESENT_OVERWRITE_CRITICAL_RATIO;
        let overwrite_degraded = overwrite_ratio >= HOST_PRESENT_OVERWRITE_DEGRADED_RATIO;
        let cadence_lag_ratio = pressure
            .display_fps
            .zip(pressure.present_fps)
            .and_then(|(display_fps, present_fps)| {
                if display_fps <= 0.0 {
                    return None;
                }
                Some(((display_fps - present_fps).max(0.0) / display_fps).clamp(0.0, 1.0))
            })
            .unwrap_or(0.0);
        let cadence_degraded = !phase_priming && cadence_lag_ratio >= 0.25;
        let cadence_critical = !phase_priming && cadence_lag_ratio >= 0.55;

        let sustained_backlog =
            !recent_history.is_empty() && !recent_history.iter().any(|depth| *depth <= 1);
        let aggressive = phase_starved
            || host_critical
            || overwrite_critical
            || cadence_critical
            || (host_degraded && sustained_backlog);
        let should_tighten =
            aggressive || overwrite_degraded || sustained_backlog || cadence_degraded;

        QueuePressureDecision {
            drop_target: if should_tighten {
                self.config.tight_drop_target
            } else {
                self.config.relaxed_drop_target
            },
            aggressive,
        }
    }

    #[cfg(test)]
    fn from_history(config: QueueHistoryConfig, history: &[usize]) -> Self {
        let mut controller = Self::new(config);
        let now = Instant::now();
        let step_ms = (config.max_history_age_ms.max(1) / (history.len().max(1) as u64 + 1)).max(1);
        for (index, depth) in history.iter().enumerate() {
            let offset_ms = ((history.len().saturating_sub(index)) as u64).saturating_mul(step_ms);
            controller.history.push_back((
                now.checked_sub(Duration::from_millis(offset_ms))
                    .unwrap_or(now),
                *depth,
            ));
        }
        controller
    }
}

#[cfg(test)]
mod tests {
    use super::{
        FramePacingAction, FramePacingPolicy, HostCadencePhaseHint, HostPacingPressure,
        QueueHistoryConfig, QueueHistoryController,
    };
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
    fn pacing_sleeps_for_short_early_gap() {
        let policy = FramePacingPolicy::new(16);
        let now = Instant::now();
        let deadline = now + Duration::from_millis(10);
        let decision = policy.decide(now, deadline, false, None);
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
        let decision = policy.decide(now, deadline, true, None);
        assert_eq!(decision.action, FramePacingAction::SubmitNow);
        assert!(decision.exit_catch_up_mode);
    }

    #[test]
    fn pacing_uses_zero_sleep_guard_override_to_submit_now_for_short_gap() {
        let policy = FramePacingPolicy::with_dynamic_budget(16, None, Some(0), None, None);
        let now = Instant::now();
        let deadline = now + Duration::from_millis(10);
        let decision = policy.decide(now, deadline, false, None);
        assert_eq!(decision.action, FramePacingAction::SubmitNow);
    }

    #[test]
    fn pacing_submits_due_frame_without_host_release_gate() {
        let policy = FramePacingPolicy::new(16);
        let now = Instant::now();
        let deadline = now - Duration::from_millis(1);
        let decision = policy.decide(now, deadline, false, Some(Duration::from_millis(5)));
        assert_eq!(decision.action, FramePacingAction::SubmitNow);
    }

    #[test]
    fn pacing_keeps_deadline_sleep_without_host_release_gate_override() {
        let policy = FramePacingPolicy::new(16);
        let now = Instant::now();
        let deadline = now + Duration::from_millis(10);
        let decision = policy.decide(now, deadline, false, Some(Duration::from_millis(4)));
        assert_eq!(
            decision.action,
            FramePacingAction::Sleep(Duration::from_millis(10))
        );
    }

    #[test]
    fn queue_history_tolerates_short_burst_when_recently_recovered() {
        let controller =
            QueueHistoryController::from_history(QueueHistoryConfig::default(), &[1, 2, 3, 2, 1]);
        let decision = controller.decide_drop_target(&HostPacingPressure::default());
        assert_eq!(decision.drop_target, 3);
        assert!(!decision.aggressive);
    }

    #[test]
    fn queue_history_tightens_when_backlog_is_sustained() {
        let controller =
            QueueHistoryController::from_history(QueueHistoryConfig::default(), &[3, 3, 2, 2, 3]);
        let decision = controller.decide_drop_target(&HostPacingPressure::default());
        assert_eq!(decision.drop_target, 1);
    }

    #[test]
    fn queue_history_uses_more_aggressive_drop_under_host_no_pending_pressure() {
        let pressure = HostPacingPressure {
            cadence_phase: HostCadencePhaseHint::Unknown,
            no_pending_pressure_level: Some("critical".to_string()),
            no_pending_streak: 180,
            present_overwrite_count_total: 24,
            present_submit_count_total: 100,
            present_fps: Some(60.0),
            display_fps: Some(60.0),
        };
        let controller =
            QueueHistoryController::from_history(QueueHistoryConfig::default(), &[2, 2, 2, 2, 2]);
        let decision = controller.decide_drop_target(&pressure);
        assert_eq!(decision.drop_target, 1);
        assert!(decision.aggressive);
    }

    #[test]
    fn queue_history_tolerates_mild_present_cadence_lag_when_backlog_is_short() {
        let pressure = HostPacingPressure {
            cadence_phase: HostCadencePhaseHint::Unknown,
            no_pending_pressure_level: Some("normal".to_string()),
            no_pending_streak: 4,
            present_overwrite_count_total: 1,
            present_submit_count_total: 200,
            present_fps: Some(48.0),
            display_fps: Some(60.0),
        };
        let controller =
            QueueHistoryController::from_history(QueueHistoryConfig::default(), &[1, 2, 1, 2, 1]);
        let decision = controller.decide_drop_target(&pressure);
        assert_eq!(decision.drop_target, 3);
        assert!(!decision.aggressive);
    }

    #[test]
    fn queue_history_tightens_when_present_cadence_lag_is_severe() {
        let pressure = HostPacingPressure {
            cadence_phase: HostCadencePhaseHint::Unknown,
            no_pending_pressure_level: Some("normal".to_string()),
            no_pending_streak: 8,
            present_overwrite_count_total: 2,
            present_submit_count_total: 200,
            present_fps: Some(20.0),
            display_fps: Some(60.0),
        };
        let controller =
            QueueHistoryController::from_history(QueueHistoryConfig::default(), &[2, 2, 2, 2, 2]);
        let decision = controller.decide_drop_target(&pressure);
        assert_eq!(decision.drop_target, 1);
        assert!(decision.aggressive);
    }

    #[test]
    fn queue_history_keeps_relaxed_target_during_priming_without_real_backlog_pressure() {
        let pressure = HostPacingPressure {
            cadence_phase: HostCadencePhaseHint::Priming,
            no_pending_pressure_level: Some("normal".to_string()),
            no_pending_streak: 0,
            present_overwrite_count_total: 0,
            present_submit_count_total: 1,
            present_fps: Some(10.0),
            display_fps: Some(60.0),
        };
        let controller =
            QueueHistoryController::from_history(QueueHistoryConfig::default(), &[1, 2, 1, 2, 1]);
        let decision = controller.decide_drop_target(&pressure);
        assert_eq!(decision.drop_target, 3);
        assert!(!decision.aggressive);
    }

    #[test]
    fn queue_history_tightens_aggressively_when_host_phase_is_starved() {
        let pressure = HostPacingPressure {
            cadence_phase: HostCadencePhaseHint::Starved,
            no_pending_pressure_level: Some("normal".to_string()),
            no_pending_streak: 12,
            present_overwrite_count_total: 0,
            present_submit_count_total: 20,
            present_fps: Some(58.0),
            display_fps: Some(60.0),
        };
        let controller =
            QueueHistoryController::from_history(QueueHistoryConfig::default(), &[2, 2, 1, 2, 2]);
        let decision = controller.decide_drop_target(&pressure);
        assert_eq!(decision.drop_target, 1);
        assert!(decision.aggressive);
    }
}
