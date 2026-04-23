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
const HOST_TIMING_DISPLAY_INTERVAL_RECREATE_THRESHOLD_MS: f64 = 3.0;
const HOST_TIMING_FRAME_AGE_BUDGET_RECREATE_THRESHOLD_MS: f64 = 8.0;
const HOST_TIMING_LOCAL_RESET_COOLDOWN_MS: f64 = 1_500.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct HostVideoTimingSample {
    host_display_interval_ms: Option<f64>,
    host_frame_age_budget_ms: Option<f64>,
}

#[derive(Debug, Default)]
pub(crate) struct LocalDecoderResetPolicyState {
    last_sample: Option<HostVideoTimingSample>,
    last_reset_at_ms: Option<f64>,
}

impl LocalDecoderResetPolicyState {
    fn observe_host_timing_change(
        &mut self,
        host_display_interval_ms: Option<f64>,
        host_frame_age_budget_ms: Option<f64>,
        observed_at_ms: f64,
    ) -> Option<String> {
        let next = HostVideoTimingSample {
            host_display_interval_ms,
            host_frame_age_budget_ms,
        };
        let previous = self.last_sample.replace(next)?;
        let display_delta_ms = timing_delta_ms(
            previous.host_display_interval_ms,
            next.host_display_interval_ms,
        );
        let frame_age_budget_delta_ms = timing_delta_ms(
            previous.host_frame_age_budget_ms,
            next.host_frame_age_budget_ms,
        );
        // presenter 重建 / 首帧 priming 会经历 `Some(..) <-> None` 的宿主 timing 过渡，
        // 这属于宿主显示链切 epoch 的正常抖动，不应误判为 decoder 需要重建。
        if previous.host_display_interval_ms.is_none() || next.host_display_interval_ms.is_none() {
            return None;
        }
        let display_changed = display_delta_ms
            .is_some_and(|delta| delta >= HOST_TIMING_DISPLAY_INTERVAL_RECREATE_THRESHOLD_MS);
        let budget_changed = frame_age_budget_delta_ms
            .is_some_and(|delta| delta >= HOST_TIMING_FRAME_AGE_BUDGET_RECREATE_THRESHOLD_MS);
        if !display_changed && !budget_changed {
            return None;
        }
        if self.last_reset_at_ms.is_some_and(|last| {
            (observed_at_ms - last).max(0.0) < HOST_TIMING_LOCAL_RESET_COOLDOWN_MS
        }) {
            return None;
        }
        self.last_reset_at_ms = Some(observed_at_ms);
        Some(build_host_timing_reset_reason(
            previous,
            next,
            display_delta_ms,
            frame_age_budget_delta_ms,
        ))
    }
}

// 负责 render/runtime 只读与轻写入口，避免 stack.rs 持续膨胀。
pub(crate) struct RtcStackRuntimePort<'a> {
    runtime_stats: &'a Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    render_state: &'a Arc<Mutex<XbxRenderState>>,
    media: &'a Arc<Mutex<RtcMediaService>>,
    local_decoder_reset_policy: &'a Arc<Mutex<LocalDecoderResetPolicyState>>,
}

impl<'a> RtcStackRuntimePort<'a> {
    pub(crate) fn new(
        runtime_stats: &'a Arc<Mutex<XbxEngineMediaRuntimeStats>>,
        render_state: &'a Arc<Mutex<XbxRenderState>>,
        media: &'a Arc<Mutex<RtcMediaService>>,
        _local_decoder_reset_handle: &'a Arc<
            Mutex<Option<Arc<crate::media::video::decode::actor::DecodeActorHandle>>>,
        >,
        local_decoder_reset_policy: &'a Arc<Mutex<LocalDecoderResetPolicyState>>,
    ) -> Self {
        Self {
            runtime_stats,
            render_state,
            media,
            local_decoder_reset_policy,
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
            // 语义分层：
            // - present freshness 统一使用 host telemetry 写入的 latest_video_host_present_time_ms
            // - render_signal 只负责 renderer stalled 判定，避免与 host present 语义混用
            stats.video_renderer_stalled = render_signal.renderer_stalled;
        }
        crate::xbx_log_debug!("[xbxengine][rtc-stack] snapshot_runtime_stats exit");
        stats
    }

    pub(crate) fn take_latest_render_frame(&self) -> Option<XbxEngineRenderFrame> {
        self.render_state
            .lock()
            .ok()
            .and_then(|mut render_state| render_state.take_latest_renderable_frame())
    }

    pub(crate) fn record_video_frame_drop(&self, observation: XbxEngineVideoFrameDropObservation) {
        RuntimeStatsSink::new(self.runtime_stats.clone()).record_video_frame_drop(observation);
    }

    pub(crate) fn update_host_video_timing(
        &self,
        host_display_interval_ms: Option<f64>,
        host_frame_age_budget_ms: Option<f64>,
    ) {
        let runtime_stats = RuntimeStatsSink::new(self.runtime_stats.clone());
        runtime_stats.record_host_video_timing(host_display_interval_ms, host_frame_age_budget_ms);
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        let timing_shift_reason =
            self.local_decoder_reset_policy
                .lock()
                .ok()
                .and_then(|mut policy| {
                    policy.observe_host_timing_change(
                        host_display_interval_ms,
                        host_frame_age_budget_ms,
                        now_ms,
                    )
                });
        let Some(reason) = timing_shift_reason else {
            return;
        };
        runtime_stats.update(|stats| {
            stats.latest_observation_label = Some("videoHostTimingShiftObserved".to_string());
            stats.latest_observation_summary = Some(reason.clone());
        });
    }

    pub(crate) fn update_host_video_present_metrics(
        &self,
        metrics: XbxEngineHostVideoPresentMetrics,
    ) {
        RuntimeStatsSink::new(self.runtime_stats.clone()).update(|stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.latest_video_host_submit_time_ms = metrics.latest_host_submit_time_ms;
            stats.latest_video_host_present_time_ms = metrics.latest_host_present_time_ms;
            stats.host_submit_epoch = metrics.host_submit_epoch;
            stats.host_display_tick_epoch = metrics.display_tick_epoch;
            stats.display_present_epoch = metrics.display_present_epoch;
            stats.video_present_epoch = metrics.display_present_epoch;
            stats.host_cadence_phase = metrics.cadence_phase;
            stats.submit_age_ms = metrics
                .latest_host_submit_time_ms
                .map(|ts| (now_ms - ts).max(0.0));
            stats.display_age_ms = metrics
                .latest_host_present_time_ms
                .map(|ts| (now_ms - ts).max(0.0));
            stats.last_displayed_frame_seq = metrics.last_displayed_frame_seq;
            stats.last_displayed_frame_rtp_timestamp = metrics.last_displayed_frame_rtp_timestamp;
            stats.last_displayed_at_ms = metrics.last_displayed_at_ms;
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
            frame_budget: None,
            observed_at_ms: event.observed_at_ms,
            width: event.width,
            height: event.height,
            is_keyframe: event.is_keyframe,
            queue_depth: event.queue_depth,
        };
        RuntimeStatsSink::new(self.runtime_stats.clone()).record_video_frame_drop(observation);
    }
}

fn timing_delta_ms(previous: Option<f64>, next: Option<f64>) -> Option<f64> {
    match (previous, next) {
        (Some(previous), Some(next)) => Some((next - previous).abs()),
        _ => None,
    }
}

fn build_host_timing_reset_reason(
    previous: HostVideoTimingSample,
    next: HostVideoTimingSample,
    display_delta_ms: Option<f64>,
    frame_age_budget_delta_ms: Option<f64>,
) -> String {
    format!(
        "hostTimingShift displayInterval={}=>{} delta={} frameAgeBudget={}=>{} delta={}",
        format_timing_value(previous.host_display_interval_ms),
        format_timing_value(next.host_display_interval_ms),
        format_timing_value(display_delta_ms),
        format_timing_value(previous.host_frame_age_budget_ms),
        format_timing_value(next.host_frame_age_budget_ms),
        format_timing_value(frame_age_budget_delta_ms),
    )
}

fn format_timing_value(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:.2}"))
        .unwrap_or_else(|| "none".to_string())
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

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::sync::{Arc, Mutex};

    use crate::api::backend::XbxEngineMediaRuntimeStats;
    use crate::media::video::decode::actor::DecodeActorHandle;
    use crate::media::video::render::renderer::XbxRenderState;
    use crate::transport::rtc::stream::RtcMediaService;

    use super::{LocalDecoderResetPolicyState, RtcStackRuntimePort};

    #[test]
    fn small_host_timing_change_does_not_trigger_local_decoder_reset() {
        let mut state = LocalDecoderResetPolicyState::default();

        assert!(state
            .observe_host_timing_change(Some(16.67), Some(25.0), 1_000.0)
            .is_none());
        assert!(state
            .observe_host_timing_change(Some(17.10), Some(29.0), 1_100.0)
            .is_none());
    }

    #[test]
    fn obvious_host_timing_change_triggers_local_decoder_reset() {
        let mut state = LocalDecoderResetPolicyState::default();

        assert!(state
            .observe_host_timing_change(Some(16.67), Some(25.0), 1_000.0)
            .is_none());
        let reason = state
            .observe_host_timing_change(Some(33.33), Some(40.0), 3_000.0)
            .expect("obvious timing shift should trigger local reset");

        assert!(reason.contains("hostTimingShift"));
        assert!(reason.contains("16.67=>33.33"));
    }

    #[test]
    fn presenter_epoch_rebuild_timing_shift_does_not_trigger_local_decoder_reset() {
        let mut state = LocalDecoderResetPolicyState::default();

        assert!(state
            .observe_host_timing_change(Some(8.32), Some(24.0), 1_000.0)
            .is_none());
        assert!(state
            .observe_host_timing_change(None, Some(75.0), 3_000.0)
            .is_none());
    }

    #[test]
    fn reset_trigger_is_debounced_within_cooldown_window() {
        let mut state = LocalDecoderResetPolicyState::default();

        assert!(state
            .observe_host_timing_change(Some(16.67), Some(25.0), 1_000.0)
            .is_none());
        assert!(state
            .observe_host_timing_change(Some(33.33), Some(40.0), 3_000.0)
            .is_some());
        assert!(state
            .observe_host_timing_change(Some(16.67), Some(25.0), 3_400.0)
            .is_none());
    }

    #[test]
    fn host_timing_shift_only_records_observation_without_requesting_decoder_reset() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let render_state = Arc::new(Mutex::new(XbxRenderState::default()));
        let media = Arc::new(Mutex::new(RtcMediaService::default()));
        let (tx, rx) = mpsc::sync_channel(1);
        let local_decoder_reset_handle = Arc::new(Mutex::new(Some(Arc::new(
            DecodeActorHandle::from_test_sender(tx),
        ))));
        let local_decoder_reset_policy =
            Arc::new(Mutex::new(LocalDecoderResetPolicyState::default()));
        let runtime_port = RtcStackRuntimePort::new(
            &runtime_stats,
            &render_state,
            &media,
            &local_decoder_reset_handle,
            &local_decoder_reset_policy,
        );

        runtime_port.update_host_video_timing(Some(16.67), Some(25.0));
        runtime_port.update_host_video_timing(Some(33.33), Some(40.0));

        let snapshot = runtime_stats.lock().expect("runtime stats lock").clone();
        assert_eq!(
            snapshot.latest_observation_label.as_deref(),
            Some("videoHostTimingShiftObserved")
        );
        let summary = snapshot
            .latest_observation_summary
            .as_deref()
            .expect("timing shift summary");
        assert!(summary.contains("hostTimingShift"));
        assert!(rx.try_recv().is_err());
    }
}
