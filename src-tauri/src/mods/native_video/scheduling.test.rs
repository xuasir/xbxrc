use std::sync::Arc;

use xbxengine::{XbxEngineRenderFrame, XbxEngineRenderPixelData};

use super::{
    HostCadencePhase, HostCadenceTelemetry, ScheduledFrameSlot, ScheduledFrameSubmitOutcome,
    ScheduledFrameTakeOutcome,
};

fn mk_frame(frame_seq: u64) -> XbxEngineRenderFrame {
    XbxEngineRenderFrame {
        width: 1920,
        height: 1080,
        frame_seq,
        rendered_at_ms: 1_000.0,
        rtp_timestamp: Some(frame_seq as u32),
        recovery_epoch_tag: None,
        recovery_owner_rtp_timestamp: None,
        is_keyframe: frame_seq == 1,
        frame_recovery_disposition: Some("repairing".to_string()),
        frame_unrecoverable_reason: None,
        presentation_value_role: None,
        pixel_data: XbxEngineRenderPixelData::Rgba {
            bytes: Arc::from(vec![0_u8; 4].into_boxed_slice()),
        },
    }
}

fn mk_keyframe(frame_seq: u64) -> XbxEngineRenderFrame {
    XbxEngineRenderFrame {
        is_keyframe: true,
        ..mk_frame(frame_seq)
    }
}

fn mk_frame_with_epoch(frame_seq: u64, recovery_epoch_tag: Option<u64>) -> XbxEngineRenderFrame {
    XbxEngineRenderFrame {
        recovery_epoch_tag,
        recovery_owner_rtp_timestamp: None,
        ..mk_frame(frame_seq)
    }
}

#[test]
fn rebuilding_supply_pending_frame_is_not_overwritten_by_plain_delta() {
    let mut slot = ScheduledFrameSlot::default();
    let mut telemetry = HostCadenceTelemetry::default();

    let rebuilding_supply = XbxEngineRenderFrame {
        frame_seq: 20,
        recovery_epoch_tag: Some(7),
        is_keyframe: false,
        frame_recovery_disposition: Some("rebuilding-supply".to_string()),
        ..mk_frame(20)
    };
    match slot.submit_frame(&rebuilding_supply, 1_020.0, &mut telemetry) {
        ScheduledFrameSubmitOutcome::Accepted {
            overwrote_pending, ..
        } => assert!(!overwrote_pending),
        other => panic!("expected rebuilding-supply frame to be accepted, got {other:?}"),
    }

    let plain_delta = XbxEngineRenderFrame {
        frame_seq: 21,
        recovery_epoch_tag: Some(7),
        is_keyframe: false,
        frame_recovery_disposition: None,
        ..mk_frame(21)
    };
    match slot.submit_frame(&plain_delta, 1_021.0, &mut telemetry) {
        ScheduledFrameSubmitOutcome::RejectedAlreadyPresented { frame_seq, .. } => {
            assert_eq!(frame_seq, 21)
        }
        other => panic!(
            "expected plain delta to be rejected while rebuilding-supply is pending, got {other:?}"
        ),
    }

    match slot.take_ready_frame(1_030.0, &mut telemetry) {
        ScheduledFrameTakeOutcome::Ready(frame) => {
            assert_eq!(frame.frame_seq, 20);
            assert_eq!(
                frame.frame_recovery_disposition.as_deref(),
                Some("rebuilding-supply")
            );
        }
        other => {
            panic!("expected rebuilding-supply frame to stay pending and present, got {other:?}")
        }
    }
}

#[test]
fn plain_delta_replaces_old_repairing_continuation_in_same_epoch() {
    let mut slot = ScheduledFrameSlot::default();
    let mut telemetry = HostCadenceTelemetry::default();

    let old_repairing_continuation = XbxEngineRenderFrame {
        frame_seq: 30,
        recovery_epoch_tag: Some(8),
        is_keyframe: false,
        frame_recovery_disposition: Some("repairing".to_string()),
        ..mk_frame(30)
    };
    match slot.submit_frame(&old_repairing_continuation, 1_020.0, &mut telemetry) {
        ScheduledFrameSubmitOutcome::Accepted {
            overwrote_pending, ..
        } => assert!(!overwrote_pending),
        other => panic!("expected repairing continuation accepted, got {other:?}"),
    }

    let latest_plain_delta = XbxEngineRenderFrame {
        frame_seq: 31,
        recovery_epoch_tag: Some(8),
        is_keyframe: false,
        frame_recovery_disposition: None,
        ..mk_frame(31)
    };
    match slot.submit_frame(&latest_plain_delta, 1_021.0, &mut telemetry) {
        ScheduledFrameSubmitOutcome::Accepted {
            overwrote_pending,
            replaced_frame_seq,
            ..
        } => {
            assert!(overwrote_pending);
            assert_eq!(replaced_frame_seq, Some(30));
        }
        other => panic!(
            "expected latest plain delta to replace old repairing continuation, got {other:?}"
        ),
    }

    match slot.take_ready_frame(1_030.0, &mut telemetry) {
        ScheduledFrameTakeOutcome::Ready(frame) => {
            assert_eq!(frame.frame_seq, 31);
            assert_eq!(frame.frame_recovery_disposition, None);
        }
        other => panic!("expected latest plain delta to win host mailbox, got {other:?}"),
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
fn view_epoch_replays_displayed_frame_once_for_new_host_view() {
    let mut slot = ScheduledFrameSlot::default();
    let mut telemetry = HostCadenceTelemetry::default();

    match slot.submit_frame(&mk_frame(77), 1_010.0, &mut telemetry) {
        ScheduledFrameSubmitOutcome::Accepted { .. } => {}
        other => panic!("expected accepted frame, got {other:?}"),
    }
    match slot.take_ready_frame(1_020.0, &mut telemetry) {
        ScheduledFrameTakeOutcome::Ready(frame) => assert_eq!(frame.frame_seq, 77),
        other => panic!("expected ready frame, got {other:?}"),
    }

    slot.begin_view_epoch();

    match slot.take_ready_frame(1_030.0, &mut telemetry) {
        ScheduledFrameTakeOutcome::Ready(frame) => assert_eq!(frame.frame_seq, 77),
        other => panic!("expected displayed frame replay, got {other:?}"),
    }

    match slot.take_ready_frame(1_040.0, &mut telemetry) {
        ScheduledFrameTakeOutcome::RetainedDisplayedFrame
        | ScheduledFrameTakeOutcome::NoPendingFrame => {}
        other => panic!("expected replay to happen once, got {other:?}"),
    }
}

#[test]
fn recovery_keyframe_with_restarted_frame_seq_opens_new_media_epoch() {
    let mut slot = ScheduledFrameSlot::default();
    let mut telemetry = HostCadenceTelemetry::default();

    match slot.submit_frame(&mk_frame(25), 1_010.0, &mut telemetry) {
        ScheduledFrameSubmitOutcome::Accepted { .. } => {}
        other => panic!("expected accepted frame, got {other:?}"),
    }
    match slot.take_ready_frame(1_020.0, &mut telemetry) {
        ScheduledFrameTakeOutcome::Ready(frame) => assert_eq!(frame.frame_seq, 25),
        other => panic!("expected ready frame, got {other:?}"),
    }

    match slot.submit_frame(&mk_frame(1), 1_030.0, &mut telemetry) {
        ScheduledFrameSubmitOutcome::Accepted { frame_seq, .. } => assert_eq!(frame_seq, 1),
        other => panic!("expected restarted epoch keyframe to be accepted, got {other:?}"),
    }
    match slot.take_ready_frame(1_040.0, &mut telemetry) {
        ScheduledFrameTakeOutcome::Ready(frame) => assert_eq!(frame.frame_seq, 1),
        other => panic!("expected restarted epoch keyframe to present, got {other:?}"),
    }

    match slot.submit_frame(&mk_frame(2), 1_050.0, &mut telemetry) {
        ScheduledFrameSubmitOutcome::Accepted { frame_seq, .. } => assert_eq!(frame_seq, 2),
        other => {
            panic!("expected restarted epoch continuation frame to be accepted, got {other:?}")
        }
    }
}

#[test]
fn recovery_keyframe_with_rewound_frame_seq_opens_new_media_epoch() {
    let mut slot = ScheduledFrameSlot::default();
    let mut telemetry = HostCadenceTelemetry::default();

    match slot.submit_frame(&mk_frame(161), 1_010.0, &mut telemetry) {
        ScheduledFrameSubmitOutcome::Accepted { .. } => {}
        other => panic!("expected accepted frame, got {other:?}"),
    }
    match slot.take_ready_frame(1_020.0, &mut telemetry) {
        ScheduledFrameTakeOutcome::Ready(frame) => assert_eq!(frame.frame_seq, 161),
        other => panic!("expected ready frame, got {other:?}"),
    }

    match slot.submit_frame(&mk_keyframe(13), 1_030.0, &mut telemetry) {
        ScheduledFrameSubmitOutcome::Accepted { frame_seq, .. } => assert_eq!(frame_seq, 13),
        other => panic!("expected rewound epoch keyframe to be accepted, got {other:?}"),
    }
    match slot.take_ready_frame(1_040.0, &mut telemetry) {
        ScheduledFrameTakeOutcome::Ready(frame) => assert_eq!(frame.frame_seq, 13),
        other => panic!("expected rewound epoch keyframe to present, got {other:?}"),
    }
}

#[test]
fn recovery_continuation_with_newer_epoch_opens_new_media_epoch() {
    let mut slot = ScheduledFrameSlot::default();
    let mut telemetry = HostCadenceTelemetry::default();

    match slot.submit_frame(&mk_frame(161), 1_010.0, &mut telemetry) {
        ScheduledFrameSubmitOutcome::Accepted { .. } => {}
        other => panic!("expected accepted frame, got {other:?}"),
    }
    match slot.take_ready_frame(1_020.0, &mut telemetry) {
        ScheduledFrameTakeOutcome::Ready(frame) => assert_eq!(frame.frame_seq, 161),
        other => panic!("expected ready frame, got {other:?}"),
    }

    let recovery_continuation = XbxEngineRenderFrame {
        frame_seq: 13,
        recovery_epoch_tag: Some(1),
        is_keyframe: false,
        frame_recovery_disposition: Some("repairing".to_string()),
        ..mk_frame(13)
    };
    match slot.submit_frame(&recovery_continuation, 1_030.0, &mut telemetry) {
        ScheduledFrameSubmitOutcome::Accepted { frame_seq, .. } => assert_eq!(frame_seq, 13),
        other => panic!(
            "expected rewound continuation with newer recovery epoch to be accepted, got {other:?}"
        ),
    }
    match slot.take_ready_frame(1_040.0, &mut telemetry) {
        ScheduledFrameTakeOutcome::Ready(frame) => {
            assert_eq!(frame.frame_seq, 13);
            assert_eq!(frame.recovery_epoch_tag, Some(1));
            assert!(!frame.is_keyframe);
        }
        other => panic!(
            "expected rewound continuation with newer recovery epoch to present, got {other:?}"
        ),
    }
}

#[test]
fn rewound_recovery_continuation_opens_new_media_epoch_even_without_newer_epoch() {
    let mut slot = ScheduledFrameSlot::default();
    let mut telemetry = HostCadenceTelemetry::default();

    match slot.submit_frame(&mk_frame_with_epoch(161, Some(1)), 1_010.0, &mut telemetry) {
        ScheduledFrameSubmitOutcome::Accepted { .. } => {}
        other => panic!("expected accepted frame, got {other:?}"),
    }
    match slot.take_ready_frame(1_020.0, &mut telemetry) {
        ScheduledFrameTakeOutcome::Ready(frame) => assert_eq!(frame.frame_seq, 161),
        other => panic!("expected ready frame, got {other:?}"),
    }

    let rewound_continuation = XbxEngineRenderFrame {
        frame_seq: 13,
        recovery_epoch_tag: Some(1),
        is_keyframe: false,
        frame_recovery_disposition: Some("repairing".to_string()),
        ..mk_frame(13)
    };
    match slot.submit_frame(&rewound_continuation, 1_030.0, &mut telemetry) {
        ScheduledFrameSubmitOutcome::Accepted { frame_seq, .. } => assert_eq!(frame_seq, 13),
        other => panic!(
            "expected rewound recovery continuation to open a new media epoch, got {other:?}"
        ),
    }
    match slot.take_ready_frame(1_040.0, &mut telemetry) {
        ScheduledFrameTakeOutcome::Ready(frame) => {
            assert_eq!(frame.frame_seq, 13);
            assert_eq!(frame.recovery_epoch_tag, Some(1));
        }
        other => panic!(
            "expected rewound recovery continuation to present after epoch rollover, got {other:?}"
        ),
    }
}

#[test]
fn rewound_plain_continuation_without_recovery_signal_stays_rejected() {
    let mut slot = ScheduledFrameSlot::default();
    let mut telemetry = HostCadenceTelemetry::default();

    match slot.submit_frame(&mk_frame_with_epoch(161, Some(1)), 1_010.0, &mut telemetry) {
        ScheduledFrameSubmitOutcome::Accepted { .. } => {}
        other => panic!("expected accepted frame, got {other:?}"),
    }
    match slot.take_ready_frame(1_020.0, &mut telemetry) {
        ScheduledFrameTakeOutcome::Ready(frame) => assert_eq!(frame.frame_seq, 161),
        other => panic!("expected ready frame, got {other:?}"),
    }

    let rewound_plain = XbxEngineRenderFrame {
        frame_seq: 13,
        recovery_epoch_tag: Some(1),
        is_keyframe: false,
        frame_recovery_disposition: None,
        ..mk_frame(13)
    };
    match slot.submit_frame(&rewound_plain, 1_030.0, &mut telemetry) {
        ScheduledFrameSubmitOutcome::RejectedAlreadyPresented { frame_seq, .. } => {
            assert_eq!(frame_seq, 13)
        }
        other => panic!(
            "expected rewound plain continuation without recovery signal to stay rejected, got {other:?}"
        ),
    }
}

#[test]
fn plain_delta_with_new_owner_stays_rejected() {
    let mut slot = ScheduledFrameSlot::default();
    let mut telemetry = HostCadenceTelemetry::default();

    let displayed = XbxEngineRenderFrame {
        frame_seq: 1931,
        recovery_epoch_tag: Some(1),
        recovery_owner_rtp_timestamp: Some(100_001),
        frame_recovery_disposition: None,
        ..mk_frame(1931)
    };
    match slot.submit_frame(&displayed, 1_010.0, &mut telemetry) {
        ScheduledFrameSubmitOutcome::Accepted { .. } => {}
        other => panic!("expected accepted displayed frame, got {other:?}"),
    }
    match slot.take_ready_frame(1_020.0, &mut telemetry) {
        ScheduledFrameTakeOutcome::Ready(frame) => assert_eq!(frame.frame_seq, 1931),
        other => panic!("expected ready displayed frame, got {other:?}"),
    }

    let next_owner_delta = XbxEngineRenderFrame {
        frame_seq: 17,
        recovery_epoch_tag: Some(1),
        recovery_owner_rtp_timestamp: Some(200_001),
        frame_recovery_disposition: None,
        ..mk_frame(17)
    };
    match slot.submit_frame(&next_owner_delta, 1_030.0, &mut telemetry) {
        ScheduledFrameSubmitOutcome::RejectedAlreadyPresented { frame_seq, .. } => {
            assert_eq!(frame_seq, 17)
        }
        other => panic!("expected plain delta with new owner to stay rejected, got {other:?}"),
    };
    match slot.take_ready_frame(1_040.0, &mut telemetry) {
        ScheduledFrameTakeOutcome::RetainedDisplayedFrame => {}
        other => panic!("expected displayed frame to stay retained, got {other:?}"),
    }
}

#[test]
fn trace_like_recovery_continuation_replaces_old_displayed_epoch() {
    let mut slot = ScheduledFrameSlot::default();
    let mut telemetry = HostCadenceTelemetry::default();

    let old_displayed = XbxEngineRenderFrame {
        frame_seq: 1663,
        rtp_timestamp: Some(451_337_195),
        recovery_epoch_tag: Some(1),
        frame_recovery_disposition: None,
        ..mk_frame(1663)
    };
    match slot.submit_frame(&old_displayed, 1_010.0, &mut telemetry) {
        ScheduledFrameSubmitOutcome::Accepted { .. } => {}
        other => panic!("expected accepted frame, got {other:?}"),
    }
    match slot.take_ready_frame(1_020.0, &mut telemetry) {
        ScheduledFrameTakeOutcome::Ready(frame) => assert_eq!(frame.frame_seq, 1663),
        other => panic!("expected ready frame, got {other:?}"),
    }

    let recovery_continuation = XbxEngineRenderFrame {
        frame_seq: 534,
        rtp_timestamp: Some(415_770_399),
        recovery_epoch_tag: Some(1),
        is_keyframe: false,
        frame_recovery_disposition: Some("rebuilding-supply".to_string()),
        ..mk_frame(534)
    };
    match slot.submit_frame(&recovery_continuation, 1_030.0, &mut telemetry) {
        ScheduledFrameSubmitOutcome::Accepted { frame_seq, .. } => assert_eq!(frame_seq, 534),
        other => panic!(
            "expected trace-like recovery continuation to open a fresh host epoch, got {other:?}"
        ),
    }
    match slot.take_ready_frame(1_040.0, &mut telemetry) {
        ScheduledFrameTakeOutcome::Ready(frame) => {
            assert_eq!(frame.frame_seq, 534);
            assert_eq!(frame.rtp_timestamp, Some(415_770_399));
            assert_eq!(
                frame.frame_recovery_disposition.as_deref(),
                Some("rebuilding-supply")
            );
        }
        other => panic!(
            "expected trace-like recovery continuation to replace the retained old frame, got {other:?}"
        ),
    }
}

#[test]
fn take_reuses_last_presented_frame_and_continues_counting_no_pending() {
    let mut slot = ScheduledFrameSlot::default();
    let mut telemetry = HostCadenceTelemetry::default();

    match slot.submit_frame(&mk_frame(11), 1_010.0, &mut telemetry) {
        ScheduledFrameSubmitOutcome::Accepted { .. } => {}
        other => panic!("expected accepted frame, got {other:?}"),
    }
    match slot.take_ready_frame(1_020.0, &mut telemetry) {
        ScheduledFrameTakeOutcome::Ready(frame) => assert_eq!(frame.frame_seq, 11),
        other => panic!("expected ready frame, got {other:?}"),
    }
    match slot.take_ready_frame(1_036.0, &mut telemetry) {
        ScheduledFrameTakeOutcome::RetainedDisplayedFrame => {}
        other => panic!("expected retained displayed frame marker, got {other:?}"),
    }

    assert_eq!(telemetry.no_pending_take_count_total, 1);
    assert_eq!(telemetry.no_pending_streak, 1);
}

#[test]
fn submit_uses_bounded_queue_and_drops_oldest_pending_on_overflow() {
    let mut slot = ScheduledFrameSlot::default();
    let mut telemetry = HostCadenceTelemetry::default();

    for frame_seq in 2..=5 {
        match slot.submit_frame(
            &mk_frame(frame_seq),
            1_010.0 + frame_seq as f64,
            &mut telemetry,
        ) {
            ScheduledFrameSubmitOutcome::Accepted {
                overwrote_pending,
                replaced_frame_seq,
                ..
            } => {
                if frame_seq == 2 {
                    assert!(!overwrote_pending);
                    assert_eq!(replaced_frame_seq, None);
                } else {
                    assert!(overwrote_pending);
                    assert_eq!(replaced_frame_seq, Some((frame_seq - 1) as u64));
                }
            }
            other => panic!("expected accepted frame, got {other:?}"),
        }
    }

    // mailbox：只保留最新 pending 候选，因此只会取到最后一帧。
    match slot.take_ready_frame(1_062.0, &mut telemetry) {
        ScheduledFrameTakeOutcome::Ready(frame) => assert_eq!(frame.frame_seq, 5),
        other => panic!("expected newest pending frame, got {other:?}"),
    }
    match slot.take_ready_frame(1_078.0, &mut telemetry) {
        ScheduledFrameTakeOutcome::RetainedDisplayedFrame => {}
        other => {
            panic!("expected retained displayed frame marker after queue drains, got {other:?}")
        }
    }

    assert_eq!(telemetry.present_overwrite_count_total, 3);
    assert_eq!(telemetry.no_pending_take_count_total, 1);
}

#[test]
fn pending_recovery_keyframe_is_not_overwritten_by_newer_delta_frame() {
    let mut slot = ScheduledFrameSlot::default();
    let mut telemetry = HostCadenceTelemetry::default();

    match slot.submit_frame(&mk_keyframe(10), 1_010.0, &mut telemetry) {
        ScheduledFrameSubmitOutcome::Accepted { .. } => {}
        other => panic!("expected accepted keyframe, got {other:?}"),
    }
    match slot.submit_frame(&mk_frame(11), 1_011.0, &mut telemetry) {
        ScheduledFrameSubmitOutcome::RejectedAlreadyPresented { frame_seq, .. } => {
            assert_eq!(frame_seq, 11)
        }
        other => {
            panic!("expected delta frame to be rejected behind pending keyframe, got {other:?}")
        }
    }
    match slot.take_ready_frame(1_030.0, &mut telemetry) {
        ScheduledFrameTakeOutcome::Ready(frame) => {
            assert_eq!(frame.frame_seq, 10);
            assert!(frame.is_keyframe);
        }
        other => panic!("expected pending keyframe to survive host mailbox, got {other:?}"),
    }
}

#[test]
fn higher_recovery_epoch_pending_frame_replaces_lower_epoch_candidate() {
    let mut slot = ScheduledFrameSlot::default();
    let mut telemetry = HostCadenceTelemetry::default();

    let stale_epoch_anchor = XbxEngineRenderFrame {
        is_keyframe: true,
        frame_recovery_disposition: Some("repairing".to_string()),
        ..mk_frame_with_epoch(20, Some(2))
    };
    let current_epoch_anchor = XbxEngineRenderFrame {
        frame_seq: 19,
        is_keyframe: true,
        frame_recovery_disposition: Some("repairing".to_string()),
        ..mk_frame_with_epoch(19, Some(3))
    };

    match slot.submit_frame(&stale_epoch_anchor, 1_010.0, &mut telemetry) {
        ScheduledFrameSubmitOutcome::Accepted { .. } => {}
        other => panic!("expected stale epoch anchor accepted, got {other:?}"),
    }
    match slot.submit_frame(&current_epoch_anchor, 1_011.0, &mut telemetry) {
        ScheduledFrameSubmitOutcome::Accepted {
            overwrote_pending,
            replaced_frame_seq,
            ..
        } => {
            assert!(overwrote_pending);
            assert_eq!(replaced_frame_seq, Some(20));
        }
        other => panic!(
            "expected current epoch anchor to replace pending stale epoch frame, got {other:?}"
        ),
    }
    match slot.take_ready_frame(1_030.0, &mut telemetry) {
        ScheduledFrameTakeOutcome::Ready(frame) => {
            assert_eq!(frame.frame_seq, 19);
            assert_eq!(frame.recovery_epoch_tag, Some(3));
        }
        other => panic!("expected current epoch anchor to win host mailbox, got {other:?}"),
    }
}

#[test]
fn owner_frame_in_same_epoch_replaces_non_owner_pending_candidate() {
    let mut slot = ScheduledFrameSlot::default();
    let mut telemetry = HostCadenceTelemetry::default();

    let non_owner = XbxEngineRenderFrame {
        recovery_epoch_tag: Some(4),
        recovery_owner_rtp_timestamp: Some(120),
        rtp_timestamp: Some(121),
        frame_seq: 121,
        is_keyframe: false,
        frame_recovery_disposition: Some("repairing".to_string()),
        ..mk_frame(121)
    };
    let owner = XbxEngineRenderFrame {
        recovery_epoch_tag: Some(4),
        recovery_owner_rtp_timestamp: Some(120),
        rtp_timestamp: Some(120),
        frame_seq: 120,
        is_keyframe: true,
        frame_recovery_disposition: Some("repairing".to_string()),
        ..mk_frame(120)
    };

    match slot.submit_frame(&non_owner, 1_010.0, &mut telemetry) {
        ScheduledFrameSubmitOutcome::Accepted { .. } => {}
        other => panic!("expected non-owner pending accepted, got {other:?}"),
    }
    match slot.submit_frame(&owner, 1_011.0, &mut telemetry) {
        ScheduledFrameSubmitOutcome::Accepted {
            overwrote_pending,
            replaced_frame_seq,
            ..
        } => {
            assert!(overwrote_pending);
            assert_eq!(replaced_frame_seq, Some(121));
        }
        other => panic!("expected owner frame to replace non-owner pending, got {other:?}"),
    }
    match slot.take_ready_frame(1_030.0, &mut telemetry) {
        ScheduledFrameTakeOutcome::Ready(frame) => {
            assert_eq!(frame.rtp_timestamp, Some(120));
            assert_eq!(frame.recovery_owner_rtp_timestamp, Some(120));
        }
        other => panic!("expected owner frame to win host mailbox, got {other:?}"),
    }
}

#[test]
fn stale_pending_frame_falls_back_to_displayed_frame_instead_of_starving() {
    let mut slot = ScheduledFrameSlot::default();
    let mut telemetry = HostCadenceTelemetry::default();

    match slot.submit_frame(&mk_frame(40), 1_010.0, &mut telemetry) {
        ScheduledFrameSubmitOutcome::Accepted { .. } => {}
        other => panic!("expected accepted frame, got {other:?}"),
    }
    match slot.take_ready_frame(1_020.0, &mut telemetry) {
        ScheduledFrameTakeOutcome::Ready(frame) => assert_eq!(frame.frame_seq, 40),
        other => panic!("expected ready frame, got {other:?}"),
    }

    let stale_pending = XbxEngineRenderFrame {
        rendered_at_ms: 1_030.0,
        frame_recovery_disposition: None,
        ..mk_frame(41)
    };
    match slot.submit_frame(&stale_pending, 1_035.0, &mut telemetry) {
        ScheduledFrameSubmitOutcome::Accepted { .. } => {}
        other => panic!("expected accepted frame, got {other:?}"),
    }

    match slot.take_ready_frame(1_240.0, &mut telemetry) {
        ScheduledFrameTakeOutcome::RetainedDisplayedFrame => {}
        other => panic!("expected fallback to retained displayed frame marker, got {other:?}"),
    }

    assert_eq!(telemetry.present_drop_count_total, 1);
    assert_eq!(telemetry.no_pending_take_count_total, 1);
    assert_eq!(telemetry.no_pending_streak, 1);
}

#[test]
fn stale_recovery_anchor_replaces_old_displayed_frame_once() {
    let mut slot = ScheduledFrameSlot::default();
    let mut telemetry = HostCadenceTelemetry::default();

    match slot.submit_frame(&mk_frame(40), 1_010.0, &mut telemetry) {
        ScheduledFrameSubmitOutcome::Accepted { .. } => {}
        other => panic!("expected accepted frame, got {other:?}"),
    }
    match slot.take_ready_frame(1_020.0, &mut telemetry) {
        ScheduledFrameTakeOutcome::Ready(frame) => assert_eq!(frame.frame_seq, 40),
        other => panic!("expected ready frame, got {other:?}"),
    }

    let stale_recovery_anchor = XbxEngineRenderFrame {
        frame_seq: 41,
        rendered_at_ms: 1_030.0,
        is_keyframe: true,
        frame_recovery_disposition: Some("repairing".to_string()),
        ..mk_frame(41)
    };
    match slot.submit_frame(&stale_recovery_anchor, 1_035.0, &mut telemetry) {
        ScheduledFrameSubmitOutcome::Accepted { .. } => {}
        other => panic!("expected accepted recovery anchor, got {other:?}"),
    }

    match slot.take_ready_frame(1_240.0, &mut telemetry) {
        ScheduledFrameTakeOutcome::Ready(frame) => {
            assert_eq!(frame.frame_seq, 41);
            assert!(frame.is_keyframe);
        }
        other => panic!("expected stale recovery anchor to replace displayed frame, got {other:?}"),
    }

    assert_eq!(telemetry.present_drop_count_total, 0);
    assert_eq!(telemetry.no_pending_take_count_total, 0);
}

#[test]
fn stale_rebuilding_supply_replaces_displayed_repairing_frame_once() {
    let mut slot = ScheduledFrameSlot::default();
    let mut telemetry = HostCadenceTelemetry::default();

    let displayed_repairing = XbxEngineRenderFrame {
        frame_seq: 45,
        recovery_epoch_tag: Some(9),
        rendered_at_ms: 1_010.0,
        is_keyframe: false,
        frame_recovery_disposition: Some("repairing".to_string()),
        ..mk_frame(45)
    };
    match slot.submit_frame(&displayed_repairing, 1_012.0, &mut telemetry) {
        ScheduledFrameSubmitOutcome::Accepted { .. } => {}
        other => panic!("expected accepted displayed repairing frame, got {other:?}"),
    }
    match slot.take_ready_frame(1_020.0, &mut telemetry) {
        ScheduledFrameTakeOutcome::Ready(frame) => assert_eq!(frame.frame_seq, 45),
        other => panic!("expected ready displayed repairing frame, got {other:?}"),
    }

    let stale_rebuilding_supply = XbxEngineRenderFrame {
        frame_seq: 46,
        recovery_epoch_tag: Some(10),
        recovery_owner_rtp_timestamp: Some(46),
        rendered_at_ms: 1_030.0,
        is_keyframe: false,
        frame_recovery_disposition: Some("rebuilding-supply".to_string()),
        ..mk_frame(46)
    };
    match slot.submit_frame(&stale_rebuilding_supply, 1_035.0, &mut telemetry) {
        ScheduledFrameSubmitOutcome::Accepted { .. } => {}
        other => panic!("expected accepted rebuilding-supply frame, got {other:?}"),
    }

    match slot.take_ready_frame(1_240.0, &mut telemetry) {
        ScheduledFrameTakeOutcome::Ready(frame) => {
            assert_eq!(frame.frame_seq, 46);
            assert_eq!(
                frame.frame_recovery_disposition.as_deref(),
                Some("rebuilding-supply")
            );
        }
        other => panic!(
            "expected stale rebuilding-supply frame to replace displayed repairing frame, got {other:?}"
        ),
    }

    let snapshot = slot.diagnostics_snapshot();
    assert_eq!(snapshot.displayed_frame_seq, Some(46));
    assert_eq!(
        snapshot.displayed_frame_recovery_disposition.as_deref(),
        Some("rebuilding-supply")
    );
    assert_eq!(telemetry.present_drop_count_total, 0);
}

#[test]
fn stale_repairing_continuation_does_not_replace_displayed_frame() {
    let mut slot = ScheduledFrameSlot::default();
    let mut telemetry = HostCadenceTelemetry::default();

    let displayed = XbxEngineRenderFrame {
        frame_recovery_disposition: None,
        ..mk_frame(50)
    };
    match slot.submit_frame(&displayed, 1_010.0, &mut telemetry) {
        ScheduledFrameSubmitOutcome::Accepted { .. } => {}
        other => panic!("expected accepted displayed frame, got {other:?}"),
    }
    match slot.take_ready_frame(1_020.0, &mut telemetry) {
        ScheduledFrameTakeOutcome::Ready(frame) => assert_eq!(frame.frame_seq, 50),
        other => panic!("expected ready displayed frame, got {other:?}"),
    }

    let stale_repairing_continuation = XbxEngineRenderFrame {
        frame_seq: 51,
        rendered_at_ms: 1_030.0,
        is_keyframe: false,
        frame_recovery_disposition: Some("repairing".to_string()),
        ..mk_frame(51)
    };
    match slot.submit_frame(&stale_repairing_continuation, 1_035.0, &mut telemetry) {
        ScheduledFrameSubmitOutcome::Accepted { .. } => {}
        other => panic!("expected accepted repairing continuation, got {other:?}"),
    }

    match slot.take_ready_frame(1_240.0, &mut telemetry) {
        ScheduledFrameTakeOutcome::RetainedDisplayedFrame => {}
        other => panic!(
            "expected stale repairing continuation to be dropped in favor of displayed frame, got {other:?}"
        ),
    }

    assert_eq!(telemetry.present_drop_count_total, 1);
}

#[test]
fn fresh_pending_frame_is_presented_without_displayed_frame_recomparison() {
    let mut slot = ScheduledFrameSlot::default();
    let mut telemetry = HostCadenceTelemetry::default();

    let displayed = XbxEngineRenderFrame {
        frame_recovery_disposition: None,
        ..mk_frame(60)
    };
    match slot.submit_frame(&displayed, 1_010.0, &mut telemetry) {
        ScheduledFrameSubmitOutcome::Accepted { .. } => {}
        other => panic!("expected accepted displayed frame, got {other:?}"),
    }
    match slot.take_ready_frame(1_020.0, &mut telemetry) {
        ScheduledFrameTakeOutcome::Ready(frame) => assert_eq!(frame.frame_seq, 60),
        other => panic!("expected ready displayed frame, got {other:?}"),
    }

    let fresh_repairing_continuation = XbxEngineRenderFrame {
        frame_seq: 61,
        rendered_at_ms: 1_034.0,
        is_keyframe: false,
        frame_recovery_disposition: Some("repairing".to_string()),
        ..mk_frame(61)
    };
    match slot.submit_frame(&fresh_repairing_continuation, 1_035.0, &mut telemetry) {
        ScheduledFrameSubmitOutcome::Accepted { .. } => {}
        other => panic!("expected accepted repairing continuation, got {other:?}"),
    }

    match slot.take_ready_frame(1_040.0, &mut telemetry) {
        ScheduledFrameTakeOutcome::Ready(frame) => {
            assert_eq!(frame.frame_seq, 61);
            assert_eq!(
                frame.frame_recovery_disposition.as_deref(),
                Some("repairing")
            );
        }
        other => panic!("expected host to present accepted pending frame, got {other:?}"),
    }
}

#[test]
fn fresh_plain_delta_replaces_displayed_repairing_frame() {
    let mut slot = ScheduledFrameSlot::default();
    let mut telemetry = HostCadenceTelemetry::default();

    let displayed_repairing = XbxEngineRenderFrame {
        frame_recovery_disposition: Some("repairing".to_string()),
        ..mk_frame(70)
    };
    match slot.submit_frame(&displayed_repairing, 1_010.0, &mut telemetry) {
        ScheduledFrameSubmitOutcome::Accepted { .. } => {}
        other => panic!("expected accepted repairing displayed frame, got {other:?}"),
    }
    match slot.take_ready_frame(1_020.0, &mut telemetry) {
        ScheduledFrameTakeOutcome::Ready(frame) => assert_eq!(frame.frame_seq, 70),
        other => panic!("expected ready repairing displayed frame, got {other:?}"),
    }

    let fresh_plain_delta = XbxEngineRenderFrame {
        frame_seq: 71,
        rendered_at_ms: 1_034.0,
        is_keyframe: false,
        frame_recovery_disposition: None,
        ..mk_frame(71)
    };
    match slot.submit_frame(&fresh_plain_delta, 1_035.0, &mut telemetry) {
        ScheduledFrameSubmitOutcome::Accepted { .. } => {}
        other => panic!("expected accepted plain delta, got {other:?}"),
    }

    match slot.take_ready_frame(1_040.0, &mut telemetry) {
        ScheduledFrameTakeOutcome::Ready(frame) => {
            assert_eq!(frame.frame_seq, 71);
            assert_eq!(frame.frame_recovery_disposition, None);
        }
        other => panic!("expected plain delta to replace displayed repairing frame, got {other:?}"),
    }
}

#[test]
fn fresh_plain_delta_replaces_displayed_recovery_keyframe_after_anchor_commit() {
    let mut slot = ScheduledFrameSlot::default();
    let mut telemetry = HostCadenceTelemetry::default();

    let displayed_recovery_keyframe = XbxEngineRenderFrame {
        frame_seq: 2,
        rendered_at_ms: 1_010.0,
        rtp_timestamp: Some(2_236_399_049),
        recovery_epoch_tag: Some(1),
        recovery_owner_rtp_timestamp: Some(2_236_399_049),
        is_keyframe: true,
        frame_recovery_disposition: Some("repairing".to_string()),
        ..mk_frame(2)
    };
    match slot.submit_frame(&displayed_recovery_keyframe, 1_012.0, &mut telemetry) {
        ScheduledFrameSubmitOutcome::Accepted { .. } => {}
        other => panic!("expected accepted displayed recovery keyframe, got {other:?}"),
    }
    match slot.take_ready_frame(1_020.0, &mut telemetry) {
        ScheduledFrameTakeOutcome::Ready(frame) => {
            assert_eq!(frame.frame_seq, 2);
            assert!(frame.is_keyframe);
        }
        other => panic!("expected ready displayed recovery keyframe, got {other:?}"),
    }

    let fresh_plain_delta = XbxEngineRenderFrame {
        frame_seq: 48,
        rendered_at_ms: 1_030.0,
        rtp_timestamp: Some(2_236_536_929),
        recovery_epoch_tag: Some(1),
        recovery_owner_rtp_timestamp: Some(2_236_399_049),
        is_keyframe: false,
        frame_recovery_disposition: None,
        ..mk_frame(48)
    };
    match slot.submit_frame(&fresh_plain_delta, 1_035.0, &mut telemetry) {
        ScheduledFrameSubmitOutcome::Accepted { .. } => {}
        other => panic!("expected accepted plain delta, got {other:?}"),
    }

    match slot.take_ready_frame(1_040.0, &mut telemetry) {
        ScheduledFrameTakeOutcome::Ready(frame) => {
            assert_eq!(frame.frame_seq, 48);
            assert_eq!(frame.rtp_timestamp, Some(2_236_536_929));
            assert_eq!(frame.frame_recovery_disposition, None);
        }
        other => panic!(
            "expected fresh plain delta to replace displayed recovery keyframe, got {other:?}"
        ),
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
fn telemetry_diagnostics_snapshot_captures_epochs_and_no_pending_state() {
    let mut telemetry = HostCadenceTelemetry::default();

    telemetry.record_display_tick(1_000.0);
    telemetry.record_display_tick(1_016.0);
    telemetry.record_present(1_018.0);
    telemetry.record_no_pending_take();

    let snapshot = telemetry.diagnostics_snapshot();
    assert_eq!(snapshot.display_tick_epoch, 2);
    assert_eq!(snapshot.present_epoch, 1);
    assert_eq!(snapshot.cadence_phase, HostCadencePhase::Starved);
    assert_eq!(snapshot.no_pending_streak, 1);
    assert_eq!(snapshot.no_pending_take_count_total, 1);
    assert_eq!(snapshot.present_enqueue_count_total, 0);
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

#[test]
fn starved_submit_uses_relaxed_stale_budget_to_accept_recovery_frame() {
    let mut slot = ScheduledFrameSlot::default();
    let mut telemetry = HostCadenceTelemetry::default();

    telemetry.record_display_tick(1_000.0);
    telemetry.record_display_tick(1_008.0);
    telemetry.record_present(1_010.0);
    for _ in 0..16 {
        telemetry.record_no_pending_take();
    }

    let stale_candidate = XbxEngineRenderFrame {
        rendered_at_ms: 1_050.0,
        ..mk_frame(77)
    };

    match slot.submit_frame(&stale_candidate, 1_210.0, &mut telemetry) {
        ScheduledFrameSubmitOutcome::Accepted { frame_seq, .. } => assert_eq!(frame_seq, 77),
        other => panic!("expected starved recovery frame to be accepted, got {other:?}"),
    }
}

#[test]
fn recovery_keyframe_is_not_dropped_as_stale_immediately_after_submit() {
    let mut slot = ScheduledFrameSlot::default();
    let mut telemetry = HostCadenceTelemetry::default();

    match slot.submit_frame(&mk_frame(20), 1_012.0, &mut telemetry) {
        ScheduledFrameSubmitOutcome::Accepted { .. } => {}
        other => panic!("expected accepted frame, got {other:?}"),
    }
    match slot.take_ready_frame(1_012.0, &mut telemetry) {
        ScheduledFrameTakeOutcome::Ready(frame) => assert_eq!(frame.frame_seq, 20),
        other => panic!("expected ready frame, got {other:?}"),
    }

    let recovery_keyframe = XbxEngineRenderFrame {
        frame_seq: 27,
        rendered_at_ms: 1_671.0,
        is_keyframe: true,
        frame_recovery_disposition: Some("repairing".to_string()),
        ..mk_frame(27)
    };

    match slot.submit_frame(&recovery_keyframe, 1_687.0, &mut telemetry) {
        ScheduledFrameSubmitOutcome::Accepted { frame_seq, .. } => assert_eq!(frame_seq, 27),
        other => panic!("expected accepted recovery keyframe, got {other:?}"),
    }

    match slot.take_ready_frame(1_696.0, &mut telemetry) {
        ScheduledFrameTakeOutcome::Ready(frame) => assert_eq!(frame.frame_seq, 27),
        other => panic!("expected recovery keyframe to stay eligible for present, got {other:?}"),
    }
}

#[test]
fn low_video_fps_submit_interval_relaxes_host_stale_budget_under_high_refresh_ticks() {
    let mut telemetry = HostCadenceTelemetry::default();

    for index in 0..6 {
        telemetry.record_display_tick(1_000.0 + index as f64 * 8.33);
    }

    let first_gap = telemetry.record_submit(1_000.0);
    let second_gap = telemetry.record_submit(1_033.0);
    let third_gap = telemetry.record_submit(1_066.0);

    assert_eq!(first_gap, None);
    assert_eq!(second_gap, Some(33.0));
    assert_eq!(third_gap, Some(33.0));
    assert!(
        telemetry.frame_age_budget_ms() >= 70.0,
        "expected video-fps-aware budget under high-refresh host, got {}",
        telemetry.frame_age_budget_ms()
    );
}

#[test]
fn slot_diagnostics_snapshot_reports_displayed_and_pending_frames() {
    let mut slot = ScheduledFrameSlot::default();
    let mut telemetry = HostCadenceTelemetry::default();

    match slot.submit_frame(&mk_frame(11), 1_010.0, &mut telemetry) {
        ScheduledFrameSubmitOutcome::Accepted { .. } => {}
        other => panic!("expected accepted frame, got {other:?}"),
    }
    match slot.take_ready_frame(1_011.0, &mut telemetry) {
        ScheduledFrameTakeOutcome::Ready(frame) => assert_eq!(frame.frame_seq, 11),
        other => panic!("expected ready frame, got {other:?}"),
    }
    match slot.submit_frame(&mk_frame(12), 1_012.0, &mut telemetry) {
        ScheduledFrameSubmitOutcome::Accepted { .. } => {}
        other => panic!("expected accepted frame, got {other:?}"),
    }

    let snapshot = slot.diagnostics_snapshot();
    assert_eq!(snapshot.displayed_frame_seq, Some(11));
    assert_eq!(snapshot.displayed_frame_rtp_timestamp, Some(11));
    assert_eq!(
        snapshot.displayed_frame_recovery_disposition.as_deref(),
        Some("repairing")
    );
    assert_eq!(snapshot.displayed_frame_rendered_at_ms, Some(1_000.0));
    assert_eq!(snapshot.pending_frame_seqs, vec![12]);
    assert_eq!(snapshot.last_presented_frame_seq, Some(11));
    assert_eq!(snapshot.queue_depth, 2);
    assert_eq!(snapshot.pending_queue_depth, 1);
    assert!(snapshot.has_displayed_frame);
}
