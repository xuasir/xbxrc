use super::{
    reset_viewport_present_runtime_state, resolve_host_timing_record_policy,
    should_emit_sampled_host_timing, should_reattach_viewport, should_update_scale,
    HostTimingRecordPolicy, MacOsWgpuTelemetry, NativeVideoViewportState,
};

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
fn host_timing_record_policy_marks_hot_stages_as_sampled() {
    assert_eq!(
        resolve_host_timing_record_policy("frame_submit"),
        HostTimingRecordPolicy::Sampled
    );
    assert_eq!(
        resolve_host_timing_record_policy("prepare_sample_ready"),
        HostTimingRecordPolicy::Sampled
    );
    assert_eq!(
        resolve_host_timing_record_policy("sample_presented"),
        HostTimingRecordPolicy::Sampled
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
        host_present_enqueue_count_total: 20,
        host_present_drop_count_total: 3,
        host_present_overwrite_count_total: 2,
        host_no_pending_take_count_total: 8,
        host_no_pending_streak: 4,
        host_no_pending_max_streak: 9,
        host_display_tick_epoch: 123,
        host_present_epoch: 88,
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
    assert_eq!(viewport.host_present_enqueue_count_total, 0);
    assert_eq!(viewport.host_display_tick_epoch, 0);
    assert_eq!(viewport.host_present_epoch, 0);
    assert_eq!(viewport.host_cadence_phase, None);
}
