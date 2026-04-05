use super::{should_reattach_viewport, should_update_scale, MacOsWgpuTelemetry};

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
