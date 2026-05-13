use std::sync::Arc;

use xbxengine::{XbxEngineRenderFrame, XbxEngineRenderPixelData};

use super::{
    HostCadenceTelemetry, ScheduledFrameSlot, ScheduledFrameSubmitOutcome,
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
