use std::collections::VecDeque;

use xbxengine::{XbxEngineHostVideoFrameDropEvent, XbxEngineRenderFrame};

const HOST_RENDER_FPS_WINDOW_MS: f64 = 1_000.0;
const HOST_RENDER_MIN_FRAME_AGE_MS: f64 = 24.0;
const HOST_RENDER_MAX_FRAME_AGE_MS: f64 = 75.0;
const HOST_RENDER_FRAME_AGE_MULTIPLIER: f64 = 2.25;
const HOST_RENDER_RECOVERY_MIN_FRAME_AGE_MS: f64 = 48.0;
const HOST_RENDER_RECOVERY_MAX_FRAME_AGE_MS: f64 = 180.0;
const HOST_RENDER_RECOVERY_STREAK_THRESHOLD: u32 = 8;
const HOST_FRAME_DROP_BACKLOG_LIMIT: usize = 32;
const HOST_SUBMIT_GAP_WARN_MS: f64 = 100.0;
const SCHEDULED_FRAME_QUEUE_CAPACITY: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostCadencePhase {
    Idle,
    Priming,
    Steady,
    Starved,
}

impl HostCadencePhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Priming => "priming",
            Self::Steady => "steady",
            Self::Starved => "starved",
        }
    }
}

impl Default for HostCadencePhase {
    fn default() -> Self {
        Self::Idle
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct HostFrameDropBacklog {
    pending: VecDeque<XbxEngineHostVideoFrameDropEvent>,
}

impl Default for HostFrameDropBacklog {
    fn default() -> Self {
        Self {
            pending: VecDeque::new(),
        }
    }
}

impl HostFrameDropBacklog {
    pub fn record_stale_frame_drop(
        &mut self,
        frame: &XbxEngineRenderFrame,
        observed_at_ms: f64,
        detail: &str,
        queue_depth: usize,
    ) {
        self.pending.push_back(XbxEngineHostVideoFrameDropEvent {
            stage: Some("present".to_string()),
            action: Some("drop".to_string()),
            detail: Some(detail.to_string()),
            frame_rtp_timestamp: frame.rtp_timestamp,
            frame_seq: Some(frame.frame_seq),
            frame_recovery_disposition: frame.frame_recovery_disposition.clone(),
            frame_unrecoverable_reason: frame.frame_unrecoverable_reason.clone(),
            observed_at_ms,
            width: frame.width,
            height: frame.height,
            is_keyframe: frame.is_keyframe,
            queue_depth,
        });
        while self.pending.len() > HOST_FRAME_DROP_BACKLOG_LIMIT {
            self.pending.pop_front();
        }
    }

    pub fn take_all(&mut self) -> Vec<XbxEngineHostVideoFrameDropEvent> {
        self.pending.drain(..).collect()
    }

    pub fn reset(&mut self) {
        self.pending.clear();
    }
}

#[derive(Default)]
pub struct HostCadenceTelemetry {
    pub latest_present_time_ms: Option<f64>,
    pub latest_submit_time_ms: Option<f64>,
    display_tick_epoch: u64,
    present_epoch: u64,
    cadence_phase: HostCadencePhase,
    recent_present_times_ms: VecDeque<f64>,
    recent_display_tick_times_ms: VecDeque<f64>,
    pub present_enqueue_count_total: u64,
    pub present_drop_count_total: u64,
    pub present_overwrite_count_total: u64,
    pub no_pending_take_count_total: u64,
    pub no_pending_streak: u32,
    pub no_pending_max_streak: u32,
    pending_frame_drops: HostFrameDropBacklog,
}

impl HostCadenceTelemetry {
    pub fn record_display_tick(&mut self, now_ms: f64) {
        self.recent_display_tick_times_ms.push_back(now_ms);
        self.trim_display_ticks(now_ms);
        self.display_tick_epoch = self.display_tick_epoch.saturating_add(1);
        if matches!(self.cadence_phase, HostCadencePhase::Idle) {
            self.cadence_phase = HostCadencePhase::Priming;
        }
    }

    pub fn record_present(&mut self, now_ms: f64) {
        self.latest_present_time_ms = Some(now_ms);
        self.recent_present_times_ms.push_back(now_ms);
        self.trim_recent(now_ms);
        self.present_epoch = self.present_epoch.saturating_add(1);
        self.cadence_phase = HostCadencePhase::Steady;
    }

    pub fn record_submit(&mut self, now_ms: f64) -> Option<f64> {
        let submit_gap_ms = self
            .latest_submit_time_ms
            .map(|previous| (now_ms - previous).max(0.0));
        self.latest_submit_time_ms = Some(now_ms);
        submit_gap_ms
    }

    pub fn record_drop(&mut self) {
        self.present_drop_count_total = self.present_drop_count_total.saturating_add(1);
    }

    pub fn record_stale_frame_drop(
        &mut self,
        frame: &XbxEngineRenderFrame,
        observed_at_ms: f64,
        detail: &str,
        queue_depth: usize,
    ) {
        self.record_drop();
        self.pending_frame_drops.record_stale_frame_drop(
            frame,
            observed_at_ms,
            detail,
            queue_depth,
        );
    }

    pub fn record_overwrite(&mut self) {
        self.present_overwrite_count_total = self.present_overwrite_count_total.saturating_add(1);
    }

    pub fn record_no_pending_take(&mut self) {
        self.no_pending_take_count_total = self.no_pending_take_count_total.saturating_add(1);
        self.no_pending_streak = self.no_pending_streak.saturating_add(1);
        self.no_pending_max_streak = self.no_pending_max_streak.max(self.no_pending_streak);
        if self.present_epoch > 0 {
            self.cadence_phase = HostCadencePhase::Starved;
        }
    }

    pub fn clear_no_pending_streak(&mut self) {
        self.no_pending_streak = 0;
        self.cadence_phase = if self.present_epoch > 0 {
            HostCadencePhase::Steady
        } else if self.display_tick_epoch > 0 {
            HostCadencePhase::Priming
        } else {
            HostCadencePhase::Idle
        };
    }

    pub fn take_pending_frame_drops(&mut self) -> Vec<XbxEngineHostVideoFrameDropEvent> {
        self.pending_frame_drops.take_all()
    }

    pub fn present_fps(&self) -> f64 {
        calculate_recent_fps(&self.recent_present_times_ms)
    }

    pub fn display_interval_ms(&self) -> Option<f64> {
        calculate_recent_interval_ms(&self.recent_display_tick_times_ms)
    }

    pub fn frame_age_budget_ms(&self) -> f64 {
        self.display_interval_ms()
            .map(|interval_ms| {
                (interval_ms * HOST_RENDER_FRAME_AGE_MULTIPLIER)
                    .clamp(HOST_RENDER_MIN_FRAME_AGE_MS, HOST_RENDER_MAX_FRAME_AGE_MS)
            })
            .unwrap_or(HOST_RENDER_MAX_FRAME_AGE_MS)
    }

    pub fn stale_frame_age_budget_ms(&self) -> f64 {
        let base_budget_ms = self.frame_age_budget_ms();
        if self.present_epoch == 0 {
            return base_budget_ms.max(HOST_RENDER_RECOVERY_MIN_FRAME_AGE_MS);
        }
        if matches!(self.cadence_phase, HostCadencePhase::Starved)
            || self.no_pending_streak >= HOST_RENDER_RECOVERY_STREAK_THRESHOLD
        {
            return (base_budget_ms * 8.0).clamp(
                HOST_RENDER_RECOVERY_MIN_FRAME_AGE_MS,
                HOST_RENDER_RECOVERY_MAX_FRAME_AGE_MS,
            );
        }
        base_budget_ms
    }

    pub fn should_warn_submit_gap(&self, submit_gap_ms: f64) -> bool {
        submit_gap_ms >= HOST_SUBMIT_GAP_WARN_MS
    }

    pub fn display_tick_epoch(&self) -> u64 {
        self.display_tick_epoch
    }

    pub fn present_epoch(&self) -> u64 {
        self.present_epoch
    }

    pub fn cadence_phase(&self) -> HostCadencePhase {
        self.cadence_phase
    }

    pub fn reset_frame_slot(&mut self) {
        self.latest_present_time_ms = None;
        self.latest_submit_time_ms = None;
        self.display_tick_epoch = 0;
        self.present_epoch = 0;
        self.cadence_phase = HostCadencePhase::Idle;
        self.recent_present_times_ms.clear();
        self.recent_display_tick_times_ms.clear();
        // 会话 detach / reattach 后需要重新统计宿主 present 指标，
        // 否则新会话会继承上一轮 submit/drop/overwrite 计数，诊断会失真。
        self.present_enqueue_count_total = 0;
        self.present_drop_count_total = 0;
        self.present_overwrite_count_total = 0;
        self.no_pending_take_count_total = 0;
        self.no_pending_streak = 0;
        self.no_pending_max_streak = 0;
        self.pending_frame_drops.reset();
    }

    fn trim_recent(&mut self, now_ms: f64) {
        while self
            .recent_present_times_ms
            .front()
            .is_some_and(|ts_ms| now_ms - *ts_ms > HOST_RENDER_FPS_WINDOW_MS)
        {
            self.recent_present_times_ms.pop_front();
        }
    }

    fn trim_display_ticks(&mut self, now_ms: f64) {
        while self
            .recent_display_tick_times_ms
            .front()
            .is_some_and(|ts_ms| now_ms - *ts_ms > HOST_RENDER_FPS_WINDOW_MS)
        {
            self.recent_display_tick_times_ms.pop_front();
        }
    }
}

#[derive(Default)]
pub struct ScheduledFrameSlot {
    displayed_frame: Option<XbxEngineRenderFrame>,
    pending_frames: VecDeque<XbxEngineRenderFrame>,
    last_presented_frame_seq: Option<u64>,
    pub render_loop_started: bool,
}

#[derive(Debug)]
pub enum ScheduledFrameSubmitOutcome {
    Accepted {
        frame_seq: u64,
        overwrote_pending: bool,
        replaced_frame_seq: Option<u64>,
        frame_age_ms: f64,
        frame_age_budget_ms: f64,
    },
    DroppedStale {
        frame_seq: u64,
        frame_age_ms: f64,
        frame_age_budget_ms: f64,
    },
    RejectedAlreadyPresented {
        frame_seq: u64,
        last_presented_frame_seq: u64,
    },
}

#[derive(Debug)]
pub enum ScheduledFrameTakeOutcome {
    Ready(XbxEngineRenderFrame),
    RetainedDisplayedFrame,
    NoPendingFrame,
    DroppedStale {
        frame: XbxEngineRenderFrame,
        frame_age_ms: f64,
        frame_age_budget_ms: f64,
    },
}

impl ScheduledFrameSlot {
    pub fn submit_frame(
        &mut self,
        frame: &XbxEngineRenderFrame,
        now_ms: f64,
        telemetry: &mut HostCadenceTelemetry,
    ) -> ScheduledFrameSubmitOutcome {
        let frame_age_budget_ms = telemetry.stale_frame_age_budget_ms();
        let frame_age_ms = (now_ms - frame.rendered_at_ms).max(0.0);
        if frame_age_ms > frame_age_budget_ms {
            telemetry.record_stale_frame_drop(frame, now_ms, "scheduledFrameStale", 1);
            return ScheduledFrameSubmitOutcome::DroppedStale {
                frame_seq: frame.frame_seq,
                frame_age_ms,
                frame_age_budget_ms,
            };
        }
        if self
            .last_presented_frame_seq
            .is_some_and(|frame_seq| frame.frame_seq <= frame_seq)
        {
            return ScheduledFrameSubmitOutcome::RejectedAlreadyPresented {
                frame_seq: frame.frame_seq,
                last_presented_frame_seq: self.last_presented_frame_seq.unwrap_or_default(),
            };
        }
        let pending_back_seq = self.pending_frames.back().map(|pending| pending.frame_seq);
        if pending_back_seq.is_some_and(|frame_seq| frame.frame_seq <= frame_seq) {
            return ScheduledFrameSubmitOutcome::RejectedAlreadyPresented {
                frame_seq: frame.frame_seq,
                last_presented_frame_seq: pending_back_seq.unwrap_or_default(),
            };
        }
        let mut replaced_frame_seq = None;
        let mut overwrote_pending = false;
        if self.pending_frames.len() >= SCHEDULED_FRAME_QUEUE_CAPACITY {
            if let Some(replaced) = self.pending_frames.pop_front() {
                replaced_frame_seq = Some(replaced.frame_seq);
                overwrote_pending = true;
                telemetry.record_overwrite();
            }
        }
        self.pending_frames.push_back(frame.clone());
        ScheduledFrameSubmitOutcome::Accepted {
            frame_seq: frame.frame_seq,
            overwrote_pending,
            replaced_frame_seq,
            frame_age_ms,
            frame_age_budget_ms,
        }
    }

    pub fn take_ready_frame(
        &mut self,
        now_ms: f64,
        telemetry: &mut HostCadenceTelemetry,
    ) -> ScheduledFrameTakeOutcome {
        loop {
            let Some(frame) = self.pending_frames.pop_front() else {
                break;
            };
            if self
                .last_presented_frame_seq
                .is_some_and(|frame_seq| frame.frame_seq <= frame_seq)
            {
                continue;
            }
            let frame_age_budget_ms = telemetry.stale_frame_age_budget_ms();
            let frame_age_ms = (now_ms - frame.rendered_at_ms).max(0.0);
            if frame_age_ms > frame_age_budget_ms {
                telemetry.record_stale_frame_drop(
                    &frame,
                    now_ms,
                    "scheduledFrameStale",
                    self.queue_depth(),
                );
                if self.pending_frames.is_empty() && self.displayed_frame.is_none() {
                    return ScheduledFrameTakeOutcome::DroppedStale {
                        frame,
                        frame_age_ms,
                        frame_age_budget_ms,
                    };
                }
                continue;
            }
            self.last_presented_frame_seq = Some(frame.frame_seq);
            self.displayed_frame = Some(frame.clone());
            telemetry.clear_no_pending_streak();
            return ScheduledFrameTakeOutcome::Ready(frame);
        }
        if self.displayed_frame.is_some() {
            telemetry.clear_no_pending_streak();
            return ScheduledFrameTakeOutcome::RetainedDisplayedFrame;
        }
        telemetry.record_no_pending_take();
        ScheduledFrameTakeOutcome::NoPendingFrame
    }

    pub fn reset(&mut self) {
        self.displayed_frame = None;
        self.pending_frames.clear();
        self.last_presented_frame_seq = None;
        self.render_loop_started = false;
    }

    pub fn begin_media_epoch(&mut self) {
        self.displayed_frame = None;
        self.pending_frames.clear();
        self.last_presented_frame_seq = None;
        // 媒体 epoch 刷新时只清去重态，不动 render_loop_started，
        // 否则会把仍在运行的 display link / fallback loop 误判成需要重启。
    }

    fn queue_depth(&self) -> usize {
        self.pending_frames.len() + usize::from(self.displayed_frame.is_some())
    }
}

#[cfg(test)]
#[path = "scheduling.test.rs"]
mod tests;

fn calculate_recent_fps(recent_times_ms: &VecDeque<f64>) -> f64 {
    if recent_times_ms.len() < 2 {
        return recent_times_ms.len() as f64;
    }
    let Some(first) = recent_times_ms.front() else {
        return 0.0;
    };
    let Some(last) = recent_times_ms.back() else {
        return 0.0;
    };
    let elapsed_ms = (last - first).max(1.0);
    ((recent_times_ms.len() - 1) as f64) * 1000.0 / elapsed_ms
}

fn calculate_recent_interval_ms(recent_times_ms: &VecDeque<f64>) -> Option<f64> {
    if recent_times_ms.len() < 2 {
        return None;
    }
    let first = *recent_times_ms.front()?;
    let last = *recent_times_ms.back()?;
    let elapsed_ms = (last - first).max(1.0);
    Some(elapsed_ms / (recent_times_ms.len() - 1) as f64)
}
