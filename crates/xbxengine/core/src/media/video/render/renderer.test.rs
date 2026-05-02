use std::sync::Arc;

use super::{XbxPresentFrameOutcome, XbxRenderFrame, XbxRenderState};
use crate::XbxEngineRenderPixelData;

#[test]
fn latest_slot_is_shadow_state_and_pending_queue_is_handoff_source() {
    let mut state = XbxRenderState::default();
    let frame = XbxRenderFrame {
        width: 2,
        height: 2,
        frame_seq: 1,
        rendered_at_ms: 1_000.0,
        rtp_timestamp: Some(1),
        recovery_epoch_tag: None,
        recovery_owner_rtp_timestamp: None,
        is_keyframe: true,
        frame_recovery_disposition: Some("repairing".to_string()),
        frame_unrecoverable_reason: None,
        presentation_value_role: None,
        pixel_data: XbxEngineRenderPixelData::Rgba {
            bytes: Arc::<[u8]>::from([0u8; 16]),
        },
    };
    state
        .present_frame(frame)
        .expect("present frame should work");

    assert_eq!(
        state.peek_latest_frame().map(|frame| frame.frame_seq),
        Some(1)
    );
    let drained = state.take_latest_renderable_frame();
    assert_eq!(drained.as_ref().map(|frame| frame.frame_seq), Some(1));
    assert!(state.take_latest_renderable_frame().is_none());
    assert!(state.peek_latest_frame().is_none());

    let frame = XbxRenderFrame {
        width: 2,
        height: 2,
        frame_seq: 3,
        rendered_at_ms: 1_016.0,
        rtp_timestamp: Some(3),
        recovery_epoch_tag: None,
        recovery_owner_rtp_timestamp: None,
        is_keyframe: false,
        frame_recovery_disposition: Some("repairing".to_string()),
        frame_unrecoverable_reason: None,
        presentation_value_role: None,
        pixel_data: XbxEngineRenderPixelData::Rgba {
            bytes: Arc::<[u8]>::from([1u8; 16]),
        },
    };
    state
        .present_frame(frame)
        .expect("present frame should work");
    assert_eq!(
        state.peek_latest_frame().map(|frame| frame.frame_seq),
        Some(3)
    );
    let drained = state.take_latest_renderable_frame();
    assert_eq!(drained.as_ref().map(|frame| frame.frame_seq), Some(3));
    assert!(state.take_latest_renderable_frame().is_none());
}

#[test]
fn present_frame_reports_overwritten_pending_metadata() {
    let mut state = XbxRenderState::default();
    let first_frame = XbxRenderFrame {
        width: 2,
        height: 2,
        frame_seq: 10,
        rendered_at_ms: 1_000.0,
        rtp_timestamp: Some(10),
        recovery_epoch_tag: None,
        recovery_owner_rtp_timestamp: None,
        is_keyframe: false,
        frame_recovery_disposition: Some("repairing".to_string()),
        frame_unrecoverable_reason: None,
        presentation_value_role: None,
        pixel_data: XbxEngineRenderPixelData::Rgba {
            bytes: Arc::<[u8]>::from([0u8; 16]),
        },
    };
    let second_frame = XbxRenderFrame {
        width: 4,
        height: 4,
        frame_seq: 11,
        rendered_at_ms: 1_016.0,
        rtp_timestamp: Some(11),
        recovery_epoch_tag: None,
        recovery_owner_rtp_timestamp: None,
        is_keyframe: false,
        frame_recovery_disposition: Some("repairing".to_string()),
        frame_unrecoverable_reason: None,
        presentation_value_role: None,
        pixel_data: XbxEngineRenderPixelData::Rgba {
            bytes: Arc::<[u8]>::from([1u8; 64]),
        },
    };

    let (_, first_outcome) = state
        .present_frame(first_frame)
        .expect("first present should work");
    let (_, second_outcome) = state
        .present_frame(second_frame)
        .expect("second present should work");

    assert_eq!(
        first_outcome,
        XbxPresentFrameOutcome {
            overwritten_pending_frame: false,
            overwritten_frame_seq: None,
            overwritten_frame_width: None,
            overwritten_frame_height: None,
        }
    );
    assert_eq!(
        second_outcome,
        XbxPresentFrameOutcome {
            overwritten_pending_frame: true,
            overwritten_frame_seq: Some(10),
            overwritten_frame_width: Some(2),
            overwritten_frame_height: Some(2),
        }
    );
}

#[test]
fn present_frame_overwrites_pending_frame_even_when_existing_has_higher_recovery_epoch() {
    let mut state = XbxRenderState::default();
    let higher_epoch_frame = XbxRenderFrame {
        width: 2,
        height: 2,
        frame_seq: 200,
        rendered_at_ms: 1_000.0,
        rtp_timestamp: Some(200),
        recovery_epoch_tag: Some(4),
        recovery_owner_rtp_timestamp: None,
        is_keyframe: false,
        frame_recovery_disposition: Some("repairing".to_string()),
        frame_unrecoverable_reason: None,
        presentation_value_role: None,
        pixel_data: XbxEngineRenderPixelData::Rgba {
            bytes: Arc::<[u8]>::from([0u8; 16]),
        },
    };
    let lower_epoch_frame = XbxRenderFrame {
        width: 2,
        height: 2,
        frame_seq: 150,
        rendered_at_ms: 1_016.0,
        rtp_timestamp: Some(150),
        recovery_epoch_tag: Some(3),
        recovery_owner_rtp_timestamp: None,
        is_keyframe: true,
        frame_recovery_disposition: Some("repairing".to_string()),
        frame_unrecoverable_reason: None,
        presentation_value_role: None,
        pixel_data: XbxEngineRenderPixelData::Rgba {
            bytes: Arc::<[u8]>::from([1u8; 16]),
        },
    };

    let _ = state
        .present_frame(higher_epoch_frame)
        .expect("first present should work");
    let (_, outcome) = state
        .present_frame(lower_epoch_frame)
        .expect("second present should resolve");

    assert!(outcome.overwritten_pending_frame);
    assert_eq!(outcome.overwritten_frame_seq, Some(200));
    assert_eq!(
        state.peek_latest_frame().map(|frame| frame.frame_seq),
        Some(150)
    );
}

#[test]
fn present_frame_overwrites_pending_owner_frame_in_same_recovery_epoch() {
    let mut state = XbxRenderState::default();
    let owner_frame = XbxRenderFrame {
        width: 2,
        height: 2,
        frame_seq: 220,
        rendered_at_ms: 1_000.0,
        rtp_timestamp: Some(220),
        recovery_epoch_tag: Some(6),
        recovery_owner_rtp_timestamp: Some(220),
        is_keyframe: true,
        frame_recovery_disposition: Some("repairing".to_string()),
        frame_unrecoverable_reason: None,
        presentation_value_role: None,
        pixel_data: XbxEngineRenderPixelData::Rgba {
            bytes: Arc::<[u8]>::from([2u8; 16]),
        },
    };
    let non_owner_frame = XbxRenderFrame {
        width: 2,
        height: 2,
        frame_seq: 221,
        rendered_at_ms: 1_016.0,
        rtp_timestamp: Some(221),
        recovery_epoch_tag: Some(6),
        recovery_owner_rtp_timestamp: Some(220),
        is_keyframe: false,
        frame_recovery_disposition: Some("repairing".to_string()),
        frame_unrecoverable_reason: None,
        presentation_value_role: None,
        pixel_data: XbxEngineRenderPixelData::Rgba {
            bytes: Arc::<[u8]>::from([3u8; 16]),
        },
    };

    let _ = state
        .present_frame(owner_frame)
        .expect("owner frame should be accepted");
    let (_, outcome) = state
        .present_frame(non_owner_frame)
        .expect("non-owner frame should be compared");

    assert!(outcome.overwritten_pending_frame);
    assert_eq!(outcome.overwritten_frame_seq, Some(220));
    assert_eq!(
        state
            .peek_latest_frame()
            .and_then(|frame| frame.rtp_timestamp),
        Some(221)
    );
}

#[test]
fn newer_plain_delta_overwrites_pending_owner_repairing_frame_in_same_recovery_epoch() {
    let mut state = XbxRenderState::default();
    let owner_frame = XbxRenderFrame {
        width: 2,
        height: 2,
        frame_seq: 260,
        rendered_at_ms: 1_000.0,
        rtp_timestamp: Some(260),
        recovery_epoch_tag: Some(7),
        recovery_owner_rtp_timestamp: Some(260),
        is_keyframe: true,
        frame_recovery_disposition: Some("repairing".to_string()),
        frame_unrecoverable_reason: None,
        presentation_value_role: None,
        pixel_data: XbxEngineRenderPixelData::Rgba {
            bytes: Arc::<[u8]>::from([13u8; 16]),
        },
    };
    let plain_delta = XbxRenderFrame {
        width: 2,
        height: 2,
        frame_seq: 261,
        rendered_at_ms: 1_016.0,
        rtp_timestamp: Some(261),
        recovery_epoch_tag: Some(7),
        recovery_owner_rtp_timestamp: Some(260),
        is_keyframe: false,
        frame_recovery_disposition: None,
        frame_unrecoverable_reason: None,
        presentation_value_role: None,
        pixel_data: XbxEngineRenderPixelData::Rgba {
            bytes: Arc::<[u8]>::from([14u8; 16]),
        },
    };

    let _ = state
        .present_frame(owner_frame)
        .expect("owner frame should be accepted");
    let (_, outcome) = state
        .present_frame(plain_delta)
        .expect("plain delta should be compared against owner frame");

    assert!(outcome.overwritten_pending_frame);
    assert_eq!(outcome.overwritten_frame_seq, Some(260));
    assert_eq!(
        state
            .peek_latest_frame()
            .and_then(|frame| frame.rtp_timestamp),
        Some(261)
    );
}

#[test]
fn plain_delta_replaces_pending_non_owner_frame_in_same_epoch() {
    let mut state = XbxRenderState::default();
    let non_owner_frame = XbxRenderFrame {
        width: 2,
        height: 2,
        frame_seq: 300,
        rendered_at_ms: 1_000.0,
        rtp_timestamp: Some(301),
        recovery_epoch_tag: Some(9),
        recovery_owner_rtp_timestamp: Some(300),
        is_keyframe: false,
        frame_recovery_disposition: Some("repairing".to_string()),
        frame_unrecoverable_reason: None,
        presentation_value_role: None,
        pixel_data: XbxEngineRenderPixelData::Rgba {
            bytes: Arc::<[u8]>::from([15u8; 16]),
        },
    };
    let plain_delta = XbxRenderFrame {
        width: 2,
        height: 2,
        frame_seq: 301,
        rendered_at_ms: 1_016.0,
        rtp_timestamp: Some(302),
        recovery_epoch_tag: Some(9),
        recovery_owner_rtp_timestamp: Some(300),
        is_keyframe: false,
        frame_recovery_disposition: None,
        frame_unrecoverable_reason: None,
        presentation_value_role: None,
        pixel_data: XbxEngineRenderPixelData::Rgba {
            bytes: Arc::<[u8]>::from([16u8; 16]),
        },
    };

    let _ = state
        .present_frame(non_owner_frame)
        .expect("non-owner frame should be accepted");
    let (_, outcome) = state
        .present_frame(plain_delta)
        .expect("plain delta should replace pending non-owner frame");

    assert_eq!(outcome.overwritten_frame_seq, Some(300));
    assert_eq!(
        state
            .peek_latest_frame()
            .and_then(|frame| frame.rtp_timestamp),
        Some(302)
    );
}

#[test]
fn present_frame_prefers_newer_plain_delta_over_old_keyframe_without_recovery_signal() {
    let mut state = XbxRenderState::default();
    let keyframe = XbxRenderFrame {
        width: 2,
        height: 2,
        frame_seq: 40,
        rendered_at_ms: 1_000.0,
        rtp_timestamp: Some(400),
        recovery_epoch_tag: None,
        recovery_owner_rtp_timestamp: None,
        is_keyframe: true,
        frame_recovery_disposition: None,
        frame_unrecoverable_reason: None,
        presentation_value_role: None,
        pixel_data: XbxEngineRenderPixelData::Rgba {
            bytes: Arc::<[u8]>::from([4u8; 16]),
        },
    };
    let newer_plain_delta = XbxRenderFrame {
        width: 2,
        height: 2,
        frame_seq: 41,
        rendered_at_ms: 1_016.0,
        rtp_timestamp: Some(401),
        recovery_epoch_tag: None,
        recovery_owner_rtp_timestamp: None,
        is_keyframe: false,
        frame_recovery_disposition: None,
        frame_unrecoverable_reason: None,
        presentation_value_role: None,
        pixel_data: XbxEngineRenderPixelData::Rgba {
            bytes: Arc::<[u8]>::from([5u8; 16]),
        },
    };

    let _ = state
        .present_frame(keyframe)
        .expect("keyframe should be accepted");
    let _ = state
        .present_frame(newer_plain_delta)
        .expect("plain delta should be compared");

    assert_eq!(
        state.peek_latest_frame().map(|frame| frame.frame_seq),
        Some(41)
    );
}

#[test]
fn render_signal_snapshot_uses_latest_shadow_frame_time() {
    let mut state = XbxRenderState::default();
    state
        .present_frame(XbxRenderFrame {
            width: 2,
            height: 2,
            frame_seq: 1,
            rendered_at_ms: 1_000.0,
            rtp_timestamp: Some(1),
            recovery_epoch_tag: None,
            recovery_owner_rtp_timestamp: None,
            is_keyframe: true,
            frame_recovery_disposition: Some("repairing".to_string()),
            frame_unrecoverable_reason: None,
            presentation_value_role: None,
            pixel_data: XbxEngineRenderPixelData::Rgba {
                bytes: Arc::<[u8]>::from([0u8; 16]),
            },
        })
        .expect("present frame should work");

    let snapshot = state.render_signal_snapshot(1_200.0);

    assert_eq!(snapshot.latest_present_time_ms, Some(1_000.0));
    assert_eq!(snapshot.renderer_stalled, Some(false));
}

#[test]
fn repairing_frame_beats_newer_unrecoverable_plain_delta() {
    let mut state = XbxRenderState::default();
    let repairing = XbxRenderFrame {
        width: 2,
        height: 2,
        frame_seq: 320,
        rendered_at_ms: 1_000.0,
        rtp_timestamp: Some(320),
        recovery_epoch_tag: Some(9),
        recovery_owner_rtp_timestamp: None,
        is_keyframe: false,
        frame_recovery_disposition: Some("repairing".to_string()),
        frame_unrecoverable_reason: None,
        presentation_value_role: None,
        pixel_data: XbxEngineRenderPixelData::Rgba {
            bytes: Arc::<[u8]>::from([7u8; 16]),
        },
    };
    let plain_delta = XbxRenderFrame {
        width: 2,
        height: 2,
        frame_seq: 321,
        rendered_at_ms: 1_008.0,
        rtp_timestamp: Some(321),
        recovery_epoch_tag: Some(9),
        recovery_owner_rtp_timestamp: None,
        is_keyframe: false,
        frame_recovery_disposition: Some("steady".to_string()),
        frame_unrecoverable_reason: Some("continuationOnly".to_string()),
        presentation_value_role: None,
        pixel_data: XbxEngineRenderPixelData::Rgba {
            bytes: Arc::<[u8]>::from([8u8; 16]),
        },
    };

    let _ = state
        .present_frame(repairing)
        .expect("repairing frame accepted");
    let (_, outcome) = state
        .present_frame(plain_delta)
        .expect("plain delta compared against repairing frame");

    assert!(outcome.overwritten_pending_frame);
    assert_eq!(outcome.overwritten_frame_seq, Some(320));
    assert_eq!(
        state.peek_latest_frame().map(|frame| frame.frame_seq),
        Some(321)
    );
}

#[test]
fn plain_delta_replaces_old_repairing_continuation_in_same_epoch() {
    let mut state = XbxRenderState::default();
    let repairing = XbxRenderFrame {
        width: 2,
        height: 2,
        frame_seq: 330,
        rendered_at_ms: 1_000.0,
        rtp_timestamp: Some(330),
        recovery_epoch_tag: Some(10),
        recovery_owner_rtp_timestamp: None,
        is_keyframe: false,
        frame_recovery_disposition: Some("repairing".to_string()),
        frame_unrecoverable_reason: None,
        presentation_value_role: None,
        pixel_data: XbxEngineRenderPixelData::Rgba {
            bytes: Arc::<[u8]>::from([9u8; 16]),
        },
    };
    let plain_delta = XbxRenderFrame {
        width: 2,
        height: 2,
        frame_seq: 331,
        rendered_at_ms: 1_008.0,
        rtp_timestamp: Some(331),
        recovery_epoch_tag: Some(10),
        recovery_owner_rtp_timestamp: None,
        is_keyframe: false,
        frame_recovery_disposition: None,
        frame_unrecoverable_reason: None,
        presentation_value_role: None,
        pixel_data: XbxEngineRenderPixelData::Rgba {
            bytes: Arc::<[u8]>::from([10u8; 16]),
        },
    };

    let _ = state
        .present_frame(repairing)
        .expect("repairing frame accepted");
    let _ = state
        .present_frame(plain_delta)
        .expect("plain delta compared against repairing frame");

    assert_eq!(
        state.peek_latest_frame().map(|frame| frame.frame_seq),
        Some(331)
    );
    assert_eq!(
        state
            .peek_latest_frame()
            .and_then(|frame| frame.frame_recovery_disposition.as_deref()),
        None
    );
}

#[test]
fn owner_rebuilding_supply_replaces_non_owner_candidate_in_same_epoch() {
    let mut state = XbxRenderState::default();
    let non_owner = XbxRenderFrame {
        width: 2,
        height: 2,
        frame_seq: 340,
        rendered_at_ms: 1_000.0,
        rtp_timestamp: Some(340),
        recovery_epoch_tag: Some(11),
        recovery_owner_rtp_timestamp: Some(320),
        is_keyframe: false,
        frame_recovery_disposition: Some("rebuilding-supply".to_string()),
        frame_unrecoverable_reason: None,
        presentation_value_role: None,
        pixel_data: XbxEngineRenderPixelData::Rgba {
            bytes: Arc::<[u8]>::from([11u8; 16]),
        },
    };
    let owner = XbxRenderFrame {
        width: 2,
        height: 2,
        frame_seq: 320,
        rendered_at_ms: 1_008.0,
        rtp_timestamp: Some(320),
        recovery_epoch_tag: Some(11),
        recovery_owner_rtp_timestamp: Some(320),
        is_keyframe: false,
        frame_recovery_disposition: Some("rebuilding-supply".to_string()),
        frame_unrecoverable_reason: None,
        presentation_value_role: None,
        pixel_data: XbxEngineRenderPixelData::Rgba {
            bytes: Arc::<[u8]>::from([12u8; 16]),
        },
    };

    let _ = state
        .present_frame(non_owner)
        .expect("non-owner candidate accepted");
    let _ = state
        .present_frame(owner)
        .expect("owner candidate compared against non-owner candidate");

    assert_eq!(
        state.peek_latest_frame().map(|frame| frame.frame_seq),
        Some(320)
    );
    assert_eq!(
        state
            .peek_latest_frame()
            .and_then(|frame| frame.recovery_owner_rtp_timestamp),
        Some(320)
    );
}

#[test]
fn render_signal_snapshot_marks_stall_after_threshold() {
    let mut state = XbxRenderState::default();
    let frame = XbxRenderFrame {
        width: 2,
        height: 2,
        frame_seq: 1,
        rendered_at_ms: 1_000.0,
        rtp_timestamp: Some(1),
        recovery_epoch_tag: None,
        recovery_owner_rtp_timestamp: None,
        is_keyframe: true,
        frame_recovery_disposition: Some("repairing".to_string()),
        frame_unrecoverable_reason: None,
        presentation_value_role: None,
        pixel_data: XbxEngineRenderPixelData::Rgba {
            bytes: Arc::<[u8]>::from([0u8; 16]),
        },
    };
    state
        .present_frame(frame)
        .expect("present frame should work");
    let snapshot = state.render_signal_snapshot(2_700.0);
    assert_eq!(snapshot.latest_present_time_ms, Some(1_000.0));
    assert_eq!(snapshot.renderer_stalled, Some(true));
}

#[test]
fn render_signal_snapshot_reports_latest_present_time_when_recent() {
    let mut state = XbxRenderState::default();
    for index in 0..4u64 {
        state
            .present_frame(XbxRenderFrame {
                width: 2,
                height: 2,
                frame_seq: index + 1,
                rendered_at_ms: 1_000.0 + index as f64 * 16.0,
                rtp_timestamp: Some(index as u32 + 1),
                recovery_epoch_tag: None,
                recovery_owner_rtp_timestamp: None,
                is_keyframe: false,
                frame_recovery_disposition: Some("repairing".to_string()),
                frame_unrecoverable_reason: None,
                presentation_value_role: None,
                pixel_data: XbxEngineRenderPixelData::Rgba {
                    bytes: Arc::<[u8]>::from([0u8; 16]),
                },
            })
            .expect("present frame should work");
    }

    let snapshot = state.render_signal_snapshot(1_050.0);
    assert_eq!(snapshot.latest_present_time_ms, Some(1_048.0));
    assert_eq!(snapshot.renderer_stalled, Some(false));
}

#[test]
fn render_signal_snapshot_marks_stall_when_latest_present_is_stale() {
    let mut state = XbxRenderState::default();
    for index in 0..4u64 {
        state
            .present_frame(XbxRenderFrame {
                width: 2,
                height: 2,
                frame_seq: index + 1,
                rendered_at_ms: 1_000.0 + index as f64 * 16.0,
                rtp_timestamp: Some(index as u32 + 1),
                recovery_epoch_tag: None,
                recovery_owner_rtp_timestamp: None,
                is_keyframe: false,
                frame_recovery_disposition: Some("repairing".to_string()),
                frame_unrecoverable_reason: None,
                presentation_value_role: None,
                pixel_data: XbxEngineRenderPixelData::Rgba {
                    bytes: Arc::<[u8]>::from([0u8; 16]),
                },
            })
            .expect("present frame should work");
    }

    let snapshot = state.render_signal_snapshot(2_200.0);
    assert_eq!(snapshot.latest_present_time_ms, Some(1_048.0));
    assert_eq!(snapshot.renderer_stalled, Some(false));

    let stalled_snapshot = state.render_signal_snapshot(2_700.0);
    assert_eq!(stalled_snapshot.latest_present_time_ms, Some(1_048.0));
    assert_eq!(stalled_snapshot.renderer_stalled, Some(true));
}

#[test]
fn render_mailbox_state_recovers_after_mailbox_overwrite_is_cleared() {
    let mut state = XbxRenderState::default();
    let mk_frame = |frame_seq: u64, rendered_at_ms: f64| XbxRenderFrame {
        width: 2,
        height: 2,
        frame_seq,
        rendered_at_ms,
        rtp_timestamp: Some(frame_seq as u32),
        recovery_epoch_tag: None,
        recovery_owner_rtp_timestamp: None,
        is_keyframe: false,
        frame_recovery_disposition: Some("repairing".to_string()),
        frame_unrecoverable_reason: None,
        presentation_value_role: None,
        pixel_data: XbxEngineRenderPixelData::Rgba {
            bytes: Arc::<[u8]>::from([0u8; 16]),
        },
    };

    state
        .present_frame(mk_frame(10, 1_000.0))
        .expect("first present should work");
    state
        .present_frame(mk_frame(11, 1_016.0))
        .expect("second present should overwrite");

    let pressured = state
        .latest_render_mailbox_decision()
        .expect("overwrite decision");
    assert_eq!(
        pressured.state,
        super::XbxRenderMailboxState::LatestOverwrite
    );
    assert_eq!(pressured.action, "replace");
    assert_eq!(pressured.detail, "mailboxOverwrite");
    assert_eq!(pressured.frame_seq, Some(10));

    let drained = state.take_latest_renderable_frame();
    assert_eq!(drained.as_ref().map(|frame| frame.frame_seq), Some(11));
    state
        .present_frame(mk_frame(12, 1_032.0))
        .expect("third present should recover");
    let recovered = state
        .latest_render_mailbox_decision()
        .expect("recovered decision");
    assert_eq!(recovered.state, super::XbxRenderMailboxState::Nominal);
    assert_eq!(recovered.action, "accept");
    assert_eq!(recovered.detail, "mailboxRecovered");
    assert_eq!(recovered.frame_seq, Some(12));
}

#[test]
fn render_mailbox_state_stays_latest_overwrite_while_pending_backlog_exists() {
    let mut state = XbxRenderState::default();
    let mk_frame = |frame_seq: u64, rendered_at_ms: f64| XbxRenderFrame {
        width: 2,
        height: 2,
        frame_seq,
        rendered_at_ms,
        rtp_timestamp: Some(frame_seq as u32),
        recovery_epoch_tag: None,
        recovery_owner_rtp_timestamp: None,
        is_keyframe: false,
        frame_recovery_disposition: Some("repairing".to_string()),
        frame_unrecoverable_reason: None,
        presentation_value_role: None,
        pixel_data: XbxEngineRenderPixelData::Rgba {
            bytes: Arc::<[u8]>::from([0u8; 16]),
        },
    };

    state
        .present_frame(mk_frame(20, 1_000.0))
        .expect("first present should work");
    state
        .present_frame(mk_frame(21, 1_016.0))
        .expect("second present should overwrite");
    state
        .present_frame(mk_frame(22, 1_032.0))
        .expect("third present should continue overwriting");

    let pressured = state
        .latest_render_mailbox_decision()
        .expect("overwrite decision should exist");
    assert_eq!(
        pressured.state,
        super::XbxRenderMailboxState::LatestOverwrite
    );
    assert_eq!(pressured.action, "replace");
    assert_eq!(pressured.detail, "mailboxOverwrite");
    assert_eq!(pressured.frame_seq, Some(21));
    assert_eq!(
        state.peek_latest_frame().map(|frame| frame.frame_seq),
        Some(22)
    );
}
