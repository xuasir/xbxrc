use std::sync::Arc;

use xbxengine::{XbxEngineRenderFrame, XbxEngineRenderPixelData};

use super::{
    HostCadencePhase, HostCadenceTelemetry, ScheduledFrameSlot, ScheduledFrameSubmitOutcome,
    ScheduledFrameTakeOutcome,
};

fn mk_frame(frame_seq: u64, rendered_at_ms: f64) -> XbxEngineRenderFrame {
    XbxEngineRenderFrame {
        width: 1920,
        height: 1080,
        frame_seq,
        rendered_at_ms,
        rtp_timestamp: Some(frame_seq as u32),
        recovery_epoch_tag: None,
        recovery_owner_rtp_timestamp: None,
        is_keyframe: frame_seq == 1,
        frame_recovery_disposition: Some("repairing".to_string()),
        frame_unrecoverable_reason: None,
        presentation_value_role: Some("steady-continuation".to_string()),
        pixel_data: XbxEngineRenderPixelData::Rgba {
            bytes: Arc::from(vec![0_u8; 4].into_boxed_slice()),
        },
    }
}

#[test]
fn pending_slot_keeps_latest_arrival() {
    let mut slot = ScheduledFrameSlot::default();
    let mut telemetry = HostCadenceTelemetry::default();

    match slot.submit_frame(&mk_frame(10, 1_000.0), 1_010.0, &mut telemetry) {
        ScheduledFrameSubmitOutcome::Accepted {
            overwrote_pending, ..
        } => assert!(!overwrote_pending),
        other => panic!("expected first frame accepted, got {other:?}"),
    }
    match slot.submit_frame(&mk_frame(11, 1_016.0), 1_020.0, &mut telemetry) {
        ScheduledFrameSubmitOutcome::Accepted {
            overwrote_pending,
            replaced_frame_seq,
            ..
        } => {
            assert!(overwrote_pending);
            assert_eq!(replaced_frame_seq, Some(10));
        }
        other => panic!("expected second frame to overwrite pending slot, got {other:?}"),
    }

    let snapshot = slot.diagnostics_snapshot();
    assert_eq!(snapshot.pending_frame_seqs, vec![11]);
    assert_eq!(snapshot.pending_queue_depth, 1);
}

#[test]
fn take_ready_frame_with_pending_never_returns_retained() {
    let mut slot = ScheduledFrameSlot::default();
    let mut telemetry = HostCadenceTelemetry::default();
    let _ = slot.submit_frame(&mk_frame(20, 1_000.0), 1_010.0, &mut telemetry);
    match slot.take_ready_frame(1_020.0, &mut telemetry) {
        ScheduledFrameTakeOutcome::Ready(_) => {}
        other => panic!("expected Ready when pending exists, got {other:?}"),
    }
}

#[test]
fn take_ready_frame_presents_latest_pending_frame() {
    let mut slot = ScheduledFrameSlot::default();
    let mut telemetry = HostCadenceTelemetry::default();

    let _ = slot.submit_frame(&mk_frame(20, 1_000.0), 1_010.0, &mut telemetry);
    let _ = slot.submit_frame(&mk_frame(21, 1_016.0), 1_020.0, &mut telemetry);

    match slot.take_ready_frame(1_030.0, &mut telemetry) {
        ScheduledFrameTakeOutcome::Ready(frame) => assert_eq!(frame.frame_seq, 21),
        other => panic!("expected latest pending frame to present, got {other:?}"),
    }
    match slot.take_ready_frame(1_040.0, &mut telemetry) {
        ScheduledFrameTakeOutcome::RetainedDisplayedFrame => {}
        other => panic!("expected displayed frame retention after pending drain, got {other:?}"),
    }
}

#[test]
fn recovery_epoch_rollover_still_allows_rewound_frame_sequence() {
    let mut slot = ScheduledFrameSlot::default();
    let mut telemetry = HostCadenceTelemetry::default();

    let _ = slot.submit_frame(&mk_frame(120, 1_000.0), 1_010.0, &mut telemetry);
    match slot.take_ready_frame(1_020.0, &mut telemetry) {
        ScheduledFrameTakeOutcome::Ready(frame) => assert_eq!(frame.frame_seq, 120),
        other => panic!("expected initial frame present, got {other:?}"),
    }

    let recovery_frame = XbxEngineRenderFrame {
        frame_seq: 5,
        recovery_epoch_tag: Some(1),
        recovery_owner_rtp_timestamp: Some(5),
        is_keyframe: true,
        ..mk_frame(5, 1_030.0)
    };
    match slot.submit_frame(&recovery_frame, 1_031.0, &mut telemetry) {
        ScheduledFrameSubmitOutcome::Accepted { frame_seq, .. } => assert_eq!(frame_seq, 5),
        other => panic!("expected recovery frame accepted after epoch rollover, got {other:?}"),
    }
    match slot.take_ready_frame(1_040.0, &mut telemetry) {
        ScheduledFrameTakeOutcome::Ready(frame) => assert_eq!(frame.frame_seq, 5),
        other => panic!("expected recovery frame present after epoch rollover, got {other:?}"),
    }
}

#[test]
fn older_pending_candidate_is_rejected_and_current_pending_is_preserved() {
    let mut slot = ScheduledFrameSlot::default();
    let mut telemetry = HostCadenceTelemetry::default();

    let newer_pending = XbxEngineRenderFrame {
        frame_seq: 50,
        rtp_timestamp: Some(500),
        recovery_epoch_tag: Some(3),
        recovery_owner_rtp_timestamp: Some(500),
        ..mk_frame(50, 1_020.0)
    };
    match slot.submit_frame(&newer_pending, 1_021.0, &mut telemetry) {
        ScheduledFrameSubmitOutcome::Accepted { .. } => {}
        other => panic!("expected newer pending accepted, got {other:?}"),
    }

    let older_candidate = XbxEngineRenderFrame {
        frame_seq: 49,
        rtp_timestamp: Some(490),
        recovery_epoch_tag: Some(3),
        recovery_owner_rtp_timestamp: Some(490),
        ..mk_frame(49, 1_022.0)
    };
    match slot.submit_frame(&older_candidate, 1_023.0, &mut telemetry) {
        ScheduledFrameSubmitOutcome::RejectedAlreadyPresented { frame_seq, .. } => {
            assert_eq!(frame_seq, 49)
        }
        other => panic!("expected older candidate rejected, got {other:?}"),
    }

    let snapshot = slot.diagnostics_snapshot();
    assert_eq!(snapshot.pending_frame_seqs, vec![50]);
    match slot.take_ready_frame(1_030.0, &mut telemetry) {
        ScheduledFrameTakeOutcome::Ready(frame) => {
            assert_eq!(frame.frame_seq, 50);
            assert_eq!(frame.rtp_timestamp, Some(500));
        }
        other => panic!("expected preserved pending frame to present, got {other:?}"),
    }
}

#[test]
fn lower_recovery_epoch_candidate_is_rejected_behind_pending_epoch() {
    let mut slot = ScheduledFrameSlot::default();
    let mut telemetry = HostCadenceTelemetry::default();

    let higher_epoch_pending = XbxEngineRenderFrame {
        frame_seq: 60,
        recovery_epoch_tag: Some(5),
        recovery_owner_rtp_timestamp: Some(600),
        rtp_timestamp: Some(600),
        ..mk_frame(60, 1_020.0)
    };
    let lower_epoch_candidate = XbxEngineRenderFrame {
        frame_seq: 90,
        recovery_epoch_tag: Some(4),
        recovery_owner_rtp_timestamp: Some(900),
        rtp_timestamp: Some(900),
        ..mk_frame(90, 1_021.0)
    };

    match slot.submit_frame(&higher_epoch_pending, 1_022.0, &mut telemetry) {
        ScheduledFrameSubmitOutcome::Accepted { .. } => {}
        other => panic!("expected higher epoch pending accepted, got {other:?}"),
    }
    match slot.submit_frame(&lower_epoch_candidate, 1_023.0, &mut telemetry) {
        ScheduledFrameSubmitOutcome::RejectedAlreadyPresented { frame_seq, .. } => {
            assert_eq!(frame_seq, 90)
        }
        other => panic!("expected lower epoch candidate rejected, got {other:?}"),
    }

    let snapshot = slot.diagnostics_snapshot();
    assert_eq!(snapshot.pending_frame_seqs, vec![60]);
}

#[test]
fn stale_pending_frame_drops_when_no_displayed_frame_exists() {
    let mut slot = ScheduledFrameSlot::default();
    let mut telemetry = HostCadenceTelemetry::default();

    let stale_frame = mk_frame(30, 1_000.0);
    let _ = slot.submit_frame(&stale_frame, 1_010.0, &mut telemetry);

    match slot.take_ready_frame(1_400.0, &mut telemetry) {
        ScheduledFrameTakeOutcome::DroppedStale {
            frame,
            frame_age_ms,
            ..
        } => {
            assert_eq!(frame.frame_seq, 30);
            assert!(frame_age_ms > 0.0);
        }
        other => panic!("expected stale frame drop, got {other:?}"),
    }
}

#[test]
fn retained_displayed_frame_does_not_enter_starved_or_no_pending_streak() {
    let mut slot = ScheduledFrameSlot::default();
    let mut telemetry = HostCadenceTelemetry::default();

    let _ = slot.submit_frame(&mk_frame(30, 1_000.0), 1_010.0, &mut telemetry);
    match slot.take_ready_frame(1_020.0, &mut telemetry) {
        ScheduledFrameTakeOutcome::Ready(frame) => {
            assert_eq!(frame.frame_seq, 30);
            telemetry.record_present(1_020.0);
        }
        other => panic!("expected initial present, got {other:?}"),
    }
    assert_eq!(telemetry.present_epoch(), 1);
    assert_eq!(telemetry.cadence_phase(), HostCadencePhase::Steady);

    for offset in [0.0, 50.0, 100.0, 150.0] {
        match slot.take_ready_frame(1_030.0 + offset, &mut telemetry) {
            ScheduledFrameTakeOutcome::RetainedDisplayedFrame => {}
            other => panic!("expected retained displayed frame, got {other:?}"),
        }
    }
    assert_eq!(telemetry.no_pending_streak, 0);
    assert_eq!(telemetry.cadence_phase(), HostCadencePhase::Steady);
    assert!(
        telemetry
            .latest_present_time_ms
            .is_some_and(|t| t >= 1_180.0),
        "present refresh should advance latest_present_time_ms"
    );
    assert!(
        telemetry.present_fps() < 30.0,
        "display hold refresh must not inflate present_fps toward display tick rate"
    );
}

#[test]
fn present_refresh_does_not_inflate_present_fps() {
    let mut telemetry = HostCadenceTelemetry::default();
    telemetry.record_present(1_000.0);
    telemetry.record_present(1_080.0);
    telemetry.record_present(1_180.0);
    let baseline = telemetry.present_fps();
    assert!(baseline > 0.0, "baseline present_fps should be measurable");
    for i in 0..12 {
        telemetry.record_present_refresh(1_190.0 + (i as f64) * 8.0);
    }
    let after_refresh = telemetry.present_fps();
    assert!(
        (after_refresh - baseline).abs() < 5.0,
        "baseline={baseline} after_refresh={after_refresh}"
    );
}

#[test]
fn present_fps_requires_minimum_sample_window() {
    let mut telemetry = HostCadenceTelemetry::default();
    assert_eq!(telemetry.present_fps(), 0.0);
    telemetry.record_present(1_000.0);
    telemetry.record_present(1_006.0);
    assert_eq!(
        telemetry.present_fps(),
        0.0,
        "two tight samples must not produce inflated present_fps"
    );
    telemetry.record_present(1_200.0);
    assert!(
        telemetry.present_fps() > 0.0 && telemetry.present_fps() < 90.0,
        "present_fps={}",
        telemetry.present_fps()
    );
}

#[test]
fn submit_allows_render_pipeline_slack_before_mailbox_stale_drop() {
    let mut slot = ScheduledFrameSlot::default();
    let mut telemetry = HostCadenceTelemetry::default();

    let frame = mk_frame(70, 1_000.0);
    match slot.submit_frame(&frame, 1_090.0, &mut telemetry) {
        ScheduledFrameSubmitOutcome::Accepted { frame_age_ms, .. } => {
            assert!(
                frame_age_ms > telemetry.stale_frame_age_budget_for_frame(&frame),
                "render-side age should exceed steady stale budget"
            );
        }
        other => panic!("expected frame accepted with submit pipeline slack, got {other:?}"),
    }
    let snapshot = slot.diagnostics_snapshot();
    assert_eq!(snapshot.pending_frame_seqs, vec![70]);
}

#[test]
fn take_discards_duplicate_pending_without_presenting_it() {
    let mut slot = ScheduledFrameSlot::default();
    let mut telemetry = HostCadenceTelemetry::default();

    let _ = slot.submit_frame(&mk_frame(80, 1_000.0), 1_010.0, &mut telemetry);
    match slot.take_ready_frame(1_020.0, &mut telemetry) {
        ScheduledFrameTakeOutcome::Ready(frame) => assert_eq!(frame.frame_seq, 80),
        other => panic!("expected initial present, got {other:?}"),
    }

    let stale_duplicate = XbxEngineRenderFrame {
        frame_seq: 75,
        is_keyframe: false,
        frame_recovery_disposition: Some("steady".to_string()),
        ..mk_frame(75, 1_005.0)
    };
    slot.set_pending_for_test(stale_duplicate, 1_032.0);
    match slot.take_ready_frame(1_040.0, &mut telemetry) {
        ScheduledFrameTakeOutcome::RetainedDisplayedFrame => {}
        other => panic!("expected duplicate pending discard with displayed hold, got {other:?}"),
    }
    assert!(
        slot.diagnostics_snapshot().pending_frame_seqs.is_empty(),
        "duplicate pending must be removed without a bogus present"
    );

    let _ = slot.submit_frame(&mk_frame(95, 1_040.0), 1_041.0, &mut telemetry);
    match slot.take_ready_frame(1_050.0, &mut telemetry) {
        ScheduledFrameTakeOutcome::Ready(frame) => assert_eq!(frame.frame_seq, 95),
        other => panic!("expected next pending frame after duplicate discard, got {other:?}"),
    }
}

#[test]
fn take_uses_mailbox_accepted_age_not_render_timestamp() {
    let mut slot = ScheduledFrameSlot::default();
    let mut telemetry = HostCadenceTelemetry::default();

    slot.set_pending_for_test(mk_frame(90, 1_000.0), 1_050.0);
    match slot.take_ready_frame(1_110.0, &mut telemetry) {
        ScheduledFrameTakeOutcome::Ready(frame) => assert_eq!(frame.frame_seq, 90),
        other => panic!(
            "expected pending accepted 60ms ago to present despite 110ms render age, got {other:?}"
        ),
    }
}

#[test]
fn frame_age_budget_tracks_submit_interval_when_display_ticks_are_sparse() {
    let mut telemetry = HostCadenceTelemetry::default();
    telemetry.record_submit(1_000.0);
    telemetry.record_submit(1_050.0);
    telemetry.record_submit(1_100.0);
    let effective_budget = telemetry.frame_age_budget_ms();
    assert!(
        effective_budget >= 20.0 && effective_budget <= 90.0,
        "effective_budget={effective_budget}"
    );
}

#[test]
fn display_interval_falls_back_to_present_cadence_before_display_ticks() {
    let mut telemetry = HostCadenceTelemetry::default();
    assert!(telemetry.display_interval_ms().is_none());
    telemetry.record_present(1_000.0);
    telemetry.record_present(1_033.0);
    let interval = telemetry
        .display_interval_ms()
        .expect("present cadence should bootstrap display interval");
    assert!((interval - 33.0).abs() < 1.0, "interval={interval}");
}

#[test]
fn steady_frame_age_budget_uses_display_interval_floor() {
    let mut telemetry = HostCadenceTelemetry::default();
    telemetry.present_epoch = 4;
    telemetry.cadence_phase = HostCadencePhase::Steady;
    for tick in 0..8 {
        let at_ms = 1_000.0 + tick as f64 * 33.0;
        telemetry.record_display_tick(at_ms);
        telemetry.record_present(at_ms);
    }
    let budget = telemetry.frame_age_budget_ms();
    assert!(
        budget >= 45.0,
        "steady budget should track display interval floor, got {budget}"
    );
}

#[test]
fn begin_view_epoch_replays_displayed_frame_once() {
    let mut slot = ScheduledFrameSlot::default();
    let mut telemetry = HostCadenceTelemetry::default();

    let _ = slot.submit_frame(&mk_frame(40, 1_000.0), 1_010.0, &mut telemetry);
    match slot.take_ready_frame(1_020.0, &mut telemetry) {
        ScheduledFrameTakeOutcome::Ready(frame) => assert_eq!(frame.frame_seq, 40),
        other => panic!("expected frame present, got {other:?}"),
    }

    slot.begin_view_epoch();

    match slot.take_ready_frame(1_030.0, &mut telemetry) {
        ScheduledFrameTakeOutcome::Ready(frame) => assert_eq!(frame.frame_seq, 40),
        other => panic!("expected displayed frame replay, got {other:?}"),
    }
    match slot.take_ready_frame(1_040.0, &mut telemetry) {
        ScheduledFrameTakeOutcome::RetainedDisplayedFrame
        | ScheduledFrameTakeOutcome::NoPendingFrame => {}
        other => panic!("expected replay to finish after one take, got {other:?}"),
    }
}
