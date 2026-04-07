use std::collections::VecDeque;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

use super::{
    XbxDecodeCandidateState, XbxDecodeWorkloadState, XbxVideoDecodeState, XbxVideoRecoveryEvent,
    XbxVideoRecoveryState,
};
use crate::media::video::decode::backend::{XbxVideoDecoderBackend, XbxVideoDecoderProbeSummary};
use crate::media::video::h264::inspection::{
    H264AccessUnitInspection, H264AccessUnitInspector, H264BootstrapRejectReason,
};
use crate::{
    api::backend::XbxEngineMediaRuntimeStats,
    media::video::render::renderer::XbxRenderFrame,
    media::video::render::renderer::XbxRenderState,
    media::video::test_fixtures::{
        make_bootstrap_assembled_frame, make_video_source_for_test, send_bootstrap_access_unit,
    },
    media::video::types::{DecodedFrame, EncodedFrame, FrameRecoveryDisposition},
    transport::rtc::stream::adapter_types::FrameSource,
    XbxEngineRenderPixelData,
};
use bytes::Bytes;

struct SpyHardwareDecoder;

impl XbxVideoDecoderBackend for SpyHardwareDecoder {
    fn backend_name(&self) -> &'static str {
        "spy"
    }

    fn decode(
        &mut self,
        _encoded_frame: EncodedFrame,
        _now_ms: f64,
    ) -> Result<Option<XbxRenderFrame>, crate::XbxEngineRuntimeError> {
        Ok(None)
    }
}

#[test]
fn request_local_decoder_reset_rebuilds_backend_and_updates_probe_snapshot() {
    let decoder = SpyHardwareDecoder;
    let reset_calls = Arc::new(AtomicUsize::new(0));
    let reset_calls_for_factory = reset_calls.clone();
    let decoder_factory = Box::new(move || {
        let call_index = reset_calls_for_factory.fetch_add(1, Ordering::Relaxed);
        let backend_name = if call_index == 0 {
            "replacement-1"
        } else {
            "replacement-2"
        };
        (
            Box::new(ScriptedHardwareDecoder {
                backend_name,
                decode_calls: Arc::new(AtomicUsize::new(0)),
                scripted_results: VecDeque::new(),
            }) as Box<dyn XbxVideoDecoderBackend>,
            XbxVideoDecoderProbeSummary {
                selected_backend_name: backend_name.to_string(),
                selected_backend_kind: "software".to_string(),
                fallback_count: 1,
                fallback_summary: Some("reset-recreate".to_string()),
            },
        )
    });
    let mut state =
        XbxVideoDecodeState::new_for_test_with_factory(20, 30, Box::new(decoder), decoder_factory);

    state
        .request_local_decoder_reset()
        .expect("decoder reset should succeed");

    assert_eq!(reset_calls.load(Ordering::Relaxed), 1);
    assert_eq!(state.decoder_backend_name(), "replacement-1");
    assert_eq!(state.decoder_reset_count(), 1);
    assert!(
        state.latest_decoder_reset_time_ms().is_some(),
        "decoder reset time should be recorded"
    );
    let probe = state
        .latest_decoder_probe()
        .expect("decoder probe snapshot should exist");
    assert_eq!(probe.observation_id, 1);
    assert_eq!(probe.selected_backend_name, "replacement-1");
    assert_eq!(probe.selected_backend_kind, "software");
    assert_eq!(probe.fallback_count, 1);
    assert_eq!(probe.fallback_summary.as_deref(), Some("reset-recreate"));
    assert_eq!(
        state.recovery_state(),
        XbxVideoRecoveryState::WaitingKeyframe
    );
    let transition = state
        .latest_recovery_transition()
        .expect("recovery transition should exist");
    assert_eq!(
        transition.event,
        XbxVideoRecoveryEvent::ExternalDecoderResetRequested
    );
    assert_eq!(transition.to_state, XbxVideoRecoveryState::WaitingKeyframe);
}

#[test]
fn recovery_fsm_moves_from_waiting_keyframe_to_recovering_then_nominal() {
    let decode_calls = Arc::new(AtomicUsize::new(0));
    let mut scripted_results = VecDeque::new();
    scripted_results.push_back(Ok(Some(XbxRenderFrame {
        width: 2,
        height: 2,
        frame_seq: 101,
        rendered_at_ms: 101.0,
        rtp_timestamp: Some(101),
        is_keyframe: true,
        frame_recovery_disposition: Some("repairing".to_string()),
        frame_unrecoverable_reason: None,
        pixel_data: XbxEngineRenderPixelData::Rgba {
            bytes: Arc::<[u8]>::from([9u8; 16]),
        },
    })));
    scripted_results.push_back(Ok(Some(XbxRenderFrame {
        width: 2,
        height: 2,
        frame_seq: 102,
        rendered_at_ms: 102.0,
        rtp_timestamp: Some(102),
        is_keyframe: false,
        frame_recovery_disposition: Some("repairing".to_string()),
        frame_unrecoverable_reason: None,
        pixel_data: XbxEngineRenderPixelData::Rgba {
            bytes: Arc::<[u8]>::from([8u8; 16]),
        },
    })));
    let decoder = SpyHardwareDecoder;
    let decode_calls_for_factory = decode_calls.clone();
    let mut scripted_results_for_factory = Some(scripted_results);
    let decoder_factory = Box::new(move || {
        let scripted_results = scripted_results_for_factory.take().unwrap_or_default();
        (
            Box::new(ScriptedHardwareDecoder {
                backend_name: "scripted-recreated",
                decode_calls: decode_calls_for_factory.clone(),
                scripted_results,
            }) as Box<dyn XbxVideoDecoderBackend>,
            XbxVideoDecoderProbeSummary {
                selected_backend_name: "scripted-recreated".to_string(),
                selected_backend_kind: "software".to_string(),
                fallback_count: 1,
                fallback_summary: Some("external-reset-recreate".to_string()),
            },
        )
    });
    let mut state =
        XbxVideoDecodeState::new_for_test_with_factory(20, 30, Box::new(decoder), decoder_factory);

    state
        .request_local_decoder_reset()
        .expect("decoder reset should succeed");
    assert_eq!(
        state.recovery_state(),
        XbxVideoRecoveryState::WaitingKeyframe
    );

    assert!(state
        .process_encoded_frame(make_encoded_frame(false), 1_000.0)
        .is_none());
    assert_eq!(
        state.recovery_state(),
        XbxVideoRecoveryState::WaitingKeyframe
    );

    assert!(state
        .process_encoded_frame(make_encoded_frame(true), 1_016.0)
        .is_none());
    assert_eq!(state.recovery_state(), XbxVideoRecoveryState::Recovering);
    assert_eq!(
        state
            .peek_decoded_frame()
            .map(|frame| frame.surface.frame_seq),
        Some(1)
    );
    assert_eq!(state.decoded_frame_queue_len(), 1);

    assert!(state
        .process_encoded_frame(make_encoded_frame(false), 1_032.0)
        .is_none());
    assert_eq!(state.recovery_state(), XbxVideoRecoveryState::Nominal);
    assert_eq!(state.decoded_frame_queue_len(), 2);
    assert_eq!(
        state
            .peek_decoded_frame()
            .map(|frame| frame.surface.frame_seq),
        Some(1)
    );

    let transition = state
        .latest_recovery_transition()
        .expect("latest recovery transition should exist");
    assert_eq!(transition.event, XbxVideoRecoveryEvent::RecoverySettled);
    assert_eq!(transition.from_state, XbxVideoRecoveryState::Recovering);
    assert_eq!(transition.to_state, XbxVideoRecoveryState::Nominal);
    assert_eq!(decode_calls.load(Ordering::Relaxed), 2);
}

#[test]
fn hardware_decode_failure_escalates_recovery_state_to_waiting_keyframe() {
    let decode_calls = Arc::new(AtomicUsize::new(0));
    let replacement_decode_calls = Arc::new(AtomicUsize::new(0));
    let mut scripted_results = VecDeque::new();
    scripted_results.push_back(Err(crate::XbxEngineRuntimeError::new(
        "xbxEngineCreateVideoFormatDescriptionFailed:status=-12909",
    )));
    let decoder = ScriptedHardwareDecoder {
        backend_name: "scripted",
        decode_calls: decode_calls.clone(),
        scripted_results,
    };
    let replacement_decode_calls_for_factory = replacement_decode_calls.clone();
    let decoder_factory = Box::new(move || {
        (
            Box::new(ScriptedHardwareDecoder {
                backend_name: "replacement",
                decode_calls: replacement_decode_calls_for_factory.clone(),
                scripted_results: VecDeque::new(),
            }) as Box<dyn XbxVideoDecoderBackend>,
            XbxVideoDecoderProbeSummary {
                selected_backend_name: "replacement".to_string(),
                selected_backend_kind: "software".to_string(),
                fallback_count: 1,
                fallback_summary: Some("scripted(hardware/initialization-failed)".to_string()),
            },
        )
    });
    let mut state =
        XbxVideoDecodeState::new_for_test_with_factory(20, 30, Box::new(decoder), decoder_factory);

    let result = state.process_encoded_frame(make_encoded_frame(true), 2_000.0);
    assert!(result.is_none());
    assert_eq!(
        state.recovery_state(),
        XbxVideoRecoveryState::WaitingKeyframe
    );
    let transition = state
        .latest_recovery_transition()
        .expect("recovery transition should exist after backend failure");
    assert_eq!(
        transition.event,
        XbxVideoRecoveryEvent::BackendFailureEscalated
    );
    assert_eq!(transition.status, Some(-12909));
    assert_eq!(decode_calls.load(Ordering::Relaxed), 1);
    assert_eq!(state.decoder_backend_name(), "replacement");
}

#[test]
fn repeated_nonfatal_decode_failures_escalate_to_waiting_keyframe_on_third_failure() {
    let decode_calls = Arc::new(AtomicUsize::new(0));
    let replacement_decode_calls = Arc::new(AtomicUsize::new(0));
    let mut scripted_results = VecDeque::new();
    scripted_results.push_back(Err(crate::XbxEngineRuntimeError::new(
        "decode failed status=-1",
    )));
    scripted_results.push_back(Err(crate::XbxEngineRuntimeError::new(
        "decode failed status=-1",
    )));
    scripted_results.push_back(Err(crate::XbxEngineRuntimeError::new(
        "decode failed status=-1",
    )));
    let decoder = ScriptedHardwareDecoder {
        backend_name: "scripted",
        decode_calls: decode_calls.clone(),
        scripted_results,
    };
    let replacement_decode_calls_for_factory = replacement_decode_calls.clone();
    let decoder_factory = Box::new(move || {
        (
            Box::new(ScriptedHardwareDecoder {
                backend_name: "replacement",
                decode_calls: replacement_decode_calls_for_factory.clone(),
                scripted_results: VecDeque::new(),
            }) as Box<dyn XbxVideoDecoderBackend>,
            XbxVideoDecoderProbeSummary {
                selected_backend_name: "replacement".to_string(),
                selected_backend_kind: "software".to_string(),
                fallback_count: 1,
                fallback_summary: Some("scripted(hardware/initialization-failed)".to_string()),
            },
        )
    });
    let mut state =
        XbxVideoDecodeState::new_for_test_with_factory(20, 30, Box::new(decoder), decoder_factory);

    assert!(state
        .process_encoded_frame(make_encoded_frame(true), 3_000.0)
        .is_none());
    assert_eq!(state.recovery_state(), XbxVideoRecoveryState::Nominal);

    assert!(state
        .process_encoded_frame(make_encoded_frame(true), 3_016.0)
        .is_none());
    assert_eq!(state.recovery_state(), XbxVideoRecoveryState::Nominal);

    assert!(state
        .process_encoded_frame(make_encoded_frame(true), 3_032.0)
        .is_none());
    assert_eq!(
        state.recovery_state(),
        XbxVideoRecoveryState::WaitingKeyframe
    );
    let transition = state
        .latest_recovery_transition()
        .expect("recovery transition should exist after third failure");
    assert_eq!(
        transition.event,
        XbxVideoRecoveryEvent::BackendFailureEscalated
    );
    assert_eq!(decode_calls.load(Ordering::Relaxed), 3);
    assert_eq!(state.decoder_backend_name(), "replacement");
}

#[test]
fn decoded_queue_keeps_latest_two_frames_under_pressure() {
    let decoder = SpyHardwareDecoder;
    let mut state = XbxVideoDecodeState::new_for_test(20, 30, Box::new(decoder));

    for seq in 1..=3 {
        state.enqueue_decoded_frame_for_test(XbxRenderFrame {
            width: 2,
            height: 2,
            frame_seq: seq,
            rendered_at_ms: seq as f64,
            rtp_timestamp: Some(seq as u32),
            is_keyframe: seq == 1,
            frame_recovery_disposition: Some("repairing".to_string()),
            frame_unrecoverable_reason: None,
            pixel_data: XbxEngineRenderPixelData::Rgba {
                bytes: Arc::<[u8]>::from([0u8; 16]),
            },
        });
    }

    assert_eq!(state.decoded_frame_queue.len(), 2);
    assert_eq!(
        state
            .decoded_frame_queue
            .front()
            .map(|frame| frame.surface.frame_seq),
        Some(2)
    );
}

#[test]
fn peek_decoded_frame_keeps_head_of_queue_intact() {
    let decoder = SpyHardwareDecoder;
    let mut state = XbxVideoDecodeState::new_for_test(20, 30, Box::new(decoder));

    state.enqueue_decoded_frame_for_test(XbxRenderFrame {
        width: 2,
        height: 2,
        frame_seq: 7,
        rendered_at_ms: 7.0,
        rtp_timestamp: Some(7),
        is_keyframe: true,
        frame_recovery_disposition: Some("repairing".to_string()),
        frame_unrecoverable_reason: None,
        pixel_data: XbxEngineRenderPixelData::Rgba {
            bytes: Arc::<[u8]>::from([0u8; 16]),
        },
    });

    assert_eq!(
        state
            .peek_decoded_frame()
            .map(|frame| frame.surface.frame_seq),
        Some(7)
    );
    assert!(state.has_decoded_frame());
    assert_eq!(
        state
            .peek_decoded_frame()
            .map(|frame| frame.surface.frame_seq),
        Some(7)
    );
    assert_eq!(
        state
            .pop_decoded_frame(8.0)
            .map(|frame| frame.surface.frame_seq),
        Some(7)
    );
    assert!(!state.has_decoded_frame());
}

#[test]
fn peek_decoded_frame_reports_front_without_consuming() {
    let decoder = SpyHardwareDecoder;
    let mut state = XbxVideoDecodeState::new_for_test(20, 30, Box::new(decoder));

    state.enqueue_decoded_frame_for_test(XbxRenderFrame {
        width: 2,
        height: 2,
        frame_seq: 1,
        rendered_at_ms: 1.0,
        rtp_timestamp: Some(1),
        is_keyframe: true,
        frame_recovery_disposition: Some("repairing".to_string()),
        frame_unrecoverable_reason: None,
        pixel_data: XbxEngineRenderPixelData::Rgba {
            bytes: Arc::<[u8]>::from([0u8; 16]),
        },
    });

    assert!(state.has_decoded_frame());
    assert_eq!(
        state
            .peek_decoded_frame()
            .map(|frame| frame.surface.frame_seq),
        Some(1)
    );
    assert_eq!(
        state
            .peek_decoded_frame()
            .map(|frame| frame.surface.frame_seq),
        Some(1)
    );
    assert_eq!(
        state
            .pop_decoded_frame(2.0)
            .map(|frame| frame.surface.frame_seq),
        Some(1)
    );
    assert!(!state.has_decoded_frame());
}

#[test]
fn decoded_frame_queue_is_full_tracks_capacity_without_consuming() {
    let decoder = SpyHardwareDecoder;
    let mut state = XbxVideoDecodeState::new_for_test(20, 30, Box::new(decoder));

    assert!(!state.decoded_frame_queue_is_full());

    for seq in 1..=2 {
        state.enqueue_decoded_frame_for_test(XbxRenderFrame {
            width: 2,
            height: 2,
            frame_seq: seq,
            rendered_at_ms: seq as f64,
            rtp_timestamp: Some(seq as u32),
            is_keyframe: seq == 1,
            frame_recovery_disposition: Some("repairing".to_string()),
            frame_unrecoverable_reason: None,
            pixel_data: XbxEngineRenderPixelData::Rgba {
                bytes: Arc::<[u8]>::from([0u8; 16]),
            },
        });
    }

    assert!(state.has_decoded_frame());
    assert_eq!(
        state
            .peek_decoded_frame()
            .map(|frame| frame.surface.frame_seq),
        Some(1)
    );
    assert!(state.decoded_frame_queue_is_full());

    assert_eq!(
        state
            .pop_decoded_frame(3.0)
            .map(|frame| frame.surface.frame_seq),
        Some(1)
    );
    assert!(!state.decoded_frame_queue_is_full());
    assert!(state.has_decoded_frame());
}

#[test]
fn requeue_decoded_frame_front_restores_head_order_after_backpressure() {
    let decoder = SpyHardwareDecoder;
    let mut state = XbxVideoDecodeState::new_for_test(20, 30, Box::new(decoder));

    state.enqueue_decoded_frame_for_test(XbxRenderFrame {
        width: 2,
        height: 2,
        frame_seq: 11,
        rendered_at_ms: 11.0,
        rtp_timestamp: Some(11),
        is_keyframe: false,
        frame_recovery_disposition: Some("repairing".to_string()),
        frame_unrecoverable_reason: None,
        pixel_data: XbxEngineRenderPixelData::Rgba {
            bytes: Arc::<[u8]>::from([0u8; 16]),
        },
    });

    let frame = state.pop_decoded_frame(12.0).expect("frame should exist");
    state.requeue_decoded_frame_front(frame);

    assert_eq!(
        state
            .peek_decoded_frame()
            .map(|frame| frame.surface.frame_seq),
        Some(11)
    );
    assert_eq!(
        state
            .pop_decoded_frame(13.0)
            .map(|frame| frame.surface.frame_seq),
        Some(11)
    );
    assert!(!state.has_decoded_frame());
}

#[test]
fn enqueue_decoded_frame_returns_dropped_oldest_frame() {
    let decoder = SpyHardwareDecoder;
    let mut state = XbxVideoDecodeState::new_for_test(20, 30, Box::new(decoder));

    state.enqueue_decoded_frame_for_test(XbxRenderFrame {
        width: 2,
        height: 2,
        frame_seq: 1,
        rendered_at_ms: 1.0,
        rtp_timestamp: Some(1),
        is_keyframe: true,
        frame_recovery_disposition: Some("repairing".to_string()),
        frame_unrecoverable_reason: None,
        pixel_data: XbxEngineRenderPixelData::Rgba {
            bytes: Arc::<[u8]>::from([0u8; 16]),
        },
    });
    state.enqueue_decoded_frame_for_test(XbxRenderFrame {
        width: 2,
        height: 2,
        frame_seq: 2,
        rendered_at_ms: 2.0,
        rtp_timestamp: Some(2),
        is_keyframe: false,
        frame_recovery_disposition: Some("repairing".to_string()),
        frame_unrecoverable_reason: None,
        pixel_data: XbxEngineRenderPixelData::Rgba {
            bytes: Arc::<[u8]>::from([1u8; 16]),
        },
    });

    let dropped = state.enqueue_decoded_frame(DecodedFrame {
        pts: Instant::now(),
        rtp_timestamp: 3,
        is_keyframe: false,
        budget: crate::media::video::ingress::budget::FrameBudgetContext::default(),
        frame_recovery_disposition: FrameRecoveryDisposition::Repairing,
        frame_unrecoverable_reason: None,
        surface: XbxRenderFrame {
            width: 2,
            height: 2,
            frame_seq: 3,
            rendered_at_ms: 3.0,
            rtp_timestamp: Some(3),
            is_keyframe: false,
            frame_recovery_disposition: Some("repairing".to_string()),
            frame_unrecoverable_reason: None,
            pixel_data: XbxEngineRenderPixelData::Rgba {
                bytes: Arc::<[u8]>::from([2u8; 16]),
            },
        },
    });

    assert_eq!(dropped.map(|frame| frame.surface.frame_seq), Some(1));
    assert_eq!(state.decoded_frame_drop_count(), 1);
    let decision = state
        .latest_decode_candidate_decision()
        .expect("candidate decision");
    assert_eq!(decision.state, XbxDecodeCandidateState::Backpressure);
    assert_eq!(decision.action, "drop");
    assert_eq!(decision.detail, "outputQueueOverflow");
    assert_eq!(decision.frame_seq, Some(1));
}

#[test]
fn decode_candidate_state_recovers_to_nominal_after_pressure_is_relieved() {
    let decoder = SpyHardwareDecoder;
    let mut state = XbxVideoDecodeState::new_for_test(20, 30, Box::new(decoder));

    for seq in 1..=3 {
        state.enqueue_decoded_frame_for_test(XbxRenderFrame {
            width: 2,
            height: 2,
            frame_seq: seq,
            rendered_at_ms: seq as f64,
            rtp_timestamp: Some(seq as u32),
            is_keyframe: seq == 1,
            frame_recovery_disposition: Some("repairing".to_string()),
            frame_unrecoverable_reason: None,
            pixel_data: XbxEngineRenderPixelData::Rgba {
                bytes: Arc::<[u8]>::from([0u8; 16]),
            },
        });
    }
    let pressured = state
        .latest_decode_candidate_decision()
        .expect("backpressure decision");
    assert_eq!(pressured.state, XbxDecodeCandidateState::Backpressure);

    let _ = state.pop_decoded_frame(4.0);
    state.enqueue_decoded_frame_for_test(XbxRenderFrame {
        width: 2,
        height: 2,
        frame_seq: 4,
        rendered_at_ms: 4.0,
        rtp_timestamp: Some(4),
        is_keyframe: false,
        frame_recovery_disposition: Some("repairing".to_string()),
        frame_unrecoverable_reason: None,
        pixel_data: XbxEngineRenderPixelData::Rgba {
            bytes: Arc::<[u8]>::from([1u8; 16]),
        },
    });
    let recovered = state
        .latest_decode_candidate_decision()
        .expect("recovered decision");
    assert_eq!(recovered.state, XbxDecodeCandidateState::Nominal);
    assert_eq!(recovered.action, "accept");
    assert_eq!(recovered.detail, "queueRecovered");
    assert_eq!(recovered.frame_seq, Some(4));
}

#[test]
fn workload_snapshot_switches_to_drain_output_when_queue_is_non_empty() {
    let decoder = SpyHardwareDecoder;
    let mut state = XbxVideoDecodeState::new_for_test(20, 30, Box::new(decoder));

    let initial = state.workload_snapshot();
    assert_eq!(initial.state, XbxDecodeWorkloadState::AwaitingInput);
    assert_eq!(initial.pending_output_queue_depth, 0);
    assert!(!initial.should_drain_output_first());

    state.enqueue_decoded_frame_for_test(XbxRenderFrame {
        width: 2,
        height: 2,
        frame_seq: 1,
        rendered_at_ms: 1.0,
        rtp_timestamp: Some(1),
        is_keyframe: true,
        frame_recovery_disposition: Some("repairing".to_string()),
        frame_unrecoverable_reason: None,
        pixel_data: XbxEngineRenderPixelData::Rgba {
            bytes: Arc::<[u8]>::from([0u8; 16]),
        },
    });

    let queued = state.workload_snapshot();
    assert_eq!(queued.state, XbxDecodeWorkloadState::DrainOutput);
    assert_eq!(queued.pending_output_queue_depth, 1);
    assert!(queued.should_drain_output_first());

    let _ = state.pop_decoded_frame(2.0);
    let drained = state.workload_snapshot();
    assert_eq!(drained.state, XbxDecodeWorkloadState::AwaitingInput);
    assert_eq!(drained.pending_output_queue_depth, 0);
    assert!(!drained.should_drain_output_first());
}

struct ScriptedHardwareDecoder {
    backend_name: &'static str,
    decode_calls: Arc<AtomicUsize>,
    scripted_results: VecDeque<Result<Option<XbxRenderFrame>, crate::XbxEngineRuntimeError>>,
}

impl XbxVideoDecoderBackend for ScriptedHardwareDecoder {
    fn backend_name(&self) -> &'static str {
        self.backend_name
    }

    fn decode(
        &mut self,
        _encoded_frame: EncodedFrame,
        _now_ms: f64,
    ) -> Result<Option<XbxRenderFrame>, crate::XbxEngineRuntimeError> {
        self.decode_calls.fetch_add(1, Ordering::Relaxed);
        self.scripted_results.pop_front().unwrap_or(Ok(None))
    }
}

fn make_encoded_frame(is_keyframe: bool) -> EncodedFrame {
    let now = Instant::now();
    EncodedFrame {
        codec: crate::media::video::types::VideoCodec::H264,
        is_keyframe,
        config_changed: false,
        value: crate::media::video::types::FrameValue::new(is_keyframe, false, 1024),
        budget: crate::media::video::ingress::budget::FrameBudgetContext::steady_for_value(
            crate::media::video::types::FrameValue::new(is_keyframe, false, 1024),
        ),
        width: 2560,
        height: 1440,
        rtp_timestamp: if is_keyframe { 1 } else { 2 },
        frame_playout_deadline_at_ms: None,
        frame_recovery_disposition: crate::media::video::types::FrameRecoveryDisposition::Repairing,
        frame_unrecoverable_reason: None,
        target_playout_time: now + Duration::from_millis(16),
        h264: make_h264_inspection(is_keyframe),
        payload: Bytes::from_static(b"\x00\x00\x00\x01\x65"),
    }
}

fn make_h264_inspection(bootstrap_ready: bool) -> H264AccessUnitInspection {
    H264AccessUnitInspection {
        nals: Vec::new(),
        parameter_sets: None,
        width: Some(2560),
        height: Some(1440),
        is_idr: bootstrap_ready,
        has_inband_sps: bootstrap_ready,
        has_inband_pps: bootstrap_ready,
        slice_headers_valid: bootstrap_ready,
        parameter_sets_changed: false,
        config_changed: false,
        bootstrap_ready,
        bootstrap_reject_reason: if bootstrap_ready {
            None
        } else {
            Some(H264BootstrapRejectReason::MissingSps)
        },
        commit_state: H264AccessUnitInspector::test_commit_state(),
    }
}

#[test]
fn bad_data_failure_waits_for_next_keyframe_before_decoding_again() {
    let decode_calls = Arc::new(AtomicUsize::new(0));
    let replacement_decode_calls = Arc::new(AtomicUsize::new(0));
    let decoder = ScriptedHardwareDecoder {
        backend_name: "scripted",
        decode_calls: decode_calls.clone(),
        scripted_results: VecDeque::from([
            Err(crate::XbxEngineRuntimeError::new(
                "xbxEngineVideoToolboxOutputCallbackFailed:status=-12909",
            )),
            Ok(None),
        ]),
    };
    let replacement_decode_calls_for_factory = replacement_decode_calls.clone();
    let decoder_factory = Box::new(move || {
        (
            Box::new(ScriptedHardwareDecoder {
                backend_name: "replacement",
                decode_calls: replacement_decode_calls_for_factory.clone(),
                scripted_results: VecDeque::from([Ok(None)]),
            }) as Box<dyn XbxVideoDecoderBackend>,
            XbxVideoDecoderProbeSummary {
                selected_backend_name: "replacement".to_string(),
                selected_backend_kind: "software".to_string(),
                fallback_count: 1,
                fallback_summary: Some("scripted(hardware/initialization-failed)".to_string()),
            },
        )
    });
    let mut state =
        XbxVideoDecodeState::new_for_test_with_factory(20, 30, Box::new(decoder), decoder_factory);

    state.process_encoded_frame(make_encoded_frame(true), 1_000.0);
    assert_eq!(decode_calls.load(Ordering::Relaxed), 1);
    assert_eq!(state.decoder_reset_count(), 1);
    assert_eq!(state.decoder_backend_name(), "replacement");

    state.process_encoded_frame(make_encoded_frame(false), 1_016.0);
    assert_eq!(decode_calls.load(Ordering::Relaxed), 1);

    state.process_encoded_frame(make_encoded_frame(true), 1_032.0);
    assert_eq!(replacement_decode_calls.load(Ordering::Relaxed), 1);
}

#[test]
fn backend_failure_resets_decoder_via_probe_factory_and_updates_probe_snapshot() {
    let failing_decode_calls = Arc::new(AtomicUsize::new(0));
    let replacement_decode_calls = Arc::new(AtomicUsize::new(0));

    let decoder = ScriptedHardwareDecoder {
        backend_name: "ffmpeg-d3d11va",
        decode_calls: failing_decode_calls.clone(),
        scripted_results: VecDeque::from([Err(crate::XbxEngineRuntimeError::new(
            "xbxEngineCreateVideoFormatDescriptionFailed:status=-12909",
        ))]),
    };

    let mut reset_performed = false;
    let replacement_decode_calls_for_factory = replacement_decode_calls.clone();
    let decoder_factory = Box::new(move || {
        assert!(!reset_performed, "decoder factory should only run once");
        reset_performed = true;
        let decoder: Box<dyn XbxVideoDecoderBackend> = Box::new(ScriptedHardwareDecoder {
            backend_name: "ffmpeg-software",
            decode_calls: replacement_decode_calls_for_factory.clone(),
            scripted_results: VecDeque::from([Ok(Some(XbxRenderFrame {
                width: 2,
                height: 2,
                frame_seq: 0,
                rendered_at_ms: 0.0,
                rtp_timestamp: None,
                is_keyframe: false,
                frame_recovery_disposition: None,
                frame_unrecoverable_reason: None,
                pixel_data: XbxEngineRenderPixelData::Rgba {
                    bytes: Arc::<[u8]>::from([1u8; 16]),
                },
            }))]),
        });
        (
            decoder,
            XbxVideoDecoderProbeSummary {
                selected_backend_name: "ffmpeg-software".to_string(),
                selected_backend_kind: "software".to_string(),
                fallback_count: 1,
                fallback_summary: Some(
                    "ffmpeg-d3d11va(hardware/initialization-failed):status=-12909".to_string(),
                ),
            },
        )
    });
    let mut state =
        XbxVideoDecodeState::new_for_test_with_factory(20, 30, Box::new(decoder), decoder_factory);

    assert!(state
        .process_encoded_frame(make_encoded_frame(true), 2_000.0)
        .is_none());
    assert_eq!(failing_decode_calls.load(Ordering::Relaxed), 1);
    assert_eq!(
        state.recovery_state(),
        XbxVideoRecoveryState::WaitingKeyframe
    );
    assert_eq!(state.decoder_backend_name(), "ffmpeg-software");

    let probe = state
        .latest_decoder_probe()
        .expect("probe observation should be present");
    assert_eq!(probe.observation_id, 1);
    assert_eq!(probe.selected_backend_name, "ffmpeg-software");
    assert_eq!(probe.selected_backend_kind, "software");
    assert_eq!(probe.fallback_count, 1);
    assert!(probe
        .fallback_summary
        .as_deref()
        .is_some_and(|summary| summary.contains("ffmpeg-d3d11va")));

    assert!(state
        .process_encoded_frame(make_encoded_frame(true), 2_016.0)
        .is_none());
    assert_eq!(replacement_decode_calls.load(Ordering::Relaxed), 1);
    assert_eq!(state.recovery_state(), XbxVideoRecoveryState::Recovering);
}

#[test]
fn assembled_bootstrap_frame_decodes_and_renders_end_to_end() {
    let decoder = ScriptedHardwareDecoder {
        backend_name: "scripted",
        decode_calls: Arc::new(AtomicUsize::new(0)),
        scripted_results: VecDeque::from([Ok(Some(XbxRenderFrame {
            width: 64,
            height: 64,
            frame_seq: 0,
            rendered_at_ms: 0.0,
            rtp_timestamp: None,
            is_keyframe: false,
            frame_recovery_disposition: None,
            frame_unrecoverable_reason: None,
            pixel_data: XbxEngineRenderPixelData::Rgba {
                bytes: Arc::<[u8]>::from(vec![7u8; 64 * 64 * 4]),
            },
        }))]),
    };
    let mut state = XbxVideoDecodeState::new_for_test(20, 30, Box::new(decoder));
    let encoded = make_bootstrap_assembled_frame(9000)
        .into_encoded_frame(Instant::now() + Duration::from_millis(16));

    assert!(state.process_encoded_frame(encoded, 1_000.0).is_none());
    let decoded = state
        .pop_decoded_frame(1_016.0)
        .expect("decoded frame should be queued");
    assert_eq!(decoded.surface.frame_seq, 1);
    assert_eq!(decoded.surface.rtp_timestamp, Some(9000));
    assert!(decoded.surface.is_keyframe);
    assert_eq!(
        decoded.surface.frame_recovery_disposition.as_deref(),
        Some("repairing")
    );

    let mut render_state = XbxRenderState::default();
    let (_stats, outcome) = render_state
        .present_frame(decoded.surface)
        .expect("render should accept decoded frame");
    assert!(!outcome.overwritten_previous_latest);
    assert_eq!(
        render_state
            .peek_latest_frame()
            .map(|frame| frame.frame_seq),
        Some(1)
    );
}

#[test]
fn backend_failure_then_clean_bootstrap_frames_recover_pipeline_to_nominal() {
    let replacement_decode_calls = Arc::new(AtomicUsize::new(0));
    let decoder = ScriptedHardwareDecoder {
        backend_name: "scripted",
        decode_calls: Arc::new(AtomicUsize::new(0)),
        scripted_results: VecDeque::from([Err(crate::XbxEngineRuntimeError::new(
            "xbxEngineCreateVideoFormatDescriptionFailed:status=-12909",
        ))]),
    };
    let replacement_decode_calls_for_factory = replacement_decode_calls.clone();
    let decoder_factory = Box::new(move || {
        let decoder: Box<dyn XbxVideoDecoderBackend> = Box::new(ScriptedHardwareDecoder {
            backend_name: "replacement",
            decode_calls: replacement_decode_calls_for_factory.clone(),
            scripted_results: VecDeque::from([
                Ok(Some(XbxRenderFrame {
                    width: 64,
                    height: 64,
                    frame_seq: 0,
                    rendered_at_ms: 0.0,
                    rtp_timestamp: None,
                    is_keyframe: false,
                    frame_recovery_disposition: None,
                    frame_unrecoverable_reason: None,
                    pixel_data: XbxEngineRenderPixelData::Rgba {
                        bytes: Arc::<[u8]>::from(vec![5u8; 64 * 64 * 4]),
                    },
                })),
                Ok(Some(XbxRenderFrame {
                    width: 64,
                    height: 64,
                    frame_seq: 0,
                    rendered_at_ms: 0.0,
                    rtp_timestamp: None,
                    is_keyframe: false,
                    frame_recovery_disposition: None,
                    frame_unrecoverable_reason: None,
                    pixel_data: XbxEngineRenderPixelData::Rgba {
                        bytes: Arc::<[u8]>::from(vec![6u8; 64 * 64 * 4]),
                    },
                })),
            ]),
        });
        (
            decoder,
            XbxVideoDecoderProbeSummary {
                selected_backend_name: "replacement".to_string(),
                selected_backend_kind: "software".to_string(),
                fallback_count: 1,
                fallback_summary: Some("scripted(hardware/initialization-failed)".to_string()),
            },
        )
    });
    let mut state =
        XbxVideoDecodeState::new_for_test_with_factory(20, 30, Box::new(decoder), decoder_factory);

    let first = make_bootstrap_assembled_frame(9000)
        .into_encoded_frame(Instant::now() + Duration::from_millis(16));
    let second = make_bootstrap_assembled_frame(9016)
        .into_encoded_frame(Instant::now() + Duration::from_millis(32));
    let third = make_bootstrap_assembled_frame(9032)
        .into_encoded_frame(Instant::now() + Duration::from_millis(48));

    assert!(state.process_encoded_frame(first, 1_000.0).is_none());
    assert_eq!(
        state.recovery_state(),
        XbxVideoRecoveryState::WaitingKeyframe
    );

    assert!(state.process_encoded_frame(second, 1_016.0).is_none());
    assert_eq!(state.recovery_state(), XbxVideoRecoveryState::Recovering);

    assert!(state.process_encoded_frame(third, 1_032.0).is_none());
    assert_eq!(state.recovery_state(), XbxVideoRecoveryState::Nominal);
    assert_eq!(replacement_decode_calls.load(Ordering::Relaxed), 2);

    let first_recovered = state
        .pop_decoded_frame(1_040.0)
        .expect("first recovered frame should exist");
    let second_recovered = state
        .pop_decoded_frame(1_048.0)
        .expect("second recovered frame should exist");
    assert_eq!(first_recovered.surface.frame_seq, 1);
    assert_eq!(second_recovered.surface.frame_seq, 2);

    let mut render_state = XbxRenderState::default();
    render_state
        .present_frame(first_recovered.surface)
        .expect("first recovered frame should render");
    render_state
        .present_frame(second_recovered.surface)
        .expect("second recovered frame should render");
    assert_eq!(
        render_state
            .peek_latest_frame()
            .map(|frame| frame.frame_seq),
        Some(2)
    );
}

#[tokio::test]
async fn rtp_to_decode_to_pacer_to_renderer_pipeline_reaches_latest_frame_and_overwrite_signal() {
    let (tx, _transport_observation_rx, mut source) = make_video_source_for_test();
    let runtime_stats = Arc::new(std::sync::Mutex::new(XbxEngineMediaRuntimeStats::default()));

    send_bootstrap_access_unit(&tx, 100, 9000).await;
    send_bootstrap_access_unit(&tx, 103, 9016).await;
    send_bootstrap_access_unit(&tx, 106, 9032).await;
    // 再送一个后续 AU，确保前一个 sample 被 SampleBuilder 刷出。
    send_bootstrap_access_unit(&tx, 109, 9048).await;
    drop(tx);

    let decoder = ScriptedHardwareDecoder {
        backend_name: "scripted",
        decode_calls: Arc::new(AtomicUsize::new(0)),
        scripted_results: VecDeque::from([
            Ok(Some(XbxRenderFrame {
                width: 64,
                height: 64,
                frame_seq: 0,
                rendered_at_ms: 0.0,
                rtp_timestamp: None,
                is_keyframe: false,
                frame_recovery_disposition: None,
                frame_unrecoverable_reason: None,
                pixel_data: XbxEngineRenderPixelData::Rgba {
                    bytes: Arc::<[u8]>::from(vec![1u8; 64 * 64 * 4]),
                },
            })),
            Ok(Some(XbxRenderFrame {
                width: 64,
                height: 64,
                frame_seq: 0,
                rendered_at_ms: 0.0,
                rtp_timestamp: None,
                is_keyframe: false,
                frame_recovery_disposition: None,
                frame_unrecoverable_reason: None,
                pixel_data: XbxEngineRenderPixelData::Rgba {
                    bytes: Arc::<[u8]>::from(vec![2u8; 64 * 64 * 4]),
                },
            })),
            Ok(Some(XbxRenderFrame {
                width: 64,
                height: 64,
                frame_seq: 0,
                rendered_at_ms: 0.0,
                rtp_timestamp: None,
                is_keyframe: false,
                frame_recovery_disposition: None,
                frame_unrecoverable_reason: None,
                pixel_data: XbxEngineRenderPixelData::Rgba {
                    bytes: Arc::<[u8]>::from(vec![3u8; 64 * 64 * 4]),
                },
            })),
        ]),
    };
    let mut decode_state = XbxVideoDecodeState::new_for_test(20, 30, Box::new(decoder));
    let render_state = Arc::new(std::sync::Mutex::new(XbxRenderState::default()));
    let renderer = Arc::new(
        crate::media::video::render::actor::RendererActorHandle::new(
            render_state.clone(),
            runtime_stats.clone(),
        ),
    );
    let pacer = crate::media::video::pacer::actor::PacerActorHandle::new(
        renderer.clone(),
        runtime_stats.clone(),
        16,
    );

    for expected_timestamp in [9000u32, 9016u32, 9032u32] {
        let assembled = tokio::time::timeout(Duration::from_millis(250), source.recv_frame())
            .await
            .expect("source should assemble frame in time")
            .expect("assembled frame should exist");
        assert_eq!(assembled.rtp_timestamp, expected_timestamp);
        let encoded = assembled.into_encoded_frame(Instant::now());
        assert!(decode_state
            .process_encoded_frame(encoded, expected_timestamp as f64)
            .is_none());
        let decoded = decode_state
            .pop_decoded_frame(expected_timestamp as f64 + 1.0)
            .expect("decoded frame should be available");

        let submit_deadline = Instant::now() + Duration::from_millis(150);
        let mut submitted = false;
        while Instant::now() < submit_deadline {
            match pacer.submit(decoded.clone()) {
                Ok(_) => {
                    submitted = true;
                    break;
                }
                Err(std::sync::mpsc::TrySendError::Full(_)) => {
                    std::thread::sleep(Duration::from_millis(2));
                }
                Err(err) => panic!("unexpected pacer submit failure: {err:?}"),
            }
        }
        assert!(submitted, "decoded frame should eventually reach pacer");
    }

    let render_deadline = Instant::now() + Duration::from_millis(300);
    let mut latest_seq = None;
    while Instant::now() < render_deadline {
        let frame = render_state
            .lock()
            .expect("render state lock")
            .take_latest_frame();
        if let Some(frame) = frame {
            latest_seq = Some(frame.frame_seq);
            if frame.frame_seq >= 3 {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(4));
    }

    pacer.stop();
    renderer.stop();

    assert_eq!(latest_seq, Some(3));
    let stats = runtime_stats.lock().expect("runtime stats lock");
    assert!(stats.video_renderer_submit_count_total >= 2);
    let decision = stats
        .latest_render_candidate_decision
        .clone()
        .expect("render candidate decision should exist");
    assert_eq!(decision.state, "latest-overwrite");
    assert_eq!(decision.detail, "latestSlotOverwrite");
}
