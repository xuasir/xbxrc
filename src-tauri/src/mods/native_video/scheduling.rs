use std::collections::VecDeque;

use xbxengine::XbxEngineRenderFrame;

const HOST_RENDER_FPS_WINDOW_MS: f64 = 1_000.0;
const HOST_RENDER_MIN_FRAME_AGE_MS: f64 = 24.0;
const HOST_RENDER_MAX_FRAME_AGE_MS: f64 = 75.0;
const HOST_RENDER_FRAME_AGE_MULTIPLIER: f64 = 2.25;

#[derive(Default)]
pub struct HostCadenceTelemetry {
    pub latest_present_time_ms: Option<f64>,
    recent_present_times_ms: VecDeque<f64>,
    recent_display_tick_times_ms: VecDeque<f64>,
    pub present_submit_count_total: u64,
    pub present_drop_count_total: u64,
    pub present_overwrite_count_total: u64,
}

impl HostCadenceTelemetry {
    pub fn record_display_tick(&mut self, now_ms: f64) {
        self.recent_display_tick_times_ms.push_back(now_ms);
        self.trim_display_ticks(now_ms);
    }

    pub fn record_present(&mut self, now_ms: f64) {
        self.latest_present_time_ms = Some(now_ms);
        self.recent_present_times_ms.push_back(now_ms);
        self.trim_recent(now_ms);
    }

    pub fn record_drop(&mut self) {
        self.present_drop_count_total = self.present_drop_count_total.saturating_add(1);
    }

    pub fn record_overwrite(&mut self) {
        self.present_overwrite_count_total = self.present_overwrite_count_total.saturating_add(1);
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

    pub fn reset_frame_slot(&mut self) {
        self.latest_present_time_ms = None;
        self.recent_present_times_ms.clear();
        self.recent_display_tick_times_ms.clear();
        // 会话 detach / reattach 后需要重新统计宿主 present 指标，
        // 否则新会话会继承上一轮 submit/drop/overwrite 计数，诊断会失真。
        self.present_submit_count_total = 0;
        self.present_drop_count_total = 0;
        self.present_overwrite_count_total = 0;
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
    latest_frame: Option<XbxEngineRenderFrame>,
    last_presented_frame_seq: Option<u64>,
    pub render_loop_started: bool,
}

impl ScheduledFrameSlot {
    pub fn submit_frame(
        &mut self,
        frame: &XbxEngineRenderFrame,
        now_ms: f64,
        telemetry: &mut HostCadenceTelemetry,
    ) -> bool {
        if now_ms - frame.rendered_at_ms > telemetry.frame_age_budget_ms() {
            telemetry.record_drop();
            return false;
        }
        if self
            .last_presented_frame_seq
            .is_some_and(|frame_seq| frame.frame_seq <= frame_seq)
        {
            return false;
        }
        if self.latest_frame.as_ref().is_some_and(|latest| {
            Some(latest.frame_seq) != self.last_presented_frame_seq
                && latest.frame_seq != frame.frame_seq
        }) {
            telemetry.record_overwrite();
        }
        self.latest_frame = Some(frame.clone());
        true
    }

    pub fn take_ready_frame(
        &mut self,
        now_ms: f64,
        telemetry: &mut HostCadenceTelemetry,
    ) -> Option<XbxEngineRenderFrame> {
        let frame = self.latest_frame.as_ref()?;
        if self
            .last_presented_frame_seq
            .is_some_and(|frame_seq| frame.frame_seq <= frame_seq)
        {
            self.latest_frame = None;
            return None;
        }
        if now_ms - frame.rendered_at_ms > telemetry.frame_age_budget_ms() {
            self.latest_frame = None;
            telemetry.record_drop();
            return None;
        }
        let frame = self.latest_frame.take()?;
        self.last_presented_frame_seq = Some(frame.frame_seq);
        Some(frame)
    }

    pub fn reset(&mut self) {
        self.latest_frame = None;
        self.last_presented_frame_seq = None;
        self.render_loop_started = false;
    }
}

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
