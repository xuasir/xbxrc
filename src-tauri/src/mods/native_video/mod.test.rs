use super::{
    clear_host_present_tick_dispatch, finish_host_present_tick_dispatch,
    request_host_present_tick_dispatch, reset_viewport_present_runtime_state,
    resolve_display_layer_layout, resolve_host_timing_record_policy,
    should_emit_sampled_host_timing, should_reattach_viewport, should_update_scale,
    HostTimingRecordPolicy, MacOsDisplayLayerGravity, MacOsWgpuTelemetry, NativeVideoDisplayState,
    NativeVideoRegistry, NativeVideoViewportState,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[test]
fn scale_update_ignores_tiny_jitter() {
    assert!(!should_update_scale(Some(2.0), Some(2.0005)));
}

#[test]
fn scale_update_detects_real_change() {
    assert!(should_update_scale(Some(2.0), Some(2.01)));
}

#[test]
fn scale_update_detects_presence_change() {
    assert!(should_update_scale(None, Some(2.0)));
    assert!(should_update_scale(Some(2.0), None));
}

#[test]
fn display_layer_layout_stretches_to_full_bounds_for_stretch() {
    let layout = resolve_display_layer_layout([0.0, 0.0, 1920.0, 1080.0], Some("Stretch"), None);
    assert_eq!(layout.frame, [0.0, 0.0, 1920.0, 1080.0]);
    assert_eq!(layout.gravity, MacOsDisplayLayerGravity::Resize);
}

#[test]
fn display_layer_layout_uses_fixed_ratio_box_for_aspect_formats() {
    let layout =
        resolve_display_layer_layout([0.0, 0.0, 1920.0, 1080.0], Some("4:3"), Some((1280, 720)));
    assert_eq!(layout.frame, [240.0, 0.0, 1440.0, 1080.0]);
    assert_eq!(layout.gravity, MacOsDisplayLayerGravity::Resize);
}

#[test]
fn registry_persists_display_state_for_future_presenter_attach() {
    let mut registry = NativeVideoRegistry::default();
    registry.apply_display_state(
        "stream-page-video",
        NativeVideoDisplayState::from_video_format(Some("Zoom")),
    );

    let viewport = registry
        .snapshot("stream-page-video")
        .expect("viewport should exist");
    assert_eq!(viewport.video_format.as_deref(), Some("Zoom"));
}

#[test]
fn wgpu_no_pending_streak_accumulates_and_tracks_max() {
    let mut telemetry = MacOsWgpuTelemetry::default();
    telemetry.record_no_pending_take();
    telemetry.record_no_pending_take();
    telemetry.record_no_pending_take();
    assert_eq!(telemetry.no_pending_take_count_total, 3);
    assert_eq!(telemetry.no_pending_streak, 3);
    assert_eq!(telemetry.no_pending_max_streak, 3);
}

#[test]
fn wgpu_clear_no_pending_streak_keeps_max_value() {
    let mut telemetry = MacOsWgpuTelemetry::default();
    telemetry.record_no_pending_take();
    telemetry.record_no_pending_take();
    telemetry.clear_no_pending_streak();
    telemetry.record_no_pending_take();
    assert_eq!(telemetry.no_pending_take_count_total, 3);
    assert_eq!(telemetry.no_pending_streak, 1);
    assert_eq!(telemetry.no_pending_max_streak, 2);
}

#[test]
fn wgpu_reset_frame_slot_clears_no_pending_counters() {
    let mut telemetry = MacOsWgpuTelemetry::default();
    telemetry.record_no_pending_take();
    telemetry.record_no_pending_take();
    telemetry.reset_frame_slot();
    assert_eq!(telemetry.no_pending_take_count_total, 0);
    assert_eq!(telemetry.no_pending_streak, 0);
    assert_eq!(telemetry.no_pending_max_streak, 0);
}

#[test]
fn viewport_reattach_only_happens_when_attach_inputs_change() {
    assert!(!should_reattach_viewport(false, false, false));
    assert!(should_reattach_viewport(true, false, false));
    assert!(should_reattach_viewport(false, true, false));
    assert!(should_reattach_viewport(false, false, true));
}

#[test]
fn host_timing_record_policy_keeps_submit_and_present_path_always_on() {
    assert_eq!(
        resolve_host_timing_record_policy("hostMailboxAccepted"),
        HostTimingRecordPolicy::Always
    );
    assert_eq!(
        resolve_host_timing_record_policy("hostMailboxRetainedDisplayed"),
        HostTimingRecordPolicy::Always
    );
    assert_eq!(
        resolve_host_timing_record_policy("hostMailboxIdle"),
        HostTimingRecordPolicy::Sampled
    );
    assert_eq!(
        resolve_host_timing_record_policy("prepare_sample_ready"),
        HostTimingRecordPolicy::Always
    );
    assert_eq!(
        resolve_host_timing_record_policy("hostFramePresented"),
        HostTimingRecordPolicy::Always
    );
    assert_eq!(
        resolve_host_timing_record_policy("tick_total"),
        HostTimingRecordPolicy::Sampled
    );
    assert_eq!(
        resolve_host_timing_record_policy("run_on_main_thread_delay"),
        HostTimingRecordPolicy::Sampled
    );
    assert_eq!(
        resolve_host_timing_record_policy("hostMailboxSubmitGap"),
        HostTimingRecordPolicy::Sampled
    );
    assert_eq!(
        resolve_host_timing_record_policy("hostMailboxUpdateFailed"),
        HostTimingRecordPolicy::Sampled
    );
    assert_eq!(
        resolve_host_timing_record_policy("present_tick_blocked"),
        HostTimingRecordPolicy::Sampled
    );
    assert_eq!(
        resolve_host_timing_record_policy("first_present"),
        HostTimingRecordPolicy::Always
    );
}

#[test]
fn sampled_host_timing_requires_min_interval() {
    assert!(should_emit_sampled_host_timing(None, 1_000.0));
    assert!(!should_emit_sampled_host_timing(Some(1_000.0), 1_500.0));
    assert!(should_emit_sampled_host_timing(Some(1_000.0), 2_000.0));
}

#[test]
fn reset_viewport_present_runtime_state_clears_display_runtime_metrics() {
    let mut viewport = NativeVideoViewportState {
        latest_frame_seq: Some(7),
        latest_frame_width: Some(1920),
        latest_frame_height: Some(1080),
        latest_renderer_frame_time_ms: Some(1_200.0),
        present_count_total: 10,
        last_present_kind: Some("rgba".to_string()),
        latest_host_present_time_ms: Some(1_210.0),
        host_present_fps: 60.0,
        host_mailbox_enqueue_count_total: 20,
        host_mailbox_drop_count_total: 3,
        host_mailbox_overwrite_count_total: 2,
        host_no_pending_take_count_total: 8,
        host_no_pending_streak: 4,
        host_no_pending_max_streak: 9,
        host_display_tick_epoch: 123,
        host_frame_present_epoch: 88,
        host_cadence_phase: Some("steady".to_string()),
        host_display_interval_ms: Some(16.6),
        host_frame_age_budget_ms: Some(32.0),
        ..Default::default()
    };

    reset_viewport_present_runtime_state(&mut viewport);

    assert_eq!(viewport.latest_frame_seq, None);
    assert_eq!(viewport.latest_renderer_frame_time_ms, None);
    assert_eq!(viewport.present_count_total, 0);
    assert_eq!(viewport.latest_host_present_time_ms, None);
    assert_eq!(viewport.host_mailbox_enqueue_count_total, 0);
    assert_eq!(viewport.host_display_tick_epoch, 0);
    assert_eq!(viewport.host_frame_present_epoch, 0);
    assert_eq!(viewport.host_cadence_phase, None);
}

#[test]
fn host_stall_presenter_reset_preserves_viewport_present_metrics() {
    let mut registry = NativeVideoRegistry::default();
    registry.viewports.insert(
        "stream-page-video".to_string(),
        NativeVideoViewportState {
            viewport_id: "stream-page-video".to_string(),
            latest_frame_seq: Some(7),
            latest_frame_width: Some(1920),
            latest_frame_height: Some(1080),
            latest_renderer_frame_time_ms: Some(1_200.0),
            present_count_total: 10,
            last_present_kind: Some("rgba".to_string()),
            latest_host_present_time_ms: Some(1_210.0),
            host_present_fps: 60.0,
            host_mailbox_enqueue_count_total: 20,
            host_mailbox_drop_count_total: 3,
            host_mailbox_overwrite_count_total: 2,
            host_no_pending_take_count_total: 8,
            host_no_pending_streak: 4,
            host_no_pending_max_streak: 9,
            host_display_tick_epoch: 123,
            host_frame_present_epoch: 88,
            host_cadence_phase: Some("starved".to_string()),
            last_displayed_frame_seq: Some(6),
            last_displayed_frame_rtp_timestamp: Some(777),
            last_displayed_at_ms: Some(1_211.0),
            host_display_interval_ms: Some(16.6),
            host_frame_age_budget_ms: Some(32.0),
            ..Default::default()
        },
    );

    registry.reset_presenter_for_host_stall_recovery("stream-page-video");

    let viewport = registry
        .snapshot("stream-page-video")
        .expect("viewport should exist");
    assert_eq!(viewport.latest_frame_seq, Some(7));
    assert_eq!(viewport.latest_renderer_frame_time_ms, Some(1_200.0));
    assert_eq!(viewport.present_count_total, 10);
    assert_eq!(viewport.latest_host_present_time_ms, Some(1_210.0));
    assert_eq!(viewport.host_mailbox_enqueue_count_total, 20);
    assert_eq!(viewport.host_display_tick_epoch, 123);
    assert_eq!(viewport.host_frame_present_epoch, 88);
    assert_eq!(viewport.host_cadence_phase.as_deref(), Some("starved"));
    assert_eq!(viewport.last_displayed_frame_seq, Some(6));
    assert_eq!(viewport.last_displayed_at_ms, Some(1_211.0));
}

#[test]
fn host_present_tick_dispatch_rearms_followup_while_current_tick_is_pending() {
    let pending = Arc::new(AtomicBool::new(false));
    let rerun_requested = Arc::new(AtomicBool::new(false));

    assert!(request_host_present_tick_dispatch(
        &pending,
        &rerun_requested
    ));
    assert!(pending.load(Ordering::Relaxed));
    assert!(!rerun_requested.load(Ordering::Relaxed));

    assert!(!request_host_present_tick_dispatch(
        &pending,
        &rerun_requested
    ));
    assert!(pending.load(Ordering::Relaxed));
    assert!(rerun_requested.load(Ordering::Relaxed));

    assert!(finish_host_present_tick_dispatch(
        &pending,
        &rerun_requested
    ));
    assert!(pending.load(Ordering::Relaxed));
    assert!(!rerun_requested.load(Ordering::Relaxed));

    assert!(!finish_host_present_tick_dispatch(
        &pending,
        &rerun_requested
    ));
    assert!(!pending.load(Ordering::Relaxed));
}

#[test]
fn clear_host_present_tick_dispatch_resets_pending_and_followup_state() {
    let pending = Arc::new(AtomicBool::new(true));
    let rerun_requested = Arc::new(AtomicBool::new(true));

    clear_host_present_tick_dispatch(&pending, &rerun_requested);

    assert!(!pending.load(Ordering::Relaxed));
    assert!(!rerun_requested.load(Ordering::Relaxed));
}
