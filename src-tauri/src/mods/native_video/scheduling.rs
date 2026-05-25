use std::collections::VecDeque;

use xbxengine::{XbxEngineHostVideoFrameDropEvent, XbxEngineRenderFrame};

const HOST_RENDER_FPS_WINDOW_MS: f64 = 1_000.0;
const HOST_RENDER_MIN_FRAME_AGE_MS: f64 = 24.0;
const HOST_RENDER_MAX_FRAME_AGE_MS: f64 = 75.0;
const HOST_RENDER_FRAME_AGE_MULTIPLIER: f64 = 2.25;
const HOST_RENDER_RECOVERY_MIN_FRAME_AGE_MS: f64 = 48.0;
const HOST_RENDER_RECOVERY_MAX_FRAME_AGE_MS: f64 = 180.0;
const HOST_RENDER_RECOVERY_STREAK_THRESHOLD: u32 = 8;
const HOST_RENDER_RECOVERY_KEYFRAME_MIN_FRAME_AGE_MS: f64 = 48.0;
const HOST_FRAME_DROP_BACKLOG_LIMIT: usize = 32;
const HOST_SUBMIT_GAP_WARN_MS: f64 = 100.0;
/// submit 侧额外宽限：decode→render 排队不应在进 host mailbox 前被误杀。
const HOST_MAILBOX_SUBMIT_PIPELINE_SLACK_MS: f64 = 96.0;
const HOST_MAILBOX_ADAPTIVE_BUDGET_MIN_INTERVAL_MS: f64 = 20.0;
const HOST_MAILBOX_ADAPTIVE_BUDGET_MAX_INTERVAL_MS: f64 = 90.0;
fn format_optional_seq(value: Option<u64>) -> String {
    value
        .map(|seq| seq.to_string())
        .unwrap_or_else(|| "-".to_string())
}

fn format_frame_seq_list(frame_seqs: &[u64]) -> String {
    if frame_seqs.is_empty() {
        "-".to_string()
    } else {
        frame_seqs
            .iter()
            .map(u64::to_string)
            .collect::<Vec<_>>()
            .join(",")
    }
}

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
    recent_submit_times_ms: VecDeque<f64>,
    recent_display_tick_times_ms: VecDeque<f64>,
    pub present_enqueue_count_total: u64,
    pub present_drop_count_total: u64,
    pub present_overwrite_count_total: u64,
    pub no_pending_take_count_total: u64,
    pub no_pending_streak: u32,
    pub no_pending_max_streak: u32,
    pending_frame_drops: HostFrameDropBacklog,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HostCadenceTelemetryDiagnostics {
    pub latest_present_time_ms: Option<f64>,
    pub latest_submit_time_ms: Option<f64>,
    pub display_tick_epoch: u64,
    pub present_epoch: u64,
    pub cadence_phase: HostCadencePhase,
    pub no_pending_take_count_total: u64,
    pub no_pending_streak: u32,
    pub no_pending_max_streak: u32,
    pub present_enqueue_count_total: u64,
    pub present_drop_count_total: u64,
    pub present_overwrite_count_total: u64,
}

impl HostCadenceTelemetry {
    pub fn record_display_tick(&mut self, now_ms: f64) {
        self.recent_display_tick_times_ms.push_back(now_ms);
        self.trim_display_ticks(now_ms);
        self.display_tick_epoch = self.display_tick_epoch.saturating_add(1);
        if matches!(self.cadence_phase, HostCadencePhase::Idle) {
            self.cadence_phase = HostCadencePhase::Priming;
        }
        self.log_host_flow("display_tick", None, None, None, None);
    }

    pub fn record_present(&mut self, now_ms: f64) {
        self.latest_present_time_ms = Some(now_ms);
        self.recent_present_times_ms.push_back(now_ms);
        self.trim_recent(now_ms);
        self.present_epoch = self.present_epoch.saturating_add(1);
        self.cadence_phase = HostCadencePhase::Steady;
        self.log_host_flow("present", None, None, None, None);
    }

    pub fn record_submit(&mut self, now_ms: f64) -> Option<f64> {
        let submit_gap_ms = self
            .latest_submit_time_ms
            .map(|previous| (now_ms - previous).max(0.0));
        self.latest_submit_time_ms = Some(now_ms);
        self.recent_submit_times_ms.push_back(now_ms);
        self.trim_submit_times(now_ms);
        self.log_host_flow("submit_telemetry", None, None, None, submit_gap_ms);
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

    /// 已有 displayed 帧、等待下一帧 pending：持帧不是断供，不应累计 no_pending 或进入 starved。
    pub fn record_display_hold(&mut self) {
        self.no_pending_streak = 0;
        if self.present_epoch > 0 {
            self.cadence_phase = HostCadencePhase::Steady;
        } else if self.display_tick_epoch > 0 {
            self.cadence_phase = HostCadencePhase::Priming;
        }
    }

    /// 刷新 present 时钟但不增加 present_epoch（持帧重绘 / 同帧维持可见）。
    /// 不计入 `present_fps`：否则 display tick 会把呈现帧率抬到 120+ 而误导性能面板。
    pub fn record_present_refresh(&mut self, now_ms: f64) {
        self.latest_present_time_ms = Some(now_ms);
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
        if self.recent_present_times_ms.len() < PRESENT_FPS_MIN_SAMPLES {
            return 0.0;
        }
        let Some(first) = self.recent_present_times_ms.front().copied() else {
            return 0.0;
        };
        let Some(last) = self.recent_present_times_ms.back().copied() else {
            return 0.0;
        };
        if (last - first) < PRESENT_FPS_MIN_WINDOW_MS {
            return 0.0;
        }
        calculate_recent_fps(&self.recent_present_times_ms)
    }

    pub fn display_interval_ms(&self) -> Option<f64> {
        calculate_recent_interval_ms(&self.recent_display_tick_times_ms)
    }

    pub fn submit_interval_ms(&self) -> Option<f64> {
        calculate_recent_interval_ms(&self.recent_submit_times_ms)
    }

    pub fn effective_frame_interval_ms(&self) -> Option<f64> {
        match (self.display_interval_ms(), self.submit_interval_ms()) {
            (Some(display_interval_ms), Some(submit_interval_ms)) => {
                Some(display_interval_ms.max(submit_interval_ms))
            }
            (Some(display_interval_ms), None) => Some(display_interval_ms),
            (None, Some(submit_interval_ms)) => Some(submit_interval_ms),
            (None, None) => None,
        }
    }

    fn adaptive_interval_budget_ms(interval_ms: f64) -> f64 {
        (interval_ms * HOST_RENDER_FRAME_AGE_MULTIPLIER).clamp(
            HOST_MAILBOX_ADAPTIVE_BUDGET_MIN_INTERVAL_MS,
            HOST_MAILBOX_ADAPTIVE_BUDGET_MAX_INTERVAL_MS,
        )
    }

    fn submit_driven_frame_age_budget_ms(&self) -> Option<f64> {
        self.submit_interval_ms()
            .map(Self::adaptive_interval_budget_ms)
    }

    pub fn frame_age_budget_ms(&self) -> f64 {
        let effective_budget = self
            .effective_frame_interval_ms()
            .map(Self::adaptive_interval_budget_ms);
        match (effective_budget, self.submit_driven_frame_age_budget_ms()) {
            (Some(left), Some(right)) => left.max(right),
            (Some(value), None) | (None, Some(value)) => value,
            (None, None) => HOST_RENDER_MAX_FRAME_AGE_MS,
        }
    }

    pub fn host_mailbox_submit_stale_budget_ms(&self, frame: &XbxEngineRenderFrame) -> f64 {
        self.stale_frame_age_budget_for_frame(frame) + HOST_MAILBOX_SUBMIT_PIPELINE_SLACK_MS
    }

    pub fn host_mailbox_take_stale_budget_ms(&self, frame: &XbxEngineRenderFrame) -> f64 {
        let base = self.stale_frame_age_budget_for_frame(frame);
        self.submit_driven_frame_age_budget_ms()
            .map(|submit_budget| base.max(submit_budget))
            .unwrap_or(base)
    }

    fn holding_displayed_awaiting_next_frame(&self) -> bool {
        self.present_epoch > 0
            && matches!(self.cadence_phase, HostCadencePhase::Steady)
            && self.no_pending_streak == 0
    }

    fn recovery_frame_age_budget_ms(&self, base_budget_ms: f64) -> f64 {
        (base_budget_ms * 8.0).clamp(
            HOST_RENDER_RECOVERY_MIN_FRAME_AGE_MS,
            HOST_RENDER_RECOVERY_MAX_FRAME_AGE_MS,
        )
    }

    pub fn stale_frame_age_budget_ms(&self) -> f64 {
        let base_budget_ms = self.frame_age_budget_ms();
        if self.present_epoch == 0 {
            return base_budget_ms.max(HOST_RENDER_RECOVERY_MIN_FRAME_AGE_MS);
        }
        if matches!(self.cadence_phase, HostCadencePhase::Starved)
            || self.no_pending_streak >= HOST_RENDER_RECOVERY_STREAK_THRESHOLD
            || self.holding_displayed_awaiting_next_frame()
        {
            return self.recovery_frame_age_budget_ms(base_budget_ms);
        }
        base_budget_ms
    }

    pub fn stale_frame_age_budget_for_frame(&self, frame: &XbxEngineRenderFrame) -> f64 {
        let base_budget_ms = self.stale_frame_age_budget_ms();
        // 恢复期关键帧经常发生在 host stall 之后的首个可恢复窗口。
        // 这类帧只要已经进入 pending，不应再被 24ms 的稳态阈值立刻打掉。
        if frame.is_keyframe && frame_has_anchor_protection(frame) {
            return base_budget_ms.max(HOST_RENDER_RECOVERY_KEYFRAME_MIN_FRAME_AGE_MS);
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
        self.recent_submit_times_ms.clear();
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

    pub fn diagnostics_snapshot(&self) -> HostCadenceTelemetryDiagnostics {
        HostCadenceTelemetryDiagnostics {
            latest_present_time_ms: self.latest_present_time_ms,
            latest_submit_time_ms: self.latest_submit_time_ms,
            display_tick_epoch: self.display_tick_epoch,
            present_epoch: self.present_epoch,
            cadence_phase: self.cadence_phase,
            no_pending_take_count_total: self.no_pending_take_count_total,
            no_pending_streak: self.no_pending_streak,
            no_pending_max_streak: self.no_pending_max_streak,
            present_enqueue_count_total: self.present_enqueue_count_total,
            present_drop_count_total: self.present_drop_count_total,
            present_overwrite_count_total: self.present_overwrite_count_total,
        }
    }

    fn log_host_flow(
        &self,
        _event: &str,
        _frame_seq: Option<u64>,
        slot_diag: Option<&ScheduledFrameSlotDiagnostics>,
        replaced_frame_seq: Option<u64>,
        submit_gap_ms: Option<f64>,
    ) {
        let _displayed_frame_seq = slot_diag.and_then(|diag| diag.displayed_frame_seq);
        let _pending_frame_seqs = slot_diag
            .map(|diag| format_frame_seq_list(&diag.pending_frame_seqs))
            .unwrap_or_else(|| "-".to_string());
        let _queue_depth = slot_diag
            .map(|diag| diag.queue_depth.to_string())
            .unwrap_or_else(|| "-".to_string());
        let _pending_queue_depth = slot_diag
            .map(|diag| diag.pending_queue_depth.to_string())
            .unwrap_or_else(|| "-".to_string());
        let _last_presented_frame_seq = slot_diag.and_then(|diag| diag.last_presented_frame_seq);
        let _replaced_frame_seq = format_optional_seq(replaced_frame_seq);
        let _submit_gap_ms = submit_gap_ms
            .map(|gap| format!("{gap:.2}"))
            .unwrap_or_else(|| "-".to_string());
        // log::info!(
        //     "[playback-flow][host] event={} frame_seq={} displayed_frame_seq={} last_presented_frame_seq={} pending_frame_seqs={} queue_depth={} pending_queue_depth={} display_tick_epoch={} present_epoch={} submit_gap_ms={} replaced_frame_seq={} no_pending_streak={} cadence_phase={}",
        //     event,
        //     format_optional_seq(frame_seq),
        //     format_optional_seq(displayed_frame_seq),
        //     format_optional_seq(last_presented_frame_seq),
        //     pending_frame_seqs,
        //     queue_depth,
        //     pending_queue_depth,
        //     self.display_tick_epoch,
        //     self.present_epoch,
        //     submit_gap_ms,
        //     replaced_frame_seq,
        //     self.no_pending_streak,
        //     self.cadence_phase.as_str(),
        // );
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

    fn trim_submit_times(&mut self, now_ms: f64) {
        while self
            .recent_submit_times_ms
            .front()
            .is_some_and(|ts_ms| now_ms - *ts_ms > HOST_RENDER_FPS_WINDOW_MS)
        {
            self.recent_submit_times_ms.pop_front();
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
    pending_frame: Option<XbxEngineRenderFrame>,
    /// pending 进入 host mailbox 的时刻；take 时用其计龄而非 render 时间戳。
    pending_accepted_at_ms: Option<f64>,
    last_presented_frame_seq: Option<u64>,
    view_epoch: u64,
    displayed_view_epoch: u64,
    pub render_loop_started: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ScheduledFrameSlotDiagnostics {
    pub displayed_frame_seq: Option<u64>,
    pub displayed_frame_rtp_timestamp: Option<u32>,
    pub displayed_frame_recovery_disposition: Option<String>,
    pub displayed_frame_rendered_at_ms: Option<f64>,
    pub pending_frame_seqs: Vec<u64>,
    pub pending_frame_rtp_timestamp: Option<u32>,
    pub last_presented_frame_seq: Option<u64>,
    pub queue_depth: usize,
    pub pending_queue_depth: usize,
    pub has_displayed_frame: bool,
    pub view_epoch: u64,
    pub displayed_view_epoch: u64,
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

impl ScheduledFrameTakeOutcome {
    pub fn mailbox_take_decision(&self) -> &'static str {
        match self {
            Self::Ready(_) => "ready",
            Self::RetainedDisplayedFrame => "retainedDisplayed",
            Self::NoPendingFrame => "noPending",
            Self::DroppedStale { .. } => "droppedStale",
        }
    }
}

impl ScheduledFrameSlot {
    fn current_media_epoch_tag(&self) -> Option<u64> {
        self.pending_frame
            .as_ref()
            .and_then(|frame| frame.recovery_epoch_tag)
            .or_else(|| {
                self.displayed_frame
                    .as_ref()
                    .and_then(|frame| frame.recovery_epoch_tag)
            })
    }

    fn current_media_owner_rtp_timestamp(&self) -> Option<u32> {
        self.pending_frame
            .as_ref()
            .and_then(|frame| frame.recovery_owner_rtp_timestamp)
            .or_else(|| {
                self.displayed_frame
                    .as_ref()
                    .and_then(|frame| frame.recovery_owner_rtp_timestamp)
            })
    }

    fn frame_is_recovery_valued(frame: &XbxEngineRenderFrame) -> bool {
        frame.is_keyframe
            || matches!(
                frame.frame_recovery_disposition.as_deref(),
                Some("repairing") | Some("rebuilding") | Some("rebuilding-supply")
            )
    }

    fn frame_opens_recovery_media_epoch(&self, frame: &XbxEngineRenderFrame) -> bool {
        let Some(last_presented) = self.last_presented_frame_seq else {
            return false;
        };
        if frame
            .recovery_epoch_tag
            .zip(self.current_media_epoch_tag())
            .is_some_and(|(incoming_epoch, current_epoch)| incoming_epoch > current_epoch)
        {
            return true;
        }
        if frame
            .recovery_owner_rtp_timestamp
            .zip(self.current_media_owner_rtp_timestamp())
            .is_some_and(|(incoming_owner, current_owner)| incoming_owner != current_owner)
            && frame_confirms_recovery_owner(frame)
        {
            return true;
        }
        Self::frame_is_recovery_valued(frame) && frame.frame_seq < last_presented
    }

    fn should_begin_new_media_epoch(&self, frame: &XbxEngineRenderFrame) -> bool {
        self.displayed_frame.is_some() && self.frame_opens_recovery_media_epoch(frame)
    }

    fn host_mailbox_pending_age_ms(
        pending_accepted_at_ms: Option<f64>,
        frame: &XbxEngineRenderFrame,
        now_ms: f64,
    ) -> f64 {
        pending_accepted_at_ms
            .map(|accepted_at_ms| (now_ms - accepted_at_ms).max(0.0))
            .unwrap_or_else(|| (now_ms - frame.rendered_at_ms).max(0.0))
    }

    fn should_discard_stale_pending_at_take(
        &self,
        pending: &XbxEngineRenderFrame,
        now_ms: f64,
        telemetry: &HostCadenceTelemetry,
    ) -> bool {
        let frame_age_budget_ms = telemetry.host_mailbox_take_stale_budget_ms(pending);
        let frame_age_ms =
            Self::host_mailbox_pending_age_ms(self.pending_accepted_at_ms, pending, now_ms);
        frame_age_ms > frame_age_budget_ms
    }

    fn should_discard_duplicate_pending_at_take(&self, pending: &XbxEngineRenderFrame) -> bool {
        self.last_presented_frame_seq.is_some_and(|last_presented| {
            pending.frame_seq <= last_presented
                && !Self::frame_is_recovery_valued(pending)
                && !frame_confirms_recovery_owner(pending)
        })
    }

    fn discard_pending_without_present(&mut self) {
        self.pending_frame = None;
        self.pending_accepted_at_ms = None;
    }

    pub fn submit_frame(
        &mut self,
        frame: &XbxEngineRenderFrame,
        now_ms: f64,
        telemetry: &mut HostCadenceTelemetry,
    ) -> ScheduledFrameSubmitOutcome {
        if self.should_begin_new_media_epoch(frame) {
            // 解码侧恢复后 frame_seq 会以较小序号重新进入；宿主仍保留旧 epoch 的 displayed frame 时，
            // 恢复关键帧需要先切 media epoch，再进入新的调度窗口。
            self.begin_media_epoch();
        }
        let frame_seq = frame.frame_seq;
        let frame_age_budget_ms = telemetry.host_mailbox_submit_stale_budget_ms(frame);
        let frame_age_ms = (now_ms - frame.rendered_at_ms).max(0.0);
        if frame_age_ms > frame_age_budget_ms {
            telemetry.record_stale_frame_drop(frame, now_ms, "scheduledFrameStale", 1);
            self.log_host_flow(
                "submit",
                Some(frame_seq),
                "DroppedStale",
                None,
                false,
                frame_age_ms,
                frame_age_budget_ms,
                telemetry,
            );
            return ScheduledFrameSubmitOutcome::DroppedStale {
                frame_seq: frame.frame_seq,
                frame_age_ms,
                frame_age_budget_ms,
            };
        }
        if self.last_presented_frame_seq.is_some_and(|last_presented| {
            frame.frame_seq <= last_presented
                && !Self::frame_is_recovery_valued(frame)
                && !frame_confirms_recovery_owner(frame)
        }) {
            self.log_host_flow(
                "submit",
                Some(frame_seq),
                "RejectedAlreadyPresented",
                None,
                false,
                frame_age_ms,
                frame_age_budget_ms,
                telemetry,
            );
            return ScheduledFrameSubmitOutcome::RejectedAlreadyPresented {
                frame_seq: frame.frame_seq,
                last_presented_frame_seq: self.last_presented_frame_seq.unwrap_or_default(),
            };
        }
        if let Some(pending) = self.pending_frame.as_ref() {
            if !incoming_frame_is_newer_than_pending(pending, frame) {
                self.log_host_flow(
                    "submit",
                    Some(frame_seq),
                    "RejectedAlreadyPresented",
                    None,
                    false,
                    frame_age_ms,
                    frame_age_budget_ms,
                    telemetry,
                );
                return ScheduledFrameSubmitOutcome::RejectedAlreadyPresented {
                    frame_seq: frame.frame_seq,
                    last_presented_frame_seq: pending.frame_seq,
                };
            }
        }
        let replaced_frame_seq = self
            .pending_frame
            .replace(frame.clone())
            .map(|frame| frame.frame_seq);
        let overwrote_pending = replaced_frame_seq.is_some();
        self.pending_accepted_at_ms = Some(now_ms);
        if overwrote_pending {
            telemetry.record_overwrite();
        }
        self.log_host_flow(
            "submit",
            Some(frame_seq),
            "Accepted",
            replaced_frame_seq,
            overwrote_pending,
            frame_age_ms,
            frame_age_budget_ms,
            telemetry,
        );
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
        if let Some(pending) = self.pending_frame.as_ref() {
            if self.should_discard_duplicate_pending_at_take(pending) {
                let frame_seq = pending.frame_seq;
                self.discard_pending_without_present();
                self.log_host_flow(
                    "take",
                    Some(frame_seq),
                    "RejectedAlreadyPresented",
                    None,
                    false,
                    0.0,
                    0.0,
                    telemetry,
                );
            } else if self.should_discard_stale_pending_at_take(pending, now_ms, telemetry) {
                let frame = pending.clone();
                let frame_age_budget_ms = telemetry.host_mailbox_take_stale_budget_ms(&frame);
                let frame_age_ms =
                    Self::host_mailbox_pending_age_ms(self.pending_accepted_at_ms, &frame, now_ms);
                self.discard_pending_without_present();
                telemetry.record_stale_frame_drop(
                    &frame,
                    now_ms,
                    "scheduledFrameStale",
                    self.queue_depth(),
                );
                self.log_host_flow(
                    "take",
                    Some(frame.frame_seq),
                    "DroppedStale",
                    None,
                    false,
                    frame_age_ms,
                    frame_age_budget_ms,
                    telemetry,
                );
                if self.displayed_frame.is_none() {
                    return ScheduledFrameTakeOutcome::DroppedStale {
                        frame,
                        frame_age_ms,
                        frame_age_budget_ms,
                    };
                }
            } else {
                let frame_age_budget_ms = telemetry.host_mailbox_take_stale_budget_ms(pending);
                let frame_age_ms =
                    Self::host_mailbox_pending_age_ms(self.pending_accepted_at_ms, pending, now_ms);
                let frame = self.pending_frame.take().expect("pending frame exists");
                self.pending_accepted_at_ms = None;
                self.last_presented_frame_seq = Some(frame.frame_seq);
                self.displayed_frame = Some(frame.clone());
                self.displayed_view_epoch = self.view_epoch;
                telemetry.clear_no_pending_streak();
                self.log_host_flow(
                    "take",
                    Some(frame.frame_seq),
                    "Ready",
                    None,
                    false,
                    frame_age_ms,
                    frame_age_budget_ms,
                    telemetry,
                );
                return ScheduledFrameTakeOutcome::Ready(frame);
            }
        }
        if self.displayed_frame.is_some() && self.displayed_view_epoch != self.view_epoch {
            let frame = self
                .displayed_frame
                .as_ref()
                .expect("displayed frame should exist when replaying view epoch")
                .clone();
            self.last_presented_frame_seq = Some(frame.frame_seq);
            self.displayed_view_epoch = self.view_epoch;
            telemetry.clear_no_pending_streak();
            self.log_host_flow(
                "take",
                Some(frame.frame_seq),
                "ReadyDisplayedReplay",
                None,
                false,
                0.0,
                0.0,
                telemetry,
            );
            return ScheduledFrameTakeOutcome::Ready(frame);
        }
        if self.displayed_frame.is_some() {
            telemetry.record_display_hold();
            telemetry.record_present_refresh(now_ms);
            self.log_host_flow(
                "take",
                self.displayed_frame.as_ref().map(|frame| frame.frame_seq),
                "RetainedDisplayedFrame",
                None,
                false,
                0.0,
                0.0,
                telemetry,
            );
            return ScheduledFrameTakeOutcome::RetainedDisplayedFrame;
        }
        telemetry.record_no_pending_take();
        self.log_host_flow(
            "take",
            None,
            "NoPendingFrame",
            None,
            false,
            0.0,
            0.0,
            telemetry,
        );
        ScheduledFrameTakeOutcome::NoPendingFrame
    }

    pub fn reset(&mut self) {
        self.displayed_frame = None;
        self.pending_frame = None;
        self.pending_accepted_at_ms = None;
        self.last_presented_frame_seq = None;
        self.view_epoch = 0;
        self.displayed_view_epoch = 0;
        self.render_loop_started = false;
    }

    pub fn begin_media_epoch(&mut self) {
        self.pending_frame = None;
        self.pending_accepted_at_ms = None;
        self.last_presented_frame_seq = None;
        // 媒体 epoch 刷新时保留已显示帧，直到新 epoch 的恢复锚点或最新帧真正接管。
        // 这样 host 末端仍有替换基准，stale recovery anchor 不会因为 displayed 被提前清空而落成 drop。
        // 同时只清去重态，不动 render_loop_started，否则会把仍在运行的 display link / fallback loop 误判成需要重启。
    }

    pub fn begin_view_epoch(&mut self) -> u64 {
        self.view_epoch = self.view_epoch.saturating_add(1);
        self.view_epoch
    }

    pub fn diagnostics_snapshot(&self) -> ScheduledFrameSlotDiagnostics {
        ScheduledFrameSlotDiagnostics {
            displayed_frame_seq: self.displayed_frame.as_ref().map(|frame| frame.frame_seq),
            displayed_frame_rtp_timestamp: self
                .displayed_frame
                .as_ref()
                .and_then(|frame| frame.rtp_timestamp),
            displayed_frame_recovery_disposition: self
                .displayed_frame
                .as_ref()
                .and_then(|frame| frame.frame_recovery_disposition.clone()),
            displayed_frame_rendered_at_ms: self
                .displayed_frame
                .as_ref()
                .map(|frame| frame.rendered_at_ms),
            pending_frame_seqs: self
                .pending_frame
                .as_ref()
                .map(|frame| vec![frame.frame_seq])
                .unwrap_or_default(),
            pending_frame_rtp_timestamp: self
                .pending_frame
                .as_ref()
                .and_then(|frame| frame.rtp_timestamp),
            last_presented_frame_seq: self.last_presented_frame_seq,
            queue_depth: self.queue_depth(),
            pending_queue_depth: usize::from(self.pending_frame.is_some()),
            has_displayed_frame: self.displayed_frame.is_some(),
            view_epoch: self.view_epoch,
            displayed_view_epoch: self.displayed_view_epoch,
            render_loop_started: self.render_loop_started,
        }
    }

    fn queue_depth(&self) -> usize {
        usize::from(self.pending_frame.is_some()) + usize::from(self.displayed_frame.is_some())
    }

    #[cfg(test)]
    pub(crate) fn set_pending_for_test(
        &mut self,
        frame: XbxEngineRenderFrame,
        accepted_at_ms: f64,
    ) {
        self.pending_frame = Some(frame);
        self.pending_accepted_at_ms = Some(accepted_at_ms);
    }

    fn log_host_flow(
        &self,
        _event: &str,
        _frame_seq: Option<u64>,
        _slot_outcome: &str,
        _replaced_frame_seq: Option<u64>,
        _overwrote_pending: bool,
        _frame_age_ms: f64,
        _frame_age_budget_ms: f64,
        _telemetry: &HostCadenceTelemetry,
    ) {
        let _diagnostics = self.diagnostics_snapshot();
        // log::info!(
        //     "[playback-flow][host] event={} slot_outcome={} frame_seq={} displayed_frame_seq={} last_presented_frame_seq={} pending_frame_seqs={} queue_depth={} pending_queue_depth={} display_tick_epoch={} present_epoch={} overwrote_pending={} replaced_frame_seq={} frame_age_ms={:.2} frame_age_budget_ms={:.2} no_pending_streak={} cadence_phase={}",
        //     event,
        //     slot_outcome,
        //     format_optional_seq(frame_seq),
        //     format_optional_seq(diagnostics.displayed_frame_seq),
        //     format_optional_seq(diagnostics.last_presented_frame_seq),
        //     format_frame_seq_list(&diagnostics.pending_frame_seqs),
        //     diagnostics.queue_depth,
        //     diagnostics.pending_queue_depth,
        //     telemetry.display_tick_epoch(),
        //     telemetry.present_epoch(),
        //     overwrote_pending,
        //     format_optional_seq(replaced_frame_seq),
        //     frame_age_ms,
        //     frame_age_budget_ms,
        //     telemetry.no_pending_streak,
        //     telemetry.cadence_phase().as_str(),
        // );
    }
}

fn frame_has_anchor_protection(frame: &XbxEngineRenderFrame) -> bool {
    matches!(
        frame.frame_recovery_disposition.as_deref(),
        Some("rebuilding") | Some("rebuilding-supply")
    )
}

fn frame_confirms_recovery_owner(frame: &XbxEngineRenderFrame) -> bool {
    frame.frame_unrecoverable_reason.is_none()
        && frame
            .recovery_owner_rtp_timestamp
            .zip(frame.rtp_timestamp)
            .is_some_and(|(owner_rtp, frame_rtp)| owner_rtp == frame_rtp)
}

fn incoming_frame_is_newer_than_pending(
    pending: &XbxEngineRenderFrame,
    incoming: &XbxEngineRenderFrame,
) -> bool {
    match (pending.recovery_epoch_tag, incoming.recovery_epoch_tag) {
        (Some(existing), Some(candidate)) if candidate != existing => return candidate > existing,
        (None, Some(_)) => return true,
        (Some(_), None) => return false,
        _ => {}
    }

    if incoming.frame_seq != pending.frame_seq {
        return incoming.frame_seq > pending.frame_seq;
    }

    match (pending.rtp_timestamp, incoming.rtp_timestamp) {
        (Some(existing), Some(candidate)) if candidate != existing => return candidate > existing,
        (None, Some(_)) => return true,
        (Some(_), None) => return false,
        _ => {}
    }

    incoming.rendered_at_ms >= pending.rendered_at_ms
}

#[cfg(test)]
#[path = "scheduling.test.rs"]
mod tests;

const PRESENT_FPS_MIN_SAMPLES: usize = 3;
const PRESENT_FPS_MIN_WINDOW_MS: f64 = 150.0;

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
