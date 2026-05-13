use std::sync::Arc;

use super::{XbxPresentFrameOutcome, XbxRenderFrame, XbxRenderMailboxState, XbxRenderState};
use crate::XbxEngineRenderPixelData;

fn mk_frame(frame_seq: u64, rendered_at_ms: f64) -> XbxRenderFrame {
    XbxRenderFrame {
        width: 2,
        height: 2,
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
            bytes: Arc::<[u8]>::from([0u8; 16]),
        },
    }
}

#[test]
fn render_mailbox_exposes_single_latest_handoff() {
    let mut state = XbxRenderState::default();

    state
        .present_frame(mk_frame(10, 1_000.0))
        .expect("first frame should be accepted");
    state
        .present_frame(mk_frame(11, 1_016.0))
        .expect("second frame should be accepted");

    assert_eq!(
        state.peek_latest_frame().map(|frame| frame.frame_seq),
        Some(11)
    );
    assert_eq!(
        state
            .take_latest_renderable_frame()
            .map(|frame| frame.frame_seq),
        Some(11)
    );
    assert!(state.take_latest_renderable_frame().is_none());
}

#[test]
fn render_mailbox_reports_overwrite_when_pending_frame_is_replaced() {
    let mut state = XbxRenderState::default();

    let (_, first_outcome) = state
        .present_frame(mk_frame(20, 1_000.0))
        .expect("first frame should be accepted");
    let (_, second_outcome) = state
        .present_frame(mk_frame(21, 1_016.0))
        .expect("second frame should be accepted");

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
            overwritten_frame_seq: Some(20),
            overwritten_frame_width: Some(2),
            overwritten_frame_height: Some(2),
        }
    );

    let decision = state
        .latest_render_mailbox_decision()
        .expect("overwrite should be recorded");
    assert_eq!(decision.state, XbxRenderMailboxState::LatestOverwrite);
    assert_eq!(decision.action, "replace");
    assert_eq!(decision.frame_seq, Some(20));
}

#[test]
fn render_mailbox_returns_to_nominal_after_overwrite_is_drained() {
    let mut state = XbxRenderState::default();

    state
        .present_frame(mk_frame(30, 1_000.0))
        .expect("first frame should be accepted");
    state
        .present_frame(mk_frame(31, 1_016.0))
        .expect("second frame should overwrite pending");
    assert_eq!(
        state
            .take_latest_renderable_frame()
            .map(|frame| frame.frame_seq),
        Some(31)
    );

    state
        .present_frame(mk_frame(32, 1_032.0))
        .expect("third frame should be accepted");

    let decision = state
        .latest_render_mailbox_decision()
        .expect("recovery should be recorded");
    assert_eq!(decision.state, XbxRenderMailboxState::Nominal);
    assert_eq!(decision.action, "accept");
    assert_eq!(decision.detail, "mailboxRecovered");
    assert_eq!(decision.frame_seq, Some(32));
}
