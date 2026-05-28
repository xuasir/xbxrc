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
            // - render_signal 负责 renderer stalled 原始信号
            // - host present/view 重建导致的 stalled 由 runtime_stats 写回，这里需要并集保留
            stats.video_renderer_stalled = Some(
                stats.video_renderer_stalled.unwrap_or(false)
                    || render_signal.renderer_stalled.unwrap_or(false),
            );
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
        let runtime_stats = RuntimeStatsSink::new(self.runtime_stats.clone());
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        let host_submit_gap_ms = metrics
            .latest_host_submit_time_ms
            .zip(metrics.latest_host_present_time_ms)
            .map(|(submit_at_ms, present_at_ms)| (submit_at_ms - present_at_ms).max(0.0));
        let host_view_pending_present =
            metrics
                .latest_host_view_created_at_ms
                .is_some_and(|created_at_ms| {
                    metrics
                        .latest_host_present_time_ms
                        .is_none_or(|present_at_ms| present_at_ms < created_at_ms)
                });
        let host_display_hold = metrics.host_frame_present_epoch > 0
            && metrics.cadence_phase.as_deref() == Some("steady")
            && metrics.no_pending_streak == 0
            && metrics
                .latest_host_present_time_ms
                .is_some_and(|present_at_ms| (now_ms - present_at_ms).max(0.0) <= 600.0);
        let host_visibility_stalled = !host_display_hold
            && metrics
                .latest_host_present_time_ms
                .map(|present_at_ms| (now_ms - present_at_ms).max(0.0) >= 1_500.0)
                .unwrap_or(false)
            && (metrics.host_mailbox_enqueue_count_total > metrics.host_frame_present_epoch
                || metrics.latest_host_submit_rtp_timestamp
                    != metrics.last_displayed_frame_rtp_timestamp
                || host_submit_gap_ms.is_some_and(|gap_ms| gap_ms >= 250.0)
                || host_view_pending_present);

        runtime_stats.update(|stats| {
            stats.latest_host_mailbox_submit_time_ms = metrics.latest_host_submit_time_ms;
            stats.latest_video_host_submit_rtp_timestamp = metrics.latest_host_submit_rtp_timestamp;
            stats.latest_video_host_present_time_ms = metrics.latest_host_present_time_ms;
            stats.host_view_generation = metrics.host_view_generation;
            stats.latest_host_view_created_at_ms = metrics.latest_host_view_created_at_ms;
            stats.host_mailbox_submit_epoch = metrics.host_mailbox_submit_epoch;
            stats.host_display_tick_epoch = metrics.host_display_tick_epoch;
            stats.host_frame_present_epoch = metrics.host_frame_present_epoch;
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
            stats.host_mailbox_enqueue_count_total = metrics.host_mailbox_enqueue_count_total;
            stats.host_mailbox_drop_count_total = metrics.host_mailbox_drop_count_total;
            stats.host_mailbox_overwrite_count_total = metrics.host_mailbox_overwrite_count_total;
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
            stats.video_renderer_stalled =
                Some(stats.video_renderer_stalled.unwrap_or(false) || host_visibility_stalled);
        });

        if let (Some(observed_at_ms), Some(displayed_rtp_timestamp)) = (
            metrics.last_displayed_at_ms,
            metrics.last_displayed_frame_rtp_timestamp,
        ) {
            let anchor_rtp = runtime_stats
                .read(|stats| {
                    crate::transport::rtc::recovery::contract::resolve_host_display_idr_anchor_rtp(
                        stats,
                        Some(displayed_rtp_timestamp),
                    )
                })
                .unwrap_or(Some(displayed_rtp_timestamp));
            if let Some(anchor_rtp) = anchor_rtp {
                runtime_stats.record_displayed_idr_fact(
                    observed_at_ms,
                    anchor_rtp,
                    metrics.last_displayed_frame_seq,
                );
            }
            runtime_stats.record_playback_recovered_fact(observed_at_ms, metrics.present_fps);
        }
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
            replacement_decision: None,
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

    use crate::api::backend::XbxEngineHostVideoPresentMetrics;
    use crate::api::backend::XbxEngineMediaRuntimeStats;

    fn seed_decoder_reference_sync_for_pending_idr(
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
        rtp_timestamp: u32,
        observed_at_ms: f64,
    ) {
        let mut stats = runtime_stats.lock().expect("runtime stats lock");
        stats.recovery_decoder_reference_synced_at_ms = Some(observed_at_ms);
        stats.latest_video_decode_ok_time_ms = Some(observed_at_ms);
        stats.latest_video_decode_ok_rtp_timestamp = Some(rtp_timestamp);
    }
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

    #[test]
    fn host_present_of_matching_submission_commits_clean_anchor() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let render_state = Arc::new(Mutex::new(XbxRenderState::default()));
        let media = Arc::new(Mutex::new(RtcMediaService::default()));
        let (tx, _rx) = mpsc::sync_channel(1);
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

        {
            let sink = crate::runtime_stats_sink::RuntimeStatsSink::new(runtime_stats.clone());
            sink.begin_transport_recovery_episode(10.0);
            sink.record_picture_recovery_episode_requested(
                88,
                Some("receiverWaitingKeyframe".to_string()),
                100.0,
                None,
            );
            sink.record_picture_recovery_episode_sent("pli", 120.0, Some(300.0));
            sink.record_picture_recovery_episode_response_observed(
                150.0,
                Some(77_001),
                true,
                "firstAcceptedIdr",
                Some(11),
                None,
                false,
                false,
            );
            sink.record_picture_recovery_episode_decoded(180.0, 77_001, 55);
            sink.record_pending_displayed_idr_rtp(77_001);
        }
        seed_decoder_reference_sync_for_pending_idr(&runtime_stats, 77_001, 180.0);

        runtime_port.update_host_video_present_metrics(XbxEngineHostVideoPresentMetrics {
            latest_host_present_time_ms: Some(210.0),
            host_frame_present_epoch: 1,
            present_fps: 30.0,
            last_displayed_frame_seq: Some(55),
            last_displayed_frame_rtp_timestamp: Some(77_001),
            last_displayed_at_ms: Some(210.0),
            ..Default::default()
        });

        let snapshot = runtime_stats.lock().expect("runtime stats lock").clone();
        assert_eq!(snapshot.video_anchor_clean_epoch, Some(1));
        assert_eq!(snapshot.video_anchor_clean_observed_at_ms, Some(210.0));
        assert_eq!(
            snapshot.video_anchor_clean_source_event.as_deref(),
            Some("displayed-idr")
        );
        assert_eq!(snapshot.recovery_displayed_idr_rtp, Some(77_001));
        assert_eq!(snapshot.recovery_fresh_anchor_recovered_at_ms, Some(210.0));
    }

    #[test]
    fn host_present_with_pending_idr_establishes_anchor_when_displayed_delta_is_latest_only() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let render_state = Arc::new(Mutex::new(XbxRenderState::default()));
        let media = Arc::new(Mutex::new(RtcMediaService::default()));
        let (tx, _rx) = mpsc::sync_channel(1);
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

        {
            let sink = crate::runtime_stats_sink::RuntimeStatsSink::new(runtime_stats.clone());
            sink.begin_transport_recovery_episode(10.0);
            sink.record_picture_recovery_episode_requested(
                88,
                Some("receiverWaitingKeyframe".to_string()),
                100.0,
                None,
            );
            sink.record_picture_recovery_episode_sent("pli", 120.0, Some(300.0));
            sink.record_picture_recovery_episode_response_observed(
                150.0,
                Some(77_001),
                true,
                "firstAcceptedIdr",
                Some(11),
                None,
                false,
                false,
            );
            sink.record_picture_recovery_episode_decoded(180.0, 77_001, 55);
            sink.record_pending_displayed_idr_rtp(77_001);
        }
        seed_decoder_reference_sync_for_pending_idr(&runtime_stats, 77_001, 180.0);

        runtime_port.update_host_video_present_metrics(XbxEngineHostVideoPresentMetrics {
            latest_host_present_time_ms: Some(210.0),
            host_frame_present_epoch: 1,
            present_fps: 30.0,
            last_displayed_frame_seq: Some(56),
            last_displayed_frame_rtp_timestamp: Some(77_002),
            last_displayed_at_ms: Some(210.0),
            ..Default::default()
        });

        let snapshot = runtime_stats.lock().expect("runtime stats lock").clone();
        assert_eq!(snapshot.video_anchor_clean_epoch, Some(1));
        assert_eq!(snapshot.recovery_fresh_anchor_recovered_at_ms, Some(210.0));
        assert_eq!(snapshot.recovery_displayed_idr_rtp, Some(77_001));
        assert_eq!(snapshot.recovery_displayed_idr_at_ms, Some(210.0));
        assert_eq!(snapshot.recovery_playback_recovered_at_ms, Some(210.0));
    }

    #[test]
    fn host_present_of_serviceable_continuation_commits_clean_anchor_without_response_frame_seq() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let render_state = Arc::new(Mutex::new(XbxRenderState::default()));
        let media = Arc::new(Mutex::new(RtcMediaService::default()));
        let (tx, _rx) = mpsc::sync_channel(1);
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

        {
            let sink = crate::runtime_stats_sink::RuntimeStatsSink::new(runtime_stats.clone());
            sink.begin_transport_recovery_episode(10.0);
            sink.record_picture_recovery_episode_requested(
                188,
                Some("receiverWaitingKeyframe".to_string()),
                100.0,
                None,
            );
            sink.record_picture_recovery_episode_sent("pli", 120.0, Some(300.0));
            sink.record_picture_recovery_episode_response_observed(
                150.0,
                Some(77_001),
                true,
                "firstAcceptedIdr",
                Some(11),
                None,
                false,
                false,
            );
            sink.record_picture_recovery_episode_decoded(180.0, 77_001, 55);
            sink.record_pending_displayed_idr_rtp(77_001);
            sink.update(|stats| {
                if let Some(episode) = stats.latest_keyframe_request_episode.as_mut() {
                    episode.response_frame_seq = None;
                }
                stats.latest_h264_inspection_observation =
                    Some(crate::XbxEngineH264InspectionObservation {
                        observation_id: 77,
                        frame_rtp_timestamp: Some(77_002),
                        committed_sps_present: true,
                        committed_pps_present: true,
                        delta_continuation_ready: true,
                        bootstrap_ready: false,
                        bootstrap_reject_reason: Some("bootstrapMissingIdr".to_string()),
                        continuation_verdict: Some("receiverLocalContinuation".to_string()),
                        admission_accepted: true,
                        observed_at_ms: 205.0,
                        bound_episode_id: Some(188),
                        bound_recovery_epoch: Some(1),
                        ..Default::default()
                    });
            });
        }
        seed_decoder_reference_sync_for_pending_idr(&runtime_stats, 77_001, 180.0);

        runtime_port.update_host_video_present_metrics(XbxEngineHostVideoPresentMetrics {
            latest_host_present_time_ms: Some(210.0),
            host_frame_present_epoch: 1,
            present_fps: 30.0,
            last_displayed_frame_seq: Some(56),
            last_displayed_frame_rtp_timestamp: Some(77_002),
            last_displayed_at_ms: Some(210.0),
            ..Default::default()
        });

        let snapshot = runtime_stats.lock().expect("runtime stats lock").clone();
        assert_eq!(snapshot.video_anchor_clean_epoch, Some(1));
        assert_eq!(snapshot.recovery_fresh_anchor_recovered_at_ms, Some(210.0));
        assert_eq!(snapshot.recovery_displayed_idr_rtp, Some(77_001));
        assert_eq!(snapshot.recovery_displayed_idr_at_ms, Some(210.0));
    }

    #[test]
    fn host_present_of_submitted_anchor_commits_after_owner_advances_within_same_episode() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let render_state = Arc::new(Mutex::new(XbxRenderState::default()));
        let media = Arc::new(Mutex::new(RtcMediaService::default()));
        let (tx, _rx) = mpsc::sync_channel(1);
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

        {
            let sink = crate::runtime_stats_sink::RuntimeStatsSink::new(runtime_stats.clone());
            sink.begin_transport_recovery_episode(10.0);
            sink.record_picture_recovery_episode_requested(
                188,
                Some("receiverWaitingKeyframe".to_string()),
                100.0,
                None,
            );
            sink.record_picture_recovery_episode_sent("pli", 120.0, Some(300.0));
            sink.record_picture_recovery_episode_response_observed(
                150.0,
                Some(77_001),
                true,
                "firstAcceptedIdr",
                Some(11),
                None,
                false,
                false,
            );
            sink.record_picture_recovery_episode_decoded(180.0, 77_001, 55);
            sink.record_picture_recovery_episode_response_observed(
                190.0,
                Some(77_101),
                true,
                "ownerFrameAdvanced",
                Some(12),
                None,
                false,
                false,
            );
            sink.record_pending_displayed_idr_rtp(77_001);
        }
        seed_decoder_reference_sync_for_pending_idr(&runtime_stats, 77_001, 180.0);

        runtime_port.update_host_video_present_metrics(XbxEngineHostVideoPresentMetrics {
            latest_host_present_time_ms: Some(210.0),
            host_frame_present_epoch: 1,
            present_fps: 30.0,
            last_displayed_frame_seq: Some(55),
            last_displayed_frame_rtp_timestamp: Some(77_001),
            last_displayed_at_ms: Some(210.0),
            ..Default::default()
        });

        let snapshot = runtime_stats.lock().expect("runtime stats lock").clone();
        assert_eq!(snapshot.video_anchor_clean_epoch, Some(1));
        assert_eq!(snapshot.recovery_displayed_idr_rtp, Some(77_001));
    }

    #[test]
    fn host_present_without_pending_idr_does_not_establish_fresh_anchor_after_episode_advances() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let render_state = Arc::new(Mutex::new(XbxRenderState::default()));
        let media = Arc::new(Mutex::new(RtcMediaService::default()));
        let (tx, _rx) = mpsc::sync_channel(1);
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

        {
            let sink = crate::runtime_stats_sink::RuntimeStatsSink::new(runtime_stats.clone());
            sink.begin_transport_recovery_episode(10.0);
            sink.record_picture_recovery_episode_requested(
                88,
                Some("receiverWaitingKeyframe".to_string()),
                100.0,
                None,
            );
            sink.record_picture_recovery_episode_sent("pli", 120.0, Some(300.0));
            sink.record_picture_recovery_episode_response_observed(
                150.0,
                Some(77_001),
                true,
                "firstAcceptedIdr",
                Some(11),
                None,
                false,
                false,
            );
            sink.record_picture_recovery_episode_decoded(180.0, 77_001, 55);
            sink.update(|stats| {
                stats.latest_keyframe_request_episode =
                    Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                        episode_id: 89,
                        request_reason: Some("receiverWaitingKeyframe".to_string()),
                        status: "waiting-response".to_string(),
                        requested_at_ms: 205.0,
                        ..Default::default()
                    });
            });
        }

        runtime_port.update_host_video_present_metrics(XbxEngineHostVideoPresentMetrics {
            latest_host_present_time_ms: Some(210.0),
            host_frame_present_epoch: 1,
            present_fps: 30.0,
            last_displayed_frame_seq: Some(55),
            last_displayed_frame_rtp_timestamp: Some(77_001),
            last_displayed_at_ms: Some(210.0),
            ..Default::default()
        });

        let snapshot = runtime_stats.lock().expect("runtime stats lock").clone();
        assert_eq!(snapshot.video_anchor_clean_epoch, None);
        let latest_episode = snapshot
            .latest_keyframe_request_episode
            .expect("latest keyframe request episode should exist");
        assert_eq!(latest_episode.episode_id, 89);
        assert_eq!(latest_episode.response_verdict, None);
    }

    #[test]
    fn host_present_of_decoded_recovery_owner_does_not_commit_clean_anchor_without_submission() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let render_state = Arc::new(Mutex::new(XbxRenderState::default()));
        let media = Arc::new(Mutex::new(RtcMediaService::default()));
        let (tx, _rx) = mpsc::sync_channel(1);
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

        {
            let sink = crate::runtime_stats_sink::RuntimeStatsSink::new(runtime_stats.clone());
            sink.begin_transport_recovery_episode(10.0);
            sink.record_picture_recovery_episode_requested(
                188,
                Some("receiverWaitingKeyframe".to_string()),
                100.0,
                None,
            );
            sink.record_picture_recovery_episode_sent("pli", 120.0, Some(300.0));
            sink.record_picture_recovery_episode_response_observed(
                150.0,
                Some(88_001),
                true,
                "firstAcceptedIdr",
                Some(11),
                None,
                false,
                false,
            );
            sink.record_picture_recovery_episode_decoded(180.0, 88_001, 66);
        }

        runtime_port.update_host_video_present_metrics(XbxEngineHostVideoPresentMetrics {
            latest_host_present_time_ms: Some(210.0),
            host_frame_present_epoch: 1,
            present_fps: 30.0,
            last_displayed_frame_seq: Some(66),
            last_displayed_frame_rtp_timestamp: Some(88_001),
            last_displayed_at_ms: Some(210.0),
            ..Default::default()
        });

        let snapshot = runtime_stats.lock().expect("runtime stats lock").clone();
        assert_eq!(snapshot.video_anchor_clean_epoch, None);
        assert_eq!(snapshot.video_anchor_clean_observed_at_ms, None);
        assert_eq!(snapshot.recovery_fresh_anchor_recovered_at_ms, None);
        assert_eq!(snapshot.video_anchor_clean_source_event, None);
        let episode = snapshot
            .latest_keyframe_request_episode
            .expect("keyframe request episode should exist");
        assert_eq!(episode.status, "decoded");
        assert_eq!(episode.response_verdict.as_deref(), Some("on-time"));
    }

    #[test]
    fn snapshot_runtime_stats_preserves_host_visibility_stall() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let render_state = Arc::new(Mutex::new(XbxRenderState::default()));
        let media = Arc::new(Mutex::new(RtcMediaService::default()));
        let (tx, _rx) = mpsc::sync_channel(1);
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
        let now_ms = crate::transport::rtc::stats::now_ms_f64();

        runtime_port.update_host_video_present_metrics(XbxEngineHostVideoPresentMetrics {
            latest_host_submit_time_ms: Some(now_ms - 50.0),
            latest_host_submit_rtp_timestamp: Some(90_001),
            latest_host_present_time_ms: Some(now_ms - 1_900.0),
            host_view_generation: 3,
            latest_host_view_created_at_ms: Some(now_ms - 40.0),
            host_mailbox_submit_epoch: 11,
            host_display_tick_epoch: 120,
            host_frame_present_epoch: 115,
            host_mailbox_enqueue_count_total: 120,
            last_displayed_frame_seq: Some(77),
            last_displayed_frame_rtp_timestamp: Some(88_888),
            last_displayed_at_ms: Some(now_ms - 1_900.0),
            ..Default::default()
        });

        let snapshot = runtime_port.snapshot_runtime_stats();
        assert_eq!(snapshot.video_renderer_stalled, Some(true));
        assert_eq!(snapshot.host_view_generation, 3);
        assert_eq!(
            snapshot.latest_video_host_submit_rtp_timestamp,
            Some(90_001)
        );
    }

    #[test]
    fn host_visibility_stall_preserves_displayed_idr_clean_anchor() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let render_state = Arc::new(Mutex::new(XbxRenderState::default()));
        let media = Arc::new(Mutex::new(RtcMediaService::default()));
        let (tx, _rx) = mpsc::sync_channel(1);
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
        let now_ms = crate::transport::rtc::stats::now_ms_f64();

        {
            let mut stats = runtime_stats.lock().expect("runtime stats lock");
            stats.transport_recovery_epoch = 7;
            stats.video_anchor_clean_epoch = Some(7);
            stats.video_anchor_clean_observed_at_ms = Some(now_ms - 2_000.0);
            stats.video_anchor_clean_source_event = Some("displayed-idr".to_string());
            stats.recovery_displayed_idr_rtp = Some(77_001);
            stats.recovery_displayed_idr_at_ms = Some(now_ms - 2_000.0);
            stats.recovery_fresh_anchor_recovered_at_ms = Some(now_ms - 2_000.0);
            stats.latest_keyframe_request_episode =
                Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                    episode_id: 11,
                    request_reason: Some("receiverWaitingKeyframe".to_string()),
                    status: "succeeded".to_string(),
                    requested_at_ms: now_ms - 2_400.0,
                    sent_at_ms: Some(now_ms - 2_350.0),
                    first_video_packet_at_ms: Some(now_ms - 2_300.0),
                    first_video_packet_rtp_timestamp: Some(77_001),
                    first_video_packet_is_keyframe: Some(true),
                    first_keyframe_packet_at_ms: Some(now_ms - 2_300.0),
                    first_keyframe_decoded_at_ms: Some(now_ms - 2_250.0),
                    response_rtp_timestamp: Some(77_001),
                    response_frame_seq: Some(2),
                    response_verdict: Some("cleanAnchorCommitted".to_string()),
                    lifecycle_phase: Some("success".to_string()),
                    retired_at_ms: Some(now_ms - 2_000.0),
                    ..Default::default()
                });
            stats.latest_h264_inspection_observation =
                Some(crate::XbxEngineH264InspectionObservation {
                    observation_id: 999,
                    frame_rtp_timestamp: Some(88_002),
                    committed_sps_present: true,
                    committed_pps_present: true,
                    delta_continuation_ready: true,
                    bootstrap_ready: false,
                    bootstrap_reject_reason: Some("bootstrapMissingIdr".to_string()),
                    continuation_verdict: Some("receiverLocalContinuation".to_string()),
                    admission_accepted: true,
                    observed_at_ms: now_ms - 10.0,
                    ..Default::default()
                });
        }

        runtime_port.update_host_video_present_metrics(XbxEngineHostVideoPresentMetrics {
            latest_host_submit_time_ms: Some(now_ms - 50.0),
            latest_host_submit_rtp_timestamp: Some(90_001),
            latest_host_present_time_ms: Some(now_ms - 2_100.0),
            host_view_generation: 3,
            latest_host_view_created_at_ms: Some(now_ms - 100.0),
            host_mailbox_submit_epoch: 11,
            host_display_tick_epoch: 120,
            host_frame_present_epoch: 115,
            host_mailbox_enqueue_count_total: 120,
            last_displayed_frame_seq: None,
            last_displayed_frame_rtp_timestamp: None,
            last_displayed_at_ms: None,
            ..Default::default()
        });

        let snapshot = runtime_stats.lock().expect("runtime stats lock").clone();
        assert_eq!(snapshot.video_anchor_clean_epoch, Some(7));
        assert_eq!(
            snapshot.video_anchor_clean_source_event.as_deref(),
            Some("displayed-idr")
        );
        assert_eq!(snapshot.recovery_displayed_idr_rtp, Some(77_001));
        assert_eq!(snapshot.video_renderer_stalled, Some(true));
    }
}
