use std::sync::Arc;

use xbxengine::{XbxEngineRenderFrame, XbxEngineRenderPixelData};

use super::{
    HostCadencePhase, HostCadenceTelemetry, ScheduledFrameSlot, ScheduledFrameSubmitOutcome,
};

fn mk_frame(frame_seq: u64) -> XbxEngineRenderFrame {
    XbxEngineRenderFrame {
        width: 1920,
        height: 1080,
        frame_seq,
        rendered_at_ms: 1_000.0,
        rtp_timestamp: Some(frame_seq as u32),
        is_keyframe: frame_seq == 1,
        frame_recovery_disposition: Some("repairing".to_string()),
        frame_unrecoverable_reason: None,
        pixel_data: XbxEngineRenderPixelData::Rgba {
            bytes: Arc::from(vec![0_u8; 4].into_boxed_slice()),
        },
    }
}

#[test]
fn begin_media_epoch_clears_presented_history_without_stopping_render_loop() {
    let mut slot = ScheduledFrameSlot::default();
    let mut telemetry = HostCadenceTelemetry::default();

    match slot.submit_frame(&mk_frame(223), 1_010.0, &mut telemetry) {
        ScheduledFrameSubmitOutcome::Accepted { .. } => {}
        other => panic!("expected accepted frame, got {other:?}"),
    }
    match slot.take_ready_frame(1_020.0, &mut telemetry) {
        super::ScheduledFrameTakeOutcome::Ready(frame) => assert_eq!(frame.frame_seq, 223),
        other => panic!("expected ready frame, got {other:?}"),
    }

    slot.render_loop_started = true;
    slot.begin_media_epoch();
    assert!(slot.render_loop_started);

    match slot.submit_frame(&mk_frame(26), 1_030.0, &mut telemetry) {
        ScheduledFrameSubmitOutcome::Accepted { frame_seq, .. } => {
            assert_eq!(frame_seq, 26)
        }
        other => panic!("expected new epoch frame to be accepted, got {other:?}"),
    }
}

#[test]
fn cadence_epoch_and_phase_progress_with_ticks_and_presents() {
    let mut telemetry = HostCadenceTelemetry::default();
    assert_eq!(telemetry.display_tick_epoch(), 0);
    assert_eq!(telemetry.present_epoch(), 0);
    assert_eq!(telemetry.cadence_phase(), HostCadencePhase::Idle);

    telemetry.record_display_tick(1_000.0);
    telemetry.record_display_tick(1_016.0);
    assert_eq!(telemetry.display_tick_epoch(), 2);
    assert_eq!(telemetry.present_epoch(), 0);
    assert_eq!(telemetry.cadence_phase(), HostCadencePhase::Priming);

    telemetry.record_present(1_018.0);
    assert_eq!(telemetry.present_epoch(), 1);
    assert_eq!(telemetry.cadence_phase(), HostCadencePhase::Steady);

    telemetry.record_no_pending_take();
    assert_eq!(telemetry.cadence_phase(), HostCadencePhase::Starved);
    telemetry.clear_no_pending_streak();
    assert_eq!(telemetry.cadence_phase(), HostCadencePhase::Steady);
}

#[test]
fn no_pending_before_first_present_stays_in_priming() {
    let mut telemetry = HostCadenceTelemetry::default();

    telemetry.record_display_tick(1_000.0);
    telemetry.record_display_tick(1_016.0);
    telemetry.record_no_pending_take();
    telemetry.record_no_pending_take();

    assert_eq!(telemetry.display_tick_epoch(), 2);
    assert_eq!(telemetry.present_epoch(), 0);
    assert_eq!(telemetry.no_pending_streak, 2);
    assert_eq!(telemetry.cadence_phase(), HostCadencePhase::Priming);

    telemetry.clear_no_pending_streak();
    assert_eq!(telemetry.cadence_phase(), HostCadencePhase::Priming);
}

#[test]
fn intermittent_no_pending_between_presents_returns_from_starved_to_steady() {
    let mut telemetry = HostCadenceTelemetry::default();

    telemetry.record_display_tick(1_000.0);
    telemetry.record_display_tick(1_016.0);
    telemetry.record_present(1_018.0);
    assert_eq!(telemetry.cadence_phase(), HostCadencePhase::Steady);

    telemetry.record_no_pending_take();
    telemetry.record_no_pending_take();
    assert_eq!(telemetry.no_pending_streak, 2);
    assert_eq!(telemetry.cadence_phase(), HostCadencePhase::Starved);

    // 模拟 trace 中的短促 no-pending 抖动：下一帧及时 present 后应回到 steady，
    // 不应把短窗饥饿粘滞成持续的 starved。
    telemetry.clear_no_pending_streak();
    telemetry.record_display_tick(1_033.0);
    telemetry.record_present(1_034.0);

    assert_eq!(telemetry.no_pending_streak, 0);
    assert_eq!(telemetry.present_epoch(), 2);
    assert_eq!(telemetry.cadence_phase(), HostCadencePhase::Steady);
}
