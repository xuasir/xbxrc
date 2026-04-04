    use std::sync::Arc;

    use super::{XbxPresentFrameOutcome, XbxRenderFrame, XbxRenderState};
    use crate::XbxEngineRenderPixelData;

    #[test]
    fn latest_slot_supports_peek_take_and_ack() {
        let mut state = XbxRenderState::default();
        let frame = XbxRenderFrame {
            width: 2,
            height: 2,
            frame_seq: 1,
            rendered_at_ms: 1_000.0,
            rtp_timestamp: Some(1),
            is_keyframe: true,
            frame_recovery_disposition: Some("repairing".to_string()),
            frame_unrecoverable_reason: None,
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
        assert!(!state.acknowledge_latest_frame(2));
        assert!(state.acknowledge_latest_frame(1));
        assert!(state.peek_latest_frame().is_none());

        let frame = XbxRenderFrame {
            width: 2,
            height: 2,
            frame_seq: 3,
            rendered_at_ms: 1_016.0,
            rtp_timestamp: Some(3),
            is_keyframe: false,
            frame_recovery_disposition: Some("repairing".to_string()),
            frame_unrecoverable_reason: None,
            pixel_data: XbxEngineRenderPixelData::Rgba {
                bytes: Arc::<[u8]>::from([1u8; 16]),
            },
        };
        state
            .present_frame(frame)
            .expect("present frame should work");
        assert_eq!(
            state.take_latest_frame().map(|frame| frame.frame_seq),
            Some(3)
        );
        // take 不消费槽位，后续仍可 peek/ack。
        assert_eq!(
            state.peek_latest_frame().map(|frame| frame.frame_seq),
            Some(3)
        );
        assert!(state.acknowledge_latest_frame(3));
        assert!(state.peek_latest_frame().is_none());
    }

    #[test]
    fn present_frame_reports_overwritten_latest_metadata() {
        let mut state = XbxRenderState::default();
        let first_frame = XbxRenderFrame {
            width: 2,
            height: 2,
            frame_seq: 1,
            rendered_at_ms: 1_000.0,
            rtp_timestamp: Some(1),
            is_keyframe: true,
            frame_recovery_disposition: Some("repairing".to_string()),
            frame_unrecoverable_reason: None,
            pixel_data: XbxEngineRenderPixelData::Rgba {
                bytes: Arc::<[u8]>::from([0u8; 16]),
            },
        };
        let second_frame = XbxRenderFrame {
            width: 4,
            height: 4,
            frame_seq: 2,
            rendered_at_ms: 1_016.0,
            rtp_timestamp: Some(2),
            is_keyframe: false,
            frame_recovery_disposition: Some("repairing".to_string()),
            frame_unrecoverable_reason: None,
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
                overwritten_previous_latest: false,
                overwritten_frame_seq: None,
                overwritten_frame_width: None,
                overwritten_frame_height: None,
            }
        );
        assert_eq!(
            second_outcome,
            XbxPresentFrameOutcome {
                overwritten_previous_latest: true,
                overwritten_frame_seq: Some(1),
                overwritten_frame_width: Some(2),
                overwritten_frame_height: Some(2),
            }
        );
    }

    #[test]
    fn acknowledge_keeps_last_present_time_for_snapshot() {
        let mut state = XbxRenderState::default();
        state
            .present_frame(XbxRenderFrame {
                width: 2,
                height: 2,
                frame_seq: 1,
                rendered_at_ms: 1_000.0,
                rtp_timestamp: Some(1),
                is_keyframe: true,
                frame_recovery_disposition: Some("repairing".to_string()),
                frame_unrecoverable_reason: None,
                pixel_data: XbxEngineRenderPixelData::Rgba {
                    bytes: Arc::<[u8]>::from([0u8; 16]),
                },
            })
            .expect("present frame should work");

        assert!(state.acknowledge_latest_frame(1));
        let snapshot = state.render_signal_snapshot(1_200.0);

        assert_eq!(snapshot.latest_present_time_ms, Some(1_000.0));
        assert_eq!(snapshot.renderer_stalled, Some(false));
        assert!(state.peek_latest_frame().is_none());
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
            is_keyframe: true,
            frame_recovery_disposition: Some("repairing".to_string()),
            frame_unrecoverable_reason: None,
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
                    is_keyframe: index == 0,
                    frame_recovery_disposition: Some("repairing".to_string()),
                    frame_unrecoverable_reason: None,
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
                    is_keyframe: index == 0,
                    frame_recovery_disposition: Some("repairing".to_string()),
                    frame_unrecoverable_reason: None,
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
    fn render_candidate_state_recovers_after_latest_slot_overwrite_is_cleared() {
        let mut state = XbxRenderState::default();
        let mk_frame = |frame_seq: u64, rendered_at_ms: f64| XbxRenderFrame {
            width: 2,
            height: 2,
            frame_seq,
            rendered_at_ms,
            rtp_timestamp: Some(frame_seq as u32),
            is_keyframe: frame_seq == 1,
            frame_recovery_disposition: Some("repairing".to_string()),
            frame_unrecoverable_reason: None,
            pixel_data: XbxEngineRenderPixelData::Rgba {
                bytes: Arc::<[u8]>::from([0u8; 16]),
            },
        };

        state
            .present_frame(mk_frame(1, 1_000.0))
            .expect("first present should work");
        state
            .present_frame(mk_frame(2, 1_016.0))
            .expect("second present should overwrite");

        let pressured = state
            .latest_render_candidate_decision()
            .expect("overwrite decision");
        assert_eq!(
            pressured.state,
            super::XbxRenderCandidateState::LatestOverwrite
        );
        assert_eq!(pressured.action, "replace");
        assert_eq!(pressured.detail, "latestSlotOverwrite");
        assert_eq!(pressured.frame_seq, Some(1));

        assert!(state.acknowledge_latest_frame(2));
        state
            .present_frame(mk_frame(3, 1_032.0))
            .expect("third present should recover");
        let recovered = state
            .latest_render_candidate_decision()
            .expect("recovered decision");
        assert_eq!(recovered.state, super::XbxRenderCandidateState::Nominal);
        assert_eq!(recovered.action, "accept");
        assert_eq!(recovered.detail, "latestSlotRecovered");
        assert_eq!(recovered.frame_seq, Some(3));
    }

    #[test]
    fn render_candidate_state_stays_latest_overwrite_until_latest_slot_is_acknowledged() {
        let mut state = XbxRenderState::default();
        let mk_frame = |frame_seq: u64, rendered_at_ms: f64| XbxRenderFrame {
            width: 2,
            height: 2,
            frame_seq,
            rendered_at_ms,
            rtp_timestamp: Some(frame_seq as u32),
            is_keyframe: frame_seq == 1,
            frame_recovery_disposition: Some("repairing".to_string()),
            frame_unrecoverable_reason: None,
            pixel_data: XbxEngineRenderPixelData::Rgba {
                bytes: Arc::<[u8]>::from([0u8; 16]),
            },
        };

        state
            .present_frame(mk_frame(1, 1_000.0))
            .expect("first present should work");
        state
            .present_frame(mk_frame(2, 1_016.0))
            .expect("second present should overwrite");
        state
            .present_frame(mk_frame(3, 1_032.0))
            .expect("third present should continue overwriting");

        let pressured = state
            .latest_render_candidate_decision()
            .expect("overwrite decision should exist");
        assert_eq!(
            pressured.state,
            super::XbxRenderCandidateState::LatestOverwrite
        );
        assert_eq!(pressured.action, "replace");
        assert_eq!(pressured.detail, "latestSlotOverwrite");
        assert_eq!(pressured.frame_seq, Some(2));
        assert_eq!(state.peek_latest_frame().map(|frame| frame.frame_seq), Some(3));
    }
