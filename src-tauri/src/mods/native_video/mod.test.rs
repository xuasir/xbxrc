use super::types::{VideoPlatformCapabilities, VideoPlatformKind};
use super::{
    clear_host_present_tick_dispatch, finish_host_present_tick_dispatch,
    host_present_tick_dispatch_diagnostics, request_host_present_tick_dispatch,
    rescue_stale_host_present_tick_dispatch, reset_viewport_present_runtime_state,
    resolve_display_layer_layout, resolve_host_timing_record_policy,
    should_emit_sampled_host_timing, should_reattach_viewport, should_update_scale,
    take_wgpu_scheduled_frame, HostPresentTickGuard, HostTimingRecordPolicy,
    MacOsDisplayLayerGravity, MacOsWgpuTelemetry, NativeVideoDisplayState, NativeVideoRegistry,
    NativeVideoViewportState,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use xbxengine::{XbxEngineRenderFrame, XbxEngineRenderPixelData};

use super::scheduling::{ScheduledFrameSlot, ScheduledFrameTakeOutcome};

fn rgba_frame(frame_seq: u64, width: u32, height: u32) -> XbxEngineRenderFrame {
    XbxEngineRenderFrame {
        width,
        height,
        frame_seq,
        rendered_at_ms: 1_000.0,
        rtp_timestamp: Some(frame_seq as u32),
        recovery_epoch_tag: None,
        recovery_owner_rtp_timestamp: None,
        is_keyframe: frame_seq == 1,
        frame_recovery_disposition: None,
        frame_unrecoverable_reason: None,
        presentation_value_role: None,
        pixel_data: XbxEngineRenderPixelData::Rgba {
            bytes: Arc::from(vec![0_u8; 4].into_boxed_slice()),
        },
    }
}

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
fn registry_records_wgpu_effect_passthrough_diagnostics() {
    let mut registry = NativeVideoRegistry::default();
    registry.platform_capabilities = VideoPlatformCapabilities {
        platform: VideoPlatformKind::Windows,
        supports_native_direct: false,
        supports_gpu_direct: true,
        supports_wgpu_effects: true,
    };

    assert!(registry.present_frame(
        "stream-page-video",
        Some("wgpu:stream-page-video"),
        &rgba_frame(7, 1280, 720),
    ));

    let viewport = registry
        .snapshot("stream-page-video")
        .expect("viewport should exist");
    assert_eq!(viewport.effect_pipeline_kind.as_deref(), Some("wgpu"));
    assert!(!viewport.effect_active);
    assert_eq!(
        viewport.effect_fallback_reason.as_deref(),
        Some("passthroughPendingEffectRenderer")
    );
    assert_eq!(viewport.latest_effect_input_width, Some(1280));
    assert_eq!(viewport.latest_effect_input_height, Some(720));
    assert_eq!(viewport.latest_effect_output_width, Some(1280));
    assert_eq!(viewport.latest_effect_output_height, Some(720));
    assert!(viewport.latest_effect_render_cost_ms.is_some());
}

#[test]
fn wgpu_scheduled_frame_take_records_display_tick_and_ready_diagnostics() {
    let frame_slot = Arc::new(Mutex::new(ScheduledFrameSlot::default()));
    let telemetry = Arc::new(Mutex::new(MacOsWgpuTelemetry::default()));
    {
        let mut slot = frame_slot.lock().expect("slot lock should succeed");
        let mut telemetry_state = telemetry.lock().expect("telemetry lock should succeed");
        let _ = slot.submit_frame(&rgba_frame(11, 1280, 720), 1_010.0, &mut telemetry_state);
    }

    let take = take_wgpu_scheduled_frame(&frame_slot, &telemetry, false, 1_020.0)
        .expect("take should succeed");

    match take.outcome {
        ScheduledFrameTakeOutcome::Ready(frame) => assert_eq!(frame.frame_seq, 11),
        other => panic!("expected ready frame, got {other:?}"),
    }
    assert_eq!(take.telemetry_diag.display_tick_epoch, 1);
    assert_eq!(take.slot_diag.displayed_frame_seq, Some(11));
    assert_eq!(
        take.slot_diag.displayed_view_epoch,
        take.slot_diag.view_epoch
    );
    assert_eq!(take.slot_diag.pending_queue_depth, 0);
}

#[test]
fn wgpu_scheduled_frame_take_replays_displayed_frame_for_view_epoch_change() {
    let frame_slot = Arc::new(Mutex::new(ScheduledFrameSlot::default()));
    let telemetry = Arc::new(Mutex::new(MacOsWgpuTelemetry::default()));
    {
        let mut slot = frame_slot.lock().expect("slot lock should succeed");
        let mut telemetry_state = telemetry.lock().expect("telemetry lock should succeed");
        let _ = slot.submit_frame(&rgba_frame(12, 1280, 720), 1_010.0, &mut telemetry_state);
        let _ = slot.take_ready_frame(1_020.0, &mut telemetry_state);
    }

    let take = take_wgpu_scheduled_frame(&frame_slot, &telemetry, true, 1_040.0)
        .expect("take should succeed");

    match take.outcome {
        ScheduledFrameTakeOutcome::Ready(frame) => assert_eq!(frame.frame_seq, 12),
        other => panic!("expected displayed replay after view epoch change, got {other:?}"),
    }
    assert_eq!(take.telemetry_diag.display_tick_epoch, 1);
    assert_eq!(take.slot_diag.view_epoch, 1);
    assert_eq!(take.slot_diag.displayed_view_epoch, 1);
    assert_eq!(take.slot_diag.displayed_frame_seq, Some(12));
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
fn host_timing_record_policy_samples_high_frequency_present_path() {
    assert_eq!(
        resolve_host_timing_record_policy("hostMailboxAccepted"),
        HostTimingRecordPolicy::Sampled
    );
    assert_eq!(
        resolve_host_timing_record_policy("hostMailboxRetainedDisplayed"),
        HostTimingRecordPolicy::Sampled
    );
    assert_eq!(
        resolve_host_timing_record_policy("hostMailboxTakeDecision"),
        HostTimingRecordPolicy::Sampled
    );
    assert_eq!(
        resolve_host_timing_record_policy("hostMailboxIdle"),
        HostTimingRecordPolicy::Sampled
    );
    assert_eq!(
        resolve_host_timing_record_policy("prepare_sample_ready"),
        HostTimingRecordPolicy::Sampled
    );
    assert_eq!(
        resolve_host_timing_record_policy("hostFramePresented"),
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
        resolve_host_timing_record_policy("hostMailboxSubmitGap"),
        HostTimingRecordPolicy::Sampled
    );
    assert_eq!(
        resolve_host_timing_record_policy("present_tick_dispatch_coalesced"),
        HostTimingRecordPolicy::Sampled
    );
    assert_eq!(
        resolve_host_timing_record_policy("present_tick_immediate_deferred"),
        HostTimingRecordPolicy::Sampled
    );
    assert_eq!(
        resolve_host_timing_record_policy("present_tick_rerun"),
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
    assert!(should_emit_sampled_host_timing(Some(1_000.0), 3_000.0));
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
        effect_pipeline_kind: Some("wgpu".to_string()),
        effect_active: true,
        effect_fallback_reason: Some("test".to_string()),
        latest_effect_render_cost_ms: Some(2.0),
        latest_effect_input_width: Some(1280),
        latest_effect_input_height: Some(720),
        latest_effect_output_width: Some(1920),
        latest_effect_output_height: Some(1080),
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
    assert_eq!(viewport.effect_pipeline_kind, None);
    assert!(!viewport.effect_active);
    assert_eq!(viewport.effect_fallback_reason, None);
    assert_eq!(viewport.latest_effect_render_cost_ms, None);
    assert_eq!(viewport.latest_effect_output_width, None);
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
fn host_present_tick_guard_releases_pending_on_early_exit() {
    let pending = Arc::new(AtomicBool::new(true));
    let rerun_requested = Arc::new(AtomicBool::new(false));

    {
        let _guard = HostPresentTickGuard::new(pending.clone(), rerun_requested.clone());
    }

    assert!(!pending.load(Ordering::Relaxed));
    assert!(!rerun_requested.load(Ordering::Relaxed));
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

#[test]
fn stale_host_present_tick_dispatch_rescue_releases_stuck_pending() {
    let pending = Arc::new(AtomicBool::new(true));
    let rerun_requested = Arc::new(AtomicBool::new(true));
    let telemetry = Arc::new(Mutex::new(MacOsWgpuTelemetry::default()));
    telemetry
        .lock()
        .expect("lock telemetry")
        .record_present(1_000.0);

    assert!(rescue_stale_host_present_tick_dispatch(
        &pending,
        &rerun_requested,
        &telemetry,
        1_300.0,
    ));
    assert!(!pending.load(Ordering::Relaxed));
    assert!(!rerun_requested.load(Ordering::Relaxed));
}

#[test]
fn fresh_host_present_tick_dispatch_rescue_keeps_pending() {
    let pending = Arc::new(AtomicBool::new(true));
    let rerun_requested = Arc::new(AtomicBool::new(true));
    let telemetry = Arc::new(Mutex::new(MacOsWgpuTelemetry::default()));
    telemetry
        .lock()
        .expect("lock telemetry")
        .record_present(1_000.0);

    assert!(!rescue_stale_host_present_tick_dispatch(
        &pending,
        &rerun_requested,
        &telemetry,
        1_100.0,
    ));
    assert!(pending.load(Ordering::Relaxed));
    assert!(rerun_requested.load(Ordering::Relaxed));
}

#[test]
fn host_present_tick_dispatch_diagnostics_reports_rescue_context() {
    let pending = Arc::new(AtomicBool::new(true));
    let rerun_requested = Arc::new(AtomicBool::new(true));
    let telemetry = Arc::new(Mutex::new(MacOsWgpuTelemetry::default()));
    {
        let mut telemetry = telemetry.lock().expect("lock telemetry");
        telemetry.record_display_tick(980.0);
        telemetry.record_display_tick(996.0);
        telemetry.record_enqueue(1_010.0);
        telemetry.record_present(1_000.0);
    }

    let details = host_present_tick_dispatch_diagnostics(
        "displayLink",
        &pending,
        &rerun_requested,
        &telemetry,
        1_300.0,
    );

    assert_eq!(details["source"], "displayLink");
    assert_eq!(details["pendingBefore"], true);
    assert_eq!(details["rerunBefore"], true);
    assert_eq!(details["presentAgeMs"], 300.0);
    assert_eq!(details["submitAgeMs"], 290.0);
    assert_eq!(details["hostDisplayTickEpoch"], 2);
    assert_eq!(details["hostFramePresentEpoch"], 1);
    assert_eq!(details["hostCadencePhase"], "steady");
    assert_eq!(details["hostMailboxEnqueueCountTotal"], 1);
}
