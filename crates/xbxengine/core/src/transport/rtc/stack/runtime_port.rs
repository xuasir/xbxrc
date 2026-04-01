use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use xbxengine_protocol::XbxEngineDisplayStateDto;

use crate::api::backend::{
    XbxEngineHostVideoFrameDropEvent, XbxEngineHostVideoPresentMetrics, XbxEngineMediaRuntimeStats,
    XbxEngineRenderFrame, XbxEngineVideoFrameDropObservation,
};
use crate::media::video::render::renderer::XbxRenderState;
use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::stream::RtcMediaService;
use crate::XbxEngineRuntimeError;

use super::runtime_stats::merge_media_snapshot_into_runtime_stats;

static HOST_FRAME_DROP_OBSERVATION_ID: AtomicU64 = AtomicU64::new(0);

// 负责 render/runtime 只读与轻写入口，避免 stack.rs 持续膨胀。
pub(crate) struct RtcStackRuntimePort<'a> {
    runtime_stats: &'a Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    render_state: &'a Arc<Mutex<XbxRenderState>>,
    media: &'a Arc<Mutex<RtcMediaService>>,
}

impl<'a> RtcStackRuntimePort<'a> {
    pub(crate) fn new(
        runtime_stats: &'a Arc<Mutex<XbxEngineMediaRuntimeStats>>,
        render_state: &'a Arc<Mutex<XbxRenderState>>,
        media: &'a Arc<Mutex<RtcMediaService>>,
    ) -> Self {
        Self {
            runtime_stats,
            render_state,
            media,
        }
    }

    pub(crate) fn apply_display_state(
        &self,
        state: XbxEngineDisplayStateDto,
    ) -> Result<(), XbxEngineRuntimeError> {
        let mut render_state = self
            .render_state
            .lock()
            .map_err(|_| XbxEngineRuntimeError::new("xbxEngineRenderStateLockFailed"))?;
        render_state.apply_display_state(state)
    }

    pub(crate) fn snapshot_runtime_stats(&self) -> XbxEngineMediaRuntimeStats {
        crate::xbx_log_debug!("[xbxengine][rtc-stack] snapshot_runtime_stats enter");
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        let mut stats = self
            .runtime_stats
            .lock()
            .ok()
            .map(|mut guard| {
                if let Ok(media) = self.media.lock() {
                    let media_snapshot = media.snapshot();
                    merge_media_snapshot_into_runtime_stats(&mut guard, &media_snapshot, now_ms);
                }
                guard.clone()
            })
            .unwrap_or_default();
        crate::xbx_log_debug!("[xbxengine][rtc-stack] snapshot_runtime_stats runtime_stats cloned");
        if let Ok(render_state) = self.render_state.lock() {
            let render_signal = render_state.render_signal_snapshot(now_ms);
            stats.latest_video_present_time_ms = render_signal.latest_present_time_ms;
            stats.video_renderer_stalled = render_signal.renderer_stalled;
        }
        crate::xbx_log_debug!("[xbxengine][rtc-stack] snapshot_runtime_stats exit");
        stats
    }

    pub(crate) fn take_latest_render_frame(&self) -> Option<XbxEngineRenderFrame> {
        self.render_state
            .lock()
            .ok()
            .and_then(|mut render_state| render_state.take_latest_frame())
    }

    pub(crate) fn acknowledge_latest_render_frame(&self, frame_seq: u64) -> bool {
        self.render_state
            .lock()
            .ok()
            .is_some_and(|mut render_state| render_state.acknowledge_latest_frame(frame_seq))
    }

    pub(crate) fn record_video_frame_drop(&self, observation: XbxEngineVideoFrameDropObservation) {
        RuntimeStatsSink::new(self.runtime_stats.clone()).record_video_frame_drop(observation);
    }

    pub(crate) fn update_host_video_timing(
        &self,
        host_display_interval_ms: Option<f64>,
        host_frame_age_budget_ms: Option<f64>,
    ) {
        RuntimeStatsSink::new(self.runtime_stats.clone())
            .record_host_video_timing(host_display_interval_ms, host_frame_age_budget_ms);
    }

    pub(crate) fn update_host_video_present_metrics(
        &self,
        metrics: XbxEngineHostVideoPresentMetrics,
    ) {
        RuntimeStatsSink::new(self.runtime_stats.clone()).update(|stats| {
            stats.video_present_fps = metrics.present_fps;
            stats.video_present_submit_count_total = metrics.present_submit_count_total;
            stats.video_present_drop_count_total = metrics.present_drop_count_total;
            stats.video_present_overwrite_count_total = metrics.present_overwrite_count_total;
            stats.host_no_pending_take_count_total = metrics.no_pending_take_count_total;
            stats.host_no_pending_streak = metrics.no_pending_streak;
            stats.host_no_pending_max_streak = metrics.no_pending_max_streak;
            stats.host_no_pending_pressure_level =
                Some(resolve_no_pending_pressure_level(metrics.no_pending_streak).to_string());
            stats.video_present_descriptor_upload_mode = metrics.descriptor_upload_mode;
            stats.video_present_descriptor_metal_import_count_total =
                metrics.descriptor_metal_import_count_total;
            stats.video_present_descriptor_cpu_upload_count_total =
                metrics.descriptor_cpu_upload_count_total;
        });
    }

    pub(crate) fn record_host_video_frame_drop(&self, event: XbxEngineHostVideoFrameDropEvent) {
        let observation = XbxEngineVideoFrameDropObservation {
            observation_id: HOST_FRAME_DROP_OBSERVATION_ID.fetch_add(1, Ordering::Relaxed) + 1,
            reason: "dropLate".to_string(),
            stage: event.stage,
            action: event.action,
            detail: event.detail,
            frame_rtp_timestamp: event.frame_rtp_timestamp,
            frame_seq: event.frame_seq,
            frame_recovery_disposition: event.frame_recovery_disposition,
            frame_unrecoverable_reason: event.frame_unrecoverable_reason,
            observed_at_ms: event.observed_at_ms,
            width: event.width,
            height: event.height,
            is_keyframe: event.is_keyframe,
            queue_depth: event.queue_depth,
        };
        RuntimeStatsSink::new(self.runtime_stats.clone()).record_video_frame_drop(observation);
    }
}

fn resolve_no_pending_pressure_level(streak: u32) -> &'static str {
    if streak >= 180 {
        "critical"
    } else if streak >= 60 {
        "high"
    } else if streak >= 20 {
        "elevated"
    } else {
        "normal"
    }
}
