use std::sync::{Arc, Mutex};

use xbxengine_protocol::XbxEngineDisplayStateDto;

use crate::api::backend::{XbxEngineMediaRuntimeStats, XbxEngineRenderFrame};
use crate::media::video::render::renderer::XbxRenderState;
use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::stream::RtcMediaService;
use crate::XbxEngineRuntimeError;

use super::runtime_stats::merge_media_snapshot_into_runtime_stats;

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
            stats.video_present_fps = render_signal.fps;
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

    pub(crate) fn update_host_video_timing(
        &self,
        host_display_interval_ms: Option<f64>,
        host_frame_age_budget_ms: Option<f64>,
    ) {
        RuntimeStatsSink::new(self.runtime_stats.clone())
            .record_host_video_timing(host_display_interval_ms, host_frame_age_budget_ms);
    }
}
