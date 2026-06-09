use std::collections::VecDeque;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

use super::{
    XbxDecodeCandidateState, XbxDecodeOutputPathVerdict, XbxDecodeWorkloadState,
    XbxVideoDecodeState, XbxVideoRecoveryEvent, XbxVideoRecoveryState,
    WAITING_KEYFRAME_CONTINUATION_MAX_FRAMES,
};
use crate::media::video::decode::backend::{
    XbxVideoDecoderBackend, XbxVideoDecoderBackendDecodeOutcome, XbxVideoDecoderProbeSummary,
};
use crate::media::video::h264::inspection::{
    H264AccessUnitInspection, H264AccessUnitInspector, H264BootstrapRejectReason, H264NalUnit,
};
use crate::{
    api::backend::XbxEngineMediaRuntimeStats,
    media::video::ingress::budget::FrameBudgetWindowSource,
    media::video::render::renderer::XbxRenderFrame,
    media::video::render::renderer::XbxRenderState,
    media::video::test_fixtures::{make_bootstrap_assembled_frame, send_bootstrap_access_unit},
    media::video::types::{DecodedFrame, EncodedFrame, FrameRecoveryDisposition},
    transport::rtc::stream::adapter_types::FrameSource,
    XbxEngineRenderPixelData,
};
use bytes::Bytes;
use h264_reader::nal::UnitType;

struct SpyHardwareDecoder;

impl XbxVideoDecoderBackend for SpyHardwareDecoder {
    fn backend_name(&self) -> &'static str {
        "spy"
    }

    fn decode(
        &mut self,
        _encoded_frame: EncodedFrame,
        _now_ms: f64,
    ) -> Result<XbxVideoDecoderBackendDecodeOutcome, crate::XbxEngineRuntimeError> {
        Ok(XbxVideoDecoderBackendDecodeOutcome {
            frames: Vec::new(),
            send_packet_status: None,
            receive_frame_status: None,
        })
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
        recovery_epoch_tag: None,
        recovery_owner_rtp_timestamp: None,
        is_keyframe: true,
        frame_recovery_disposition: Some("repairing".to_string()),
        frame_unrecoverable_reason: None,
        presentation_value_role: None,
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
        recovery_epoch_tag: None,
        recovery_owner_rtp_timestamp: None,
        is_keyframe: false,
        frame_recovery_disposition: Some("repairing".to_string()),
        frame_unrecoverable_reason: None,
        presentation_value_role: None,
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

    let _ = state.process_encoded_frame(make_encoded_frame(false), 1_032.0);
    assert_eq!(state.recovery_state(), XbxVideoRecoveryState::Nominal);
    // 恢复锚点在首次提交前继续受保护，steady continuation 不提前覆盖。
    assert_eq!(state.decoded_frame_queue_len(), 1);
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
fn waiting_keyframe_non_bootstrap_frame_records_bootstrap_gate_observation() {
    let decoder = SpyHardwareDecoder;
    let decoder_factory = Box::new(|| {
        (
            Box::new(ScriptedHardwareDecoder {
                backend_name: "scripted-recreated",
                decode_calls: Arc::new(AtomicUsize::new(0)),
                scripted_results: VecDeque::new(),
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
    assert!(state
        .process_encoded_frame(make_encoded_frame(false), 1_000.0)
        .is_none());

    let observation = state
        .latest_bootstrap_gate_observation()
        .expect("bootstrap gate observation should exist");
    assert_eq!(
        observation.recovery_state,
        XbxVideoRecoveryState::WaitingKeyframe
    );
    assert_eq!(observation.frame_rtp_timestamp, 2);
    assert!(!observation.is_idr);
    assert!(!observation.has_inband_sps);
    assert!(!observation.has_inband_pps);
    assert!(!observation.bootstrap_ready);
    assert_eq!(
        observation.bootstrap_reject_reason.as_deref(),
        Some(H264BootstrapRejectReason::MissingSps.as_str())
    );

    let output_observation = state
        .latest_decode_output_path_observation()
        .expect("decode output path observation should exist");
    assert_eq!(
        output_observation.verdict,
        XbxDecodeOutputPathVerdict::BootstrapGateRejected
    );
    assert_eq!(output_observation.detail, "bootstrapGateRejected");
    assert_eq!(
        output_observation.bootstrap_reject_reason.as_deref(),
        Some(H264BootstrapRejectReason::MissingSps.as_str())
    );
}

#[test]
fn backend_no_output_records_decode_output_path_observation() {
    let decoder = ScriptedHardwareDecoder {
        backend_name: "scripted",
        decode_calls: Arc::new(AtomicUsize::new(0)),
        scripted_results: VecDeque::from([Ok(None)]),
    };
    let mut state = XbxVideoDecodeState::new_for_test(20, 30, Box::new(decoder));

    assert!(state
        .process_encoded_frame(make_encoded_frame(true), 1_000.0)
        .is_none());

    let observation = state
        .latest_decode_output_path_observation()
        .expect("decode output path observation should exist");
    assert_eq!(
        observation.verdict,
        XbxDecodeOutputPathVerdict::BackendNoOutput
    );
    assert_eq!(observation.detail, "backendNoOutput");
    assert_eq!(observation.frame_rtp_timestamp, 1);
    assert!(observation.is_keyframe);
    assert_eq!(observation.status, None);
}

#[test]
fn repeated_hardware_backend_no_output_falls_back_to_software_decoder() {
    let software_decode_calls = Arc::new(AtomicUsize::new(0));
    let decoder = ScriptedHardwareDecoder {
        backend_name: "ffmpeg-videotoolbox",
        decode_calls: Arc::new(AtomicUsize::new(0)),
        scripted_results: VecDeque::from([Ok(None), Ok(None)]),
    };
    let software_decode_calls_for_factory = software_decode_calls.clone();
    let software_decoder_factory = Box::new(move || {
        (
            Box::new(ScriptedHardwareDecoder {
                backend_name: "ffmpeg-software",
                decode_calls: software_decode_calls_for_factory.clone(),
                scripted_results: VecDeque::from([Ok(Some(XbxRenderFrame {
                    width: 2,
                    height: 2,
                    frame_seq: 0,
                    rendered_at_ms: 0.0,
                    rtp_timestamp: None,
                    recovery_epoch_tag: None,
                    recovery_owner_rtp_timestamp: None,
                    is_keyframe: false,
                    frame_recovery_disposition: None,
                    frame_unrecoverable_reason: None,
                    presentation_value_role: None,
                    pixel_data: XbxEngineRenderPixelData::Rgba {
                        bytes: Arc::<[u8]>::from([3u8; 16]),
                    },
                }))]),
            }) as Box<dyn XbxVideoDecoderBackend>,
            XbxVideoDecoderProbeSummary {
                selected_backend_name: "ffmpeg-software".to_string(),
                selected_backend_kind: "software".to_string(),
                fallback_count: 0,
                fallback_summary: None,
            },
        )
    });
    let mut state = XbxVideoDecodeState::new_for_test_with_factories(
        20,
        30,
        Box::new(decoder),
        Box::new(|| {
            panic!(
                "decoder reset factory should not be used in backend no-output soft fallback test"
            );
        }),
        software_decoder_factory,
    );

    for observed_at_ms in [1_000.0, 1_016.0] {
        assert!(state
            .process_encoded_frame(make_encoded_frame(true), observed_at_ms)
            .is_none());
    }

    assert_eq!(state.decoder_backend_name(), "ffmpeg-software");
    assert_eq!(state.decoder_reset_count(), 1);
    assert_eq!(
        state.recovery_state(),
        XbxVideoRecoveryState::WaitingKeyframe
    );
    let transition = state
        .latest_recovery_transition()
        .expect("recovery transition should exist after software fallback");
    assert_eq!(transition.detail, "backendNoOutputSoftFallback");
    let probe = state
        .latest_decoder_probe()
        .expect("software fallback should publish decoder probe");
    assert_eq!(probe.selected_backend_name, "ffmpeg-software");
    assert_eq!(probe.selected_backend_kind, "software");
    assert!(probe
        .fallback_summary
        .as_deref()
        .is_some_and(|summary| summary.contains("backend-no-output")));
}

#[test]
fn nominal_continuation_no_output_exports_receive_keyframe_hint() {
    let decoder = ScriptedHardwareDecoder {
        backend_name: "ffmpeg-videotoolbox",
        decode_calls: Arc::new(AtomicUsize::new(0)),
        scripted_results: VecDeque::from([Ok(None), Ok(None)]),
    };
    let reset_calls = Arc::new(AtomicUsize::new(0));
    let reset_calls_for_factory = reset_calls.clone();
    let decoder_factory = Box::new(move || {
        reset_calls_for_factory.fetch_add(1, Ordering::Relaxed);
        (
            Box::new(ScriptedHardwareDecoder {
                backend_name: "ffmpeg-videotoolbox",
                decode_calls: Arc::new(AtomicUsize::new(0)),
                scripted_results: VecDeque::new(),
            }) as Box<dyn XbxVideoDecoderBackend>,
            XbxVideoDecoderProbeSummary {
                selected_backend_name: "ffmpeg-videotoolbox".to_string(),
                selected_backend_kind: "hardware".to_string(),
                fallback_count: 0,
                fallback_summary: None,
            },
        )
    });
    let mut state =
        XbxVideoDecodeState::new_for_test_with_factory(20, 30, Box::new(decoder), decoder_factory);

    let _ = state.process_encoded_frame(make_encoded_frame(true), 984.0);
    assert_eq!(state.decoder_reset_count(), 0);

    assert!(state
        .process_encoded_frame(make_non_idr_continuation_frame(10), 1_000.0)
        .is_none());
    assert!(state.take_pending_receive_keyframe_hint_at_ms().is_some());
    assert!(state
        .process_encoded_frame(make_non_idr_continuation_frame(11), 1_016.0)
        .is_none());
    assert!(state.take_pending_receive_keyframe_hint_at_ms().is_some());
    assert_eq!(state.decoder_reset_count(), 0);
    assert_eq!(reset_calls.load(Ordering::Relaxed), 0);
}

#[cfg(target_os = "windows")]
#[test]
fn d3d11va_backend_no_output_rebuilds_once_before_software_fallback() {
    let reset_calls = Arc::new(AtomicUsize::new(0));
    let replacement_decode_calls = Arc::new(AtomicUsize::new(0));
    let software_decode_calls = Arc::new(AtomicUsize::new(0));
    let decoder = ScriptedHardwareDecoder {
        backend_name: "ffmpeg-d3d11va",
        decode_calls: Arc::new(AtomicUsize::new(0)),
        scripted_results: VecDeque::from([Ok(None), Ok(None)]),
    };
    let reset_calls_for_factory = reset_calls.clone();
    let replacement_decode_calls_for_factory = replacement_decode_calls.clone();
    let decoder_factory = Box::new(move || {
        reset_calls_for_factory.fetch_add(1, Ordering::Relaxed);
        (
            Box::new(ScriptedHardwareDecoder {
                backend_name: "ffmpeg-d3d11va",
                decode_calls: replacement_decode_calls_for_factory.clone(),
                scripted_results: VecDeque::from([Ok(None), Ok(None), Ok(None), Ok(None)]),
            }) as Box<dyn XbxVideoDecoderBackend>,
            XbxVideoDecoderProbeSummary {
                selected_backend_name: "ffmpeg-d3d11va".to_string(),
                selected_backend_kind: "hardware".to_string(),
                fallback_count: 0,
                fallback_summary: None,
            },
        )
    });
    let software_decode_calls_for_factory = software_decode_calls.clone();
    let software_decoder_factory = Box::new(move || {
        (
            Box::new(ScriptedHardwareDecoder {
                backend_name: "ffmpeg-software",
                decode_calls: software_decode_calls_for_factory.clone(),
                scripted_results: VecDeque::new(),
            }) as Box<dyn XbxVideoDecoderBackend>,
            XbxVideoDecoderProbeSummary {
                selected_backend_name: "ffmpeg-software".to_string(),
                selected_backend_kind: "software".to_string(),
                fallback_count: 0,
                fallback_summary: None,
            },
        )
    });
    let mut state = XbxVideoDecodeState::new_for_test_with_factories(
        20,
        30,
        Box::new(decoder),
        decoder_factory,
        software_decoder_factory,
    );

    for observed_at_ms in [10_000.0, 10_016.0] {
        assert!(state
            .process_encoded_frame(make_encoded_frame(true), observed_at_ms)
            .is_none());
    }
    assert_eq!(reset_calls.load(Ordering::Relaxed), 1);
    assert_eq!(state.decoder_backend_name(), "ffmpeg-d3d11va");
    assert_eq!(state.decoder_reset_count(), 1);
    assert_eq!(
        state
            .latest_recovery_transition()
            .expect("rebuild transition should exist")
            .detail,
        "d3d11vaBackendNoOutputRebuild"
    );

    for observed_at_ms in [10_032.0, 10_048.0, 10_064.0, 10_080.0] {
        assert!(state
            .process_encoded_frame(make_encoded_frame(true), observed_at_ms)
            .is_none());
    }
    assert_eq!(state.decoder_backend_name(), "ffmpeg-software");
    assert_eq!(state.decoder_reset_count(), 2);
    assert_eq!(software_decode_calls.load(Ordering::Relaxed), 0);
    assert_eq!(replacement_decode_calls.load(Ordering::Relaxed), 4);
}

#[cfg(not(target_os = "windows"))]
#[test]
fn d3d11va_backend_no_output_rebuild_path_is_disabled_off_windows() {
    let reset_calls = Arc::new(AtomicUsize::new(0));
    let software_decode_calls = Arc::new(AtomicUsize::new(0));
    let decoder = ScriptedHardwareDecoder {
        backend_name: "ffmpeg-d3d11va",
        decode_calls: Arc::new(AtomicUsize::new(0)),
        scripted_results: VecDeque::from([Ok(None), Ok(None)]),
    };
    let reset_calls_for_factory = reset_calls.clone();
    let decoder_factory = Box::new(move || {
        reset_calls_for_factory.fetch_add(1, Ordering::Relaxed);
        (
            Box::new(ScriptedHardwareDecoder {
                backend_name: "ffmpeg-d3d11va",
                decode_calls: Arc::new(AtomicUsize::new(0)),
                scripted_results: VecDeque::new(),
            }) as Box<dyn XbxVideoDecoderBackend>,
            XbxVideoDecoderProbeSummary {
                selected_backend_name: "ffmpeg-d3d11va".to_string(),
                selected_backend_kind: "hardware".to_string(),
                fallback_count: 0,
                fallback_summary: None,
            },
        )
    });
    let software_decode_calls_for_factory = software_decode_calls.clone();
    let software_decoder_factory = Box::new(move || {
        (
            Box::new(ScriptedHardwareDecoder {
                backend_name: "ffmpeg-software",
                decode_calls: software_decode_calls_for_factory.clone(),
                scripted_results: VecDeque::new(),
            }) as Box<dyn XbxVideoDecoderBackend>,
            XbxVideoDecoderProbeSummary {
                selected_backend_name: "ffmpeg-software".to_string(),
                selected_backend_kind: "software".to_string(),
                fallback_count: 0,
                fallback_summary: None,
            },
        )
    });
    let mut state = XbxVideoDecodeState::new_for_test_with_factories(
        20,
        30,
        Box::new(decoder),
        decoder_factory,
        software_decoder_factory,
    );

    for observed_at_ms in [10_000.0, 10_016.0] {
        assert!(state
            .process_encoded_frame(make_encoded_frame(true), observed_at_ms)
            .is_none());
    }

    assert_eq!(reset_calls.load(Ordering::Relaxed), 0);
    assert_eq!(state.decoder_backend_name(), "ffmpeg-software");
    assert_eq!(state.decoder_reset_count(), 1);
    assert_eq!(software_decode_calls.load(Ordering::Relaxed), 0);
    assert_eq!(
        state
            .latest_recovery_transition()
            .expect("software fallback transition should exist")
            .detail,
        "backendNoOutputSoftFallback"
    );
}

#[test]
fn waiting_keyframe_bootstrap_no_output_allows_safe_continuation_decode() {
    let decode_calls = Arc::new(AtomicUsize::new(0));
    let decoder = SpyHardwareDecoder;
    let decode_calls_for_factory = decode_calls.clone();
    let mut scripted_results_for_factory = Some(VecDeque::from([Ok(None), Ok(None)]));
    let decoder_factory = Box::new(move || {
        (
            Box::new(ScriptedHardwareDecoder {
                backend_name: "scripted-replacement",
                decode_calls: decode_calls_for_factory.clone(),
                scripted_results: scripted_results_for_factory.take().unwrap_or_default(),
            }) as Box<dyn XbxVideoDecoderBackend>,
            XbxVideoDecoderProbeSummary {
                selected_backend_name: "scripted-replacement".to_string(),
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

    let bootstrap = make_bootstrap_assembled_frame(101)
        .into_encoded_frame(Instant::now() + Duration::from_millis(16));
    let continuation_commit_state = bootstrap.h264.commit_state.clone();
    bootstrap.h264.commit();
    assert!(state.process_encoded_frame(bootstrap, 1_000.0).is_none());
    assert_eq!(decode_calls.load(Ordering::Relaxed), 1);
    assert_eq!(
        state
            .latest_decode_output_path_observation()
            .expect("observation should exist")
            .detail,
        "backendNoOutputAfterBootstrapKeyframe"
    );

    let mut continuation = make_encoded_frame(false);
    continuation.rtp_timestamp = 102;
    continuation.h264 = H264AccessUnitInspection {
        nals: vec![H264NalUnit {
            range: 0..1,
            unit_type: UnitType::SliceLayerWithoutPartitioningNonIdr,
        }],
        parameter_sets: None,
        width: Some(2560),
        height: Some(1440),
        is_idr: false,
        has_inband_sps: false,
        has_inband_pps: false,
        slice_headers_valid: true,
        parameter_sets_changed: false,
        config_changed: false,
        bootstrap_ready: false,
        bootstrap_reject_reason: Some(H264BootstrapRejectReason::NonIdrVcl),
        commit_state: continuation_commit_state,
    };

    assert!(state.process_encoded_frame(continuation, 1_008.0).is_none());
    assert_eq!(decode_calls.load(Ordering::Relaxed), 1);
    let observation = state
        .latest_decode_output_path_observation()
        .expect("decode output path observation should exist");
    assert_eq!(
        observation.verdict,
        XbxDecodeOutputPathVerdict::BootstrapGateRejected
    );
    assert_eq!(observation.detail, "bootstrapGateRejected");
}

#[test]
fn waiting_keyframe_continuation_rejects_again_after_frame_budget_exhausted() {
    let decode_calls = Arc::new(AtomicUsize::new(0));
    let decoder = SpyHardwareDecoder;
    let decode_calls_for_factory = decode_calls.clone();
    let mut scripted_results_for_factory = Some(VecDeque::from(vec![
        Ok(None);
        (WAITING_KEYFRAME_CONTINUATION_MAX_FRAMES + 2)
            as usize
    ]));
    let decoder_factory = Box::new(move || {
        (
            Box::new(ScriptedHardwareDecoder {
                backend_name: "scripted-replacement",
                decode_calls: decode_calls_for_factory.clone(),
                scripted_results: scripted_results_for_factory.take().unwrap_or_default(),
            }) as Box<dyn XbxVideoDecoderBackend>,
            XbxVideoDecoderProbeSummary {
                selected_backend_name: "scripted-replacement".to_string(),
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

    let bootstrap = make_bootstrap_assembled_frame(201)
        .into_encoded_frame(Instant::now() + Duration::from_millis(16));
    let continuation_commit_state = bootstrap.h264.commit_state.clone();
    bootstrap.h264.commit();
    assert!(state.process_encoded_frame(bootstrap, 2_000.0).is_none());
    assert_eq!(decode_calls.load(Ordering::Relaxed), 1);

    for index in 0..WAITING_KEYFRAME_CONTINUATION_MAX_FRAMES {
        let mut continuation = make_encoded_frame(false);
        continuation.rtp_timestamp = 202 + index;
        continuation.h264 = H264AccessUnitInspection {
            nals: vec![H264NalUnit {
                range: 0..1,
                unit_type: UnitType::SliceLayerWithoutPartitioningNonIdr,
            }],
            parameter_sets: None,
            width: Some(2560),
            height: Some(1440),
            is_idr: false,
            has_inband_sps: false,
            has_inband_pps: false,
            slice_headers_valid: true,
            parameter_sets_changed: false,
            config_changed: false,
            bootstrap_ready: false,
            bootstrap_reject_reason: Some(H264BootstrapRejectReason::NonIdrVcl),
            commit_state: continuation_commit_state.clone(),
        };
        assert!(state
            .process_encoded_frame(continuation, 2_005.0 + f64::from(index))
            .is_none());
    }
    assert_eq!(decode_calls.load(Ordering::Relaxed), 1);

    let mut exhausted = make_encoded_frame(false);
    exhausted.rtp_timestamp = 299;
    exhausted.h264 = H264AccessUnitInspection {
        nals: vec![H264NalUnit {
            range: 0..1,
            unit_type: UnitType::SliceLayerWithoutPartitioningNonIdr,
        }],
        parameter_sets: None,
        width: Some(2560),
        height: Some(1440),
        is_idr: false,
        has_inband_sps: false,
        has_inband_pps: false,
        slice_headers_valid: true,
        parameter_sets_changed: false,
        config_changed: false,
        bootstrap_ready: false,
        bootstrap_reject_reason: Some(H264BootstrapRejectReason::NonIdrVcl),
        commit_state: continuation_commit_state,
    };
    assert!(state.process_encoded_frame(exhausted, 2_020.0).is_none());
    assert_eq!(decode_calls.load(Ordering::Relaxed), 1);
    let observation = state
        .latest_decode_output_path_observation()
        .expect("decode output path observation should exist");
    assert_eq!(
        observation.verdict,
        XbxDecodeOutputPathVerdict::BootstrapGateRejected
    );
    assert_eq!(observation.detail, "bootstrapGateRejected");
}

#[test]
fn current_clean_anchor_supply_break_allows_waiting_keyframe_continuation_without_displayed_idr() {
    let decode_calls = Arc::new(AtomicUsize::new(0));
    let decoder = SpyHardwareDecoder;
    let decode_calls_for_factory = decode_calls.clone();
    let mut scripted_results_for_factory = Some(VecDeque::from(vec![Ok(None)]));
    let decoder_factory = Box::new(move || {
        (
            Box::new(ScriptedHardwareDecoder {
                backend_name: "scripted-replacement",
                decode_calls: decode_calls_for_factory.clone(),
                scripted_results: scripted_results_for_factory.take().unwrap_or_default(),
            }) as Box<dyn XbxVideoDecoderBackend>,
            XbxVideoDecoderProbeSummary {
                selected_backend_name: "scripted-replacement".to_string(),
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

    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_recovery_epoch = 9;
    stats.video_anchor_clean_epoch = Some(9);
    stats.video_anchor_clean_observed_at_ms = Some(3_000.0);
    stats.video_anchor_clean_source_event = Some("decoded-usable-idr".to_string());
    stats.host_frame_present_epoch = 1;
    stats.recovery_playback_recovered_at_ms = Some(3_000.0);
    stats.submit_age_ms = Some(1_500.0);
    stats.video_renderer_stalled = Some(true);
    stats.display_age_ms = Some(600.0);
    state.sync_recovery_exit_policy_from_stats(&stats, 3_010.0);
    assert!(state.timed_fallback_displayed_idr_bypass);

    let bootstrap = make_bootstrap_assembled_frame(3_100)
        .into_encoded_frame(Instant::now() + Duration::from_millis(16));
    let committed_parameter_sets = bootstrap.h264.commit_state.clone();
    bootstrap.h264.commit();
    let mut continuation = make_non_idr_continuation_frame(3_101);
    continuation.h264.commit_state = committed_parameter_sets;
    assert!(state.process_encoded_frame(continuation, 3_012.0).is_none());
    assert_eq!(decode_calls.load(Ordering::Relaxed), 1);
}

#[test]
fn waiting_keyframe_after_software_fallback_does_not_allow_unarmed_continuation_stream() {
    let hardware_decode_calls = Arc::new(AtomicUsize::new(0));
    let software_decode_calls = Arc::new(AtomicUsize::new(0));
    let decoder = ScriptedHardwareDecoder {
        backend_name: "ffmpeg-videotoolbox",
        decode_calls: hardware_decode_calls.clone(),
        scripted_results: VecDeque::from([Ok(None), Ok(None), Ok(None), Ok(None)]),
    };
    let software_decode_calls_for_factory = software_decode_calls.clone();
    let software_decoder_factory = Box::new(move || {
        (
            Box::new(ScriptedHardwareDecoder {
                backend_name: "ffmpeg-software",
                decode_calls: software_decode_calls_for_factory.clone(),
                scripted_results: VecDeque::from([Ok(None), Ok(None), Ok(None)]),
            }) as Box<dyn XbxVideoDecoderBackend>,
            XbxVideoDecoderProbeSummary {
                selected_backend_name: "ffmpeg-software".to_string(),
                selected_backend_kind: "software".to_string(),
                fallback_count: 0,
                fallback_summary: None,
            },
        )
    });
    let mut state = XbxVideoDecodeState::new_for_test_with_factories(
        20,
        30,
        Box::new(decoder),
        Box::new(|| panic!("hardware reset path should not be used")),
        software_decoder_factory,
    );

    for step in 0..2 {
        assert!(state
            .process_encoded_frame(make_encoded_frame(true), 3_000.0 + f64::from(step))
            .is_none());
    }

    assert_eq!(hardware_decode_calls.load(Ordering::Relaxed), 2);
    assert_eq!(software_decode_calls.load(Ordering::Relaxed), 0);
    assert_eq!(
        state.recovery_state(),
        XbxVideoRecoveryState::WaitingKeyframe
    );
    let transition = state
        .latest_recovery_transition()
        .expect("software fallback transition should exist");
    assert_eq!(transition.detail, "backendNoOutputSoftFallback");

    let mut post_fallback_continuation = make_encoded_frame(false);
    post_fallback_continuation.rtp_timestamp = 4_100;
    post_fallback_continuation.h264 = H264AccessUnitInspection {
        nals: vec![H264NalUnit {
            range: 0..1,
            unit_type: UnitType::SliceLayerWithoutPartitioningNonIdr,
        }],
        parameter_sets: None,
        width: Some(2560),
        height: Some(1440),
        is_idr: false,
        has_inband_sps: false,
        has_inband_pps: false,
        slice_headers_valid: true,
        parameter_sets_changed: false,
        config_changed: false,
        bootstrap_ready: false,
        bootstrap_reject_reason: Some(H264BootstrapRejectReason::NonIdrVcl),
        commit_state: H264AccessUnitInspector::test_commit_state(),
    };
    assert!(state
        .process_encoded_frame(post_fallback_continuation, 3_100.0)
        .is_none());
    assert_eq!(software_decode_calls.load(Ordering::Relaxed), 0);
    let observation = state
        .latest_decode_output_path_observation()
        .expect("decode output path observation should exist");
    assert_eq!(
        observation.verdict,
        XbxDecodeOutputPathVerdict::BootstrapGateRejected
    );
    assert_eq!(observation.detail, "bootstrapGateRejected");
}

#[test]
fn hardware_backend_no_output_before_first_frame_falls_back_to_software_decoder() {
    let hardware_decode_calls = Arc::new(AtomicUsize::new(0));
    let software_decode_calls = Arc::new(AtomicUsize::new(0));
    let decoder = ScriptedHardwareDecoder {
        backend_name: "ffmpeg-videotoolbox",
        decode_calls: hardware_decode_calls.clone(),
        scripted_results: VecDeque::from([Ok(None), Ok(None), Ok(None), Ok(None)]),
    };
    let decoder_factory = Box::new(|| {
        panic!("hardware reset path should not be used in software fallback test");
    });
    let software_decode_calls_for_factory = software_decode_calls.clone();
    let software_decoder_factory = Box::new(move || {
        (
            Box::new(ScriptedHardwareDecoder {
                backend_name: "ffmpeg-software",
                decode_calls: software_decode_calls_for_factory.clone(),
                scripted_results: VecDeque::from([Ok(Some(XbxRenderFrame {
                    width: 2,
                    height: 2,
                    frame_seq: 88,
                    rendered_at_ms: 88.0,
                    rtp_timestamp: Some(88),
                    recovery_epoch_tag: None,
                    recovery_owner_rtp_timestamp: None,
                    is_keyframe: true,
                    frame_recovery_disposition: Some("repairing".to_string()),
                    frame_unrecoverable_reason: None,
                    presentation_value_role: None,
                    pixel_data: XbxEngineRenderPixelData::Rgba {
                        bytes: Arc::<[u8]>::from([7u8; 16]),
                    },
                }))]),
            }) as Box<dyn XbxVideoDecoderBackend>,
            XbxVideoDecoderProbeSummary {
                selected_backend_name: "ffmpeg-software".to_string(),
                selected_backend_kind: "software".to_string(),
                fallback_count: 0,
                fallback_summary: None,
            },
        )
    });
    let mut state = XbxVideoDecodeState::new_for_test_with_factories(
        20,
        30,
        Box::new(decoder),
        decoder_factory,
        software_decoder_factory,
    );

    for step in 0..2 {
        assert!(state
            .process_encoded_frame(make_encoded_frame(true), 4_000.0 + f64::from(step))
            .is_none());
    }

    assert_eq!(hardware_decode_calls.load(Ordering::Relaxed), 2);
    assert_eq!(state.decoder_backend_name(), "ffmpeg-software");
    assert_eq!(state.decoder_reset_count(), 1);
    let probe = state
        .latest_decoder_probe()
        .expect("software fallback probe should exist");
    assert_eq!(probe.selected_backend_name, "ffmpeg-software");
    assert_eq!(probe.selected_backend_kind, "software");
    assert!(
        probe.fallback_summary.as_deref().is_some_and(
            |summary| summary.contains("ffmpeg-videotoolbox(hardware/backend-no-output)")
        )
    );

    assert!(state
        .process_encoded_frame(make_encoded_frame(true), 4_016.0)
        .is_none());
    assert_eq!(software_decode_calls.load(Ordering::Relaxed), 1);
    assert_eq!(state.decoded_frame_queue_len(), 1);
    assert_eq!(state.recovery_state(), XbxVideoRecoveryState::Recovering);
}

#[test]
fn hardware_nominal_continuation_no_output_falls_back_to_software_decoder() {
    let hardware_decode_calls = Arc::new(AtomicUsize::new(0));
    let software_decode_calls = Arc::new(AtomicUsize::new(0));
    let decoder = ScriptedHardwareDecoder {
        backend_name: "ffmpeg-videotoolbox",
        decode_calls: hardware_decode_calls.clone(),
        scripted_results: VecDeque::from([
            Ok(Some(XbxRenderFrame {
                width: 2,
                height: 2,
                frame_seq: 1,
                rendered_at_ms: 1.0,
                rtp_timestamp: Some(1),
                recovery_epoch_tag: None,
                recovery_owner_rtp_timestamp: None,
                is_keyframe: true,
                frame_recovery_disposition: Some("repairing".to_string()),
                frame_unrecoverable_reason: None,
                presentation_value_role: None,
                pixel_data: XbxEngineRenderPixelData::Rgba {
                    bytes: Arc::<[u8]>::from([4u8; 16]),
                },
            })),
            Ok(None),
            Ok(None),
            Ok(None),
            Ok(None),
        ]),
    };
    let hardware_decode_calls_for_reset = hardware_decode_calls.clone();
    let decoder_factory = Box::new(move || {
        (
            Box::new(ScriptedHardwareDecoder {
                backend_name: "ffmpeg-videotoolbox",
                decode_calls: hardware_decode_calls_for_reset.clone(),
                scripted_results: std::iter::repeat(Ok(None)).take(8).collect(),
            }) as Box<dyn XbxVideoDecoderBackend>,
            XbxVideoDecoderProbeSummary {
                selected_backend_name: "ffmpeg-videotoolbox".to_string(),
                selected_backend_kind: "hardware".to_string(),
                fallback_count: 0,
                fallback_summary: None,
            },
        )
    });
    let software_decode_calls_for_factory = software_decode_calls.clone();
    let software_decoder_factory = Box::new(move || {
        (
            Box::new(ScriptedHardwareDecoder {
                backend_name: "ffmpeg-software",
                decode_calls: software_decode_calls_for_factory.clone(),
                scripted_results: VecDeque::new(),
            }) as Box<dyn XbxVideoDecoderBackend>,
            XbxVideoDecoderProbeSummary {
                selected_backend_name: "ffmpeg-software".to_string(),
                selected_backend_kind: "software".to_string(),
                fallback_count: 0,
                fallback_summary: None,
            },
        )
    });
    let mut state = XbxVideoDecodeState::new_for_test_with_factories(
        20,
        30,
        Box::new(decoder),
        decoder_factory,
        software_decoder_factory,
    );

    assert!(state
        .process_encoded_frame(make_encoded_frame(true), 5_000.0)
        .is_none());
    assert_eq!(state.decoded_frame_queue_len(), 1);
    assert_eq!(state.recovery_state(), XbxVideoRecoveryState::Nominal);

    for step in 0..2 {
        assert!(state
            .process_encoded_frame(
                make_non_idr_continuation_frame(5_100 + step),
                5_016.0 + f64::from(step),
            )
            .is_none());
    }

    assert_eq!(hardware_decode_calls.load(Ordering::Relaxed), 3);
    assert_eq!(state.decoder_reset_count(), 1);
    assert_eq!(state.decoder_backend_name(), "ffmpeg-videotoolbox");
    assert_eq!(
        state.recovery_state(),
        XbxVideoRecoveryState::WaitingKeyframe
    );
    let transition = state
        .latest_recovery_transition()
        .expect("recovery transition should exist");
    assert_eq!(transition.detail, "nominalContinuationNoOutputReset");
    let observation = state
        .latest_decode_output_path_observation()
        .expect("decode output path observation should exist");
    assert_eq!(
        observation.detail,
        "backendNoOutputAfterNominalContinuation"
    );
    assert_eq!(software_decode_calls.load(Ordering::Relaxed), 0);
}

#[test]
fn software_nominal_continuation_no_output_resets_decoder() {
    let decode_calls = Arc::new(AtomicUsize::new(0));
    let reset_calls = Arc::new(AtomicUsize::new(0));
    let decoder = ScriptedHardwareDecoder {
        backend_name: "ffmpeg-software",
        decode_calls: decode_calls.clone(),
        scripted_results: VecDeque::from([
            Ok(Some(XbxRenderFrame {
                width: 2,
                height: 2,
                frame_seq: 1,
                rendered_at_ms: 1.0,
                rtp_timestamp: Some(1),
                recovery_epoch_tag: None,
                recovery_owner_rtp_timestamp: None,
                is_keyframe: true,
                frame_recovery_disposition: Some("repairing".to_string()),
                frame_unrecoverable_reason: None,
                presentation_value_role: None,
                pixel_data: XbxEngineRenderPixelData::Rgba {
                    bytes: Arc::<[u8]>::from([5u8; 16]),
                },
            })),
            Ok(None),
            Ok(None),
        ]),
    };
    let reset_calls_for_factory = reset_calls.clone();
    let decoder_factory = Box::new(move || {
        reset_calls_for_factory.fetch_add(1, Ordering::Relaxed);
        (
            Box::new(ScriptedHardwareDecoder {
                backend_name: "replacement",
                decode_calls: Arc::new(AtomicUsize::new(0)),
                scripted_results: VecDeque::new(),
            }) as Box<dyn XbxVideoDecoderBackend>,
            XbxVideoDecoderProbeSummary {
                selected_backend_name: "replacement".to_string(),
                selected_backend_kind: "software".to_string(),
                fallback_count: 1,
                fallback_summary: Some("reset-recreate".to_string()),
            },
        )
    });
    let mut state =
        XbxVideoDecodeState::new_for_test_with_factory(20, 30, Box::new(decoder), decoder_factory);

    assert!(state
        .process_encoded_frame(make_encoded_frame(true), 6_000.0)
        .is_none());
    assert_eq!(state.decoded_frame_queue_len(), 1);
    assert_eq!(state.recovery_state(), XbxVideoRecoveryState::Nominal);

    for step in 0..2 {
        assert!(state
            .process_encoded_frame(
                make_non_idr_continuation_frame(6_100 + step),
                6_016.0 + f64::from(step),
            )
            .is_none());
    }

    assert_eq!(decode_calls.load(Ordering::Relaxed), 3);
    assert_eq!(reset_calls.load(Ordering::Relaxed), 1);
    assert_eq!(state.decoder_backend_name(), "replacement");
    assert_eq!(state.decoder_reset_count(), 1);
    assert_eq!(
        state.recovery_state(),
        XbxVideoRecoveryState::WaitingKeyframe
    );
    let transition = state
        .latest_recovery_transition()
        .expect("recovery transition should exist");
    assert_eq!(transition.detail, "nominalContinuationNoOutputReset");
    let observation = state
        .latest_decode_output_path_observation()
        .expect("decode output path observation should exist");
    assert_eq!(
        observation.detail,
        "backendNoOutputAfterNominalContinuation"
    );
}

#[test]
fn recovery_fsm_holds_recovering_for_unsettled_recovery_continuation() {
    let decode_calls = Arc::new(AtomicUsize::new(0));
    let decoder = ScriptedHardwareDecoder {
        backend_name: "scripted",
        decode_calls: decode_calls.clone(),
        scripted_results: VecDeque::from([
            Ok(Some(XbxRenderFrame {
                width: 2,
                height: 2,
                frame_seq: 201,
                rendered_at_ms: 201.0,
                rtp_timestamp: Some(201),
                recovery_epoch_tag: Some(9),
                recovery_owner_rtp_timestamp: Some(9001),
                is_keyframe: true,
                frame_recovery_disposition: Some("repairing".to_string()),
                frame_unrecoverable_reason: None,
                presentation_value_role: None,
                pixel_data: XbxEngineRenderPixelData::Rgba {
                    bytes: Arc::<[u8]>::from([8u8; 16]),
                },
            })),
            Ok(Some(XbxRenderFrame {
                width: 2,
                height: 2,
                frame_seq: 202,
                rendered_at_ms: 202.0,
                rtp_timestamp: Some(202),
                recovery_epoch_tag: Some(9),
                recovery_owner_rtp_timestamp: Some(9001),
                is_keyframe: false,
                frame_recovery_disposition: Some("repairing".to_string()),
                frame_unrecoverable_reason: None,
                presentation_value_role: None,
                pixel_data: XbxEngineRenderPixelData::Rgba {
                    bytes: Arc::<[u8]>::from([7u8; 16]),
                },
            })),
        ]),
    };
    let mut state = XbxVideoDecodeState::new_for_test(20, 30, Box::new(decoder));
    state.transition_recovery_state(
        XbxVideoRecoveryState::WaitingKeyframe,
        XbxVideoRecoveryEvent::ExternalDecoderResetRequested,
        "test",
        None,
        None,
        6_990.0,
    );

    let mut bootstrap = make_encoded_frame(true);
    bootstrap.recovery_epoch_tag = Some(9);
    bootstrap.recovery_owner_rtp_timestamp = Some(9001);
    bootstrap.frame_recovery_disposition = FrameRecoveryDisposition::Repairing;
    assert!(state.process_encoded_frame(bootstrap, 7_000.0).is_none());
    assert_eq!(state.recovery_state(), XbxVideoRecoveryState::Recovering);

    let mut continuation = make_non_idr_continuation_frame(9_002);
    continuation.recovery_epoch_tag = Some(9);
    continuation.recovery_owner_rtp_timestamp = Some(9001);
    continuation.frame_recovery_disposition = FrameRecoveryDisposition::Repairing;
    let _ = state.process_encoded_frame(continuation, 7_016.0);
    assert_eq!(state.recovery_state(), XbxVideoRecoveryState::Recovering);
    let transition = state
        .latest_recovery_transition()
        .expect("recovery transition should exist");
    assert_eq!(transition.detail, "bootstrapKeyframeDecoded");
    assert_eq!(decode_calls.load(Ordering::Relaxed), 2);
}

#[test]
fn hardware_recovering_continuation_no_output_falls_back_to_software() {
    let hardware_decode_calls = Arc::new(AtomicUsize::new(0));
    let software_decode_calls = Arc::new(AtomicUsize::new(0));
    let decoder = ScriptedHardwareDecoder {
        backend_name: "ffmpeg-videotoolbox",
        decode_calls: hardware_decode_calls.clone(),
        scripted_results: VecDeque::from([
            Ok(Some(XbxRenderFrame {
                width: 2,
                height: 2,
                frame_seq: 1,
                rendered_at_ms: 1.0,
                rtp_timestamp: Some(1),
                recovery_epoch_tag: Some(10),
                recovery_owner_rtp_timestamp: Some(10_001),
                is_keyframe: true,
                frame_recovery_disposition: Some("repairing".to_string()),
                frame_unrecoverable_reason: None,
                presentation_value_role: None,
                pixel_data: XbxEngineRenderPixelData::Rgba {
                    bytes: Arc::<[u8]>::from([5u8; 16]),
                },
            })),
            Ok(Some(XbxRenderFrame {
                width: 2,
                height: 2,
                frame_seq: 2,
                rendered_at_ms: 2.0,
                rtp_timestamp: Some(2),
                recovery_epoch_tag: Some(10),
                recovery_owner_rtp_timestamp: Some(10_001),
                is_keyframe: false,
                frame_recovery_disposition: Some("repairing".to_string()),
                frame_unrecoverable_reason: None,
                presentation_value_role: None,
                pixel_data: XbxEngineRenderPixelData::Rgba {
                    bytes: Arc::<[u8]>::from([6u8; 16]),
                },
            })),
            Ok(None),
            Ok(None),
        ]),
    };
    let decoder_factory = Box::new(|| {
        (
            Box::new(ScriptedHardwareDecoder {
                backend_name: "replacement-recreated",
                decode_calls: Arc::new(AtomicUsize::new(0)),
                scripted_results: VecDeque::new(),
            }) as Box<dyn XbxVideoDecoderBackend>,
            XbxVideoDecoderProbeSummary {
                selected_backend_name: "replacement-recreated".to_string(),
                selected_backend_kind: "software".to_string(),
                fallback_count: 1,
                fallback_summary: Some("reset-recreate".to_string()),
            },
        )
    });
    let software_decode_calls_for_factory = software_decode_calls.clone();
    let software_decoder_factory = Box::new(move || {
        (
            Box::new(ScriptedHardwareDecoder {
                backend_name: "ffmpeg-software",
                decode_calls: software_decode_calls_for_factory.clone(),
                scripted_results: VecDeque::new(),
            }) as Box<dyn XbxVideoDecoderBackend>,
            XbxVideoDecoderProbeSummary {
                selected_backend_name: "ffmpeg-software".to_string(),
                selected_backend_kind: "software".to_string(),
                fallback_count: 0,
                fallback_summary: None,
            },
        )
    });
    let mut state = XbxVideoDecodeState::new_for_test_with_factories(
        20,
        30,
        Box::new(decoder),
        decoder_factory,
        software_decoder_factory,
    );
    state.transition_recovery_state(
        XbxVideoRecoveryState::WaitingKeyframe,
        XbxVideoRecoveryEvent::ExternalDecoderResetRequested,
        "test",
        None,
        None,
        7_990.0,
    );

    let mut bootstrap = make_encoded_frame(true);
    bootstrap.recovery_epoch_tag = Some(10);
    bootstrap.recovery_owner_rtp_timestamp = Some(10_001);
    bootstrap.frame_recovery_disposition = FrameRecoveryDisposition::Repairing;
    assert!(state.process_encoded_frame(bootstrap, 8_000.0).is_none());
    assert_eq!(state.recovery_state(), XbxVideoRecoveryState::Recovering);

    let mut decoded_continuation = make_non_idr_continuation_frame(10_002);
    decoded_continuation.recovery_epoch_tag = Some(10);
    decoded_continuation.recovery_owner_rtp_timestamp = Some(10_001);
    decoded_continuation.frame_recovery_disposition = FrameRecoveryDisposition::Repairing;
    let _ = state.process_encoded_frame(decoded_continuation, 8_016.0);
    assert_eq!(state.recovery_state(), XbxVideoRecoveryState::Recovering);

    for step in 0..2 {
        let mut continuation = make_non_idr_continuation_frame(10_100 + step);
        continuation.recovery_epoch_tag = Some(10);
        continuation.recovery_owner_rtp_timestamp = Some(10_001);
        continuation.frame_recovery_disposition = FrameRecoveryDisposition::Repairing;
        assert!(state
            .process_encoded_frame(continuation, 8_032.0 + f64::from(step))
            .is_none());
    }

    assert_eq!(hardware_decode_calls.load(Ordering::Relaxed), 4);
    assert_eq!(state.decoder_backend_name(), "ffmpeg-software");
    assert_eq!(state.decoder_reset_count(), 1);
    assert_eq!(
        state.recovery_state(),
        XbxVideoRecoveryState::WaitingKeyframe
    );
    let transition = state
        .latest_recovery_transition()
        .expect("recovery transition should exist");
    assert_eq!(
        transition.detail,
        "recoveringContinuationNoOutputSoftFallback"
    );
    let observation = state
        .latest_decode_output_path_observation()
        .expect("decode output path observation should exist");
    assert_eq!(
        observation.detail,
        "backendNoOutputAfterRecoveringContinuation"
    );
    assert_eq!(software_decode_calls.load(Ordering::Relaxed), 0);
}

#[test]
fn software_recovering_continuation_no_output_resets_decoder() {
    let decode_calls = Arc::new(AtomicUsize::new(0));
    let reset_calls = Arc::new(AtomicUsize::new(0));
    let decoder = ScriptedHardwareDecoder {
        backend_name: "ffmpeg-software",
        decode_calls: decode_calls.clone(),
        scripted_results: VecDeque::from([
            Ok(Some(XbxRenderFrame {
                width: 2,
                height: 2,
                frame_seq: 1,
                rendered_at_ms: 1.0,
                rtp_timestamp: Some(1),
                recovery_epoch_tag: Some(11),
                recovery_owner_rtp_timestamp: Some(11_001),
                is_keyframe: true,
                frame_recovery_disposition: Some("repairing".to_string()),
                frame_unrecoverable_reason: None,
                presentation_value_role: None,
                pixel_data: XbxEngineRenderPixelData::Rgba {
                    bytes: Arc::<[u8]>::from([5u8; 16]),
                },
            })),
            Ok(Some(XbxRenderFrame {
                width: 2,
                height: 2,
                frame_seq: 2,
                rendered_at_ms: 2.0,
                rtp_timestamp: Some(2),
                recovery_epoch_tag: Some(11),
                recovery_owner_rtp_timestamp: Some(11_001),
                is_keyframe: false,
                frame_recovery_disposition: Some("repairing".to_string()),
                frame_unrecoverable_reason: None,
                presentation_value_role: None,
                pixel_data: XbxEngineRenderPixelData::Rgba {
                    bytes: Arc::<[u8]>::from([6u8; 16]),
                },
            })),
            Ok(None),
            Ok(None),
        ]),
    };
    let reset_calls_for_factory = reset_calls.clone();
    let decoder_factory = Box::new(move || {
        reset_calls_for_factory.fetch_add(1, Ordering::Relaxed);
        (
            Box::new(ScriptedHardwareDecoder {
                backend_name: "replacement",
                decode_calls: Arc::new(AtomicUsize::new(0)),
                scripted_results: VecDeque::new(),
            }) as Box<dyn XbxVideoDecoderBackend>,
            XbxVideoDecoderProbeSummary {
                selected_backend_name: "replacement".to_string(),
                selected_backend_kind: "software".to_string(),
                fallback_count: 1,
                fallback_summary: Some("reset-recreate".to_string()),
            },
        )
    });
    let mut state =
        XbxVideoDecodeState::new_for_test_with_factory(20, 30, Box::new(decoder), decoder_factory);
    state.transition_recovery_state(
        XbxVideoRecoveryState::WaitingKeyframe,
        XbxVideoRecoveryEvent::ExternalDecoderResetRequested,
        "test",
        None,
        None,
        8_990.0,
    );

    let mut bootstrap = make_encoded_frame(true);
    bootstrap.recovery_epoch_tag = Some(11);
    bootstrap.recovery_owner_rtp_timestamp = Some(11_001);
    bootstrap.frame_recovery_disposition = FrameRecoveryDisposition::Repairing;
    assert!(state.process_encoded_frame(bootstrap, 9_000.0).is_none());
    assert_eq!(state.recovery_state(), XbxVideoRecoveryState::Recovering);

    let mut decoded_continuation = make_non_idr_continuation_frame(11_002);
    decoded_continuation.recovery_epoch_tag = Some(11);
    decoded_continuation.recovery_owner_rtp_timestamp = Some(11_001);
    decoded_continuation.frame_recovery_disposition = FrameRecoveryDisposition::Repairing;
    let _ = state.process_encoded_frame(decoded_continuation, 9_016.0);
    assert_eq!(state.recovery_state(), XbxVideoRecoveryState::Recovering);

    for step in 0..2 {
        let mut continuation = make_non_idr_continuation_frame(11_100 + step);
        continuation.recovery_epoch_tag = Some(11);
        continuation.recovery_owner_rtp_timestamp = Some(11_001);
        continuation.frame_recovery_disposition = FrameRecoveryDisposition::Repairing;
        assert!(state
            .process_encoded_frame(continuation, 9_032.0 + f64::from(step))
            .is_none());
    }

    assert_eq!(decode_calls.load(Ordering::Relaxed), 4);
    assert_eq!(reset_calls.load(Ordering::Relaxed), 1);
    assert_eq!(state.decoder_backend_name(), "replacement");
    assert_eq!(state.decoder_reset_count(), 1);
    assert_eq!(
        state.recovery_state(),
        XbxVideoRecoveryState::WaitingKeyframe
    );
    let transition = state
        .latest_recovery_transition()
        .expect("recovery transition should exist");
    assert_eq!(transition.detail, "recoveringContinuationNoOutputReset");
    let observation = state
        .latest_decode_output_path_observation()
        .expect("decode output path observation should exist");
    assert_eq!(
        observation.detail,
        "backendNoOutputAfterRecoveringContinuation"
    );
}

#[test]
fn duplicate_local_decoder_reset_without_success_edge_is_coalesced() {
    let decoder = SpyHardwareDecoder;
    let reset_calls = Arc::new(AtomicUsize::new(0));
    let reset_calls_for_factory = reset_calls.clone();
    let decoder_factory = Box::new(move || {
        let call_index = reset_calls_for_factory.fetch_add(1, Ordering::Relaxed);
        (
            Box::new(ScriptedHardwareDecoder {
                backend_name: if call_index == 0 {
                    "replacement-1"
                } else {
                    "replacement-2"
                },
                decode_calls: Arc::new(AtomicUsize::new(0)),
                scripted_results: VecDeque::new(),
            }) as Box<dyn XbxVideoDecoderBackend>,
            XbxVideoDecoderProbeSummary {
                selected_backend_name: "replacement".to_string(),
                selected_backend_kind: "software".to_string(),
                fallback_count: 1,
                fallback_summary: Some("reset-recreate".to_string()),
            },
        )
    });
    let mut state =
        XbxVideoDecodeState::new_for_test_with_factory(20, 30, Box::new(decoder), decoder_factory);

    let first = state
        .request_local_decoder_reset()
        .expect("first decoder reset should succeed");
    let second = state
        .request_local_decoder_reset()
        .expect("duplicate decoder reset should be coalesced");

    assert!(first);
    assert!(!second);
    assert_eq!(reset_calls.load(Ordering::Relaxed), 1);
    assert_eq!(state.decoder_reset_count(), 1);
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
fn decoded_output_mailbox_keeps_only_latest_candidate_under_pressure() {
    let decoder = SpyHardwareDecoder;
    let mut state = XbxVideoDecodeState::new_for_test(20, 30, Box::new(decoder));

    for seq in 1..=4 {
        state.enqueue_decoded_frame_for_test(XbxRenderFrame {
            width: 2,
            height: 2,
            frame_seq: seq,
            rendered_at_ms: seq as f64,
            rtp_timestamp: Some(seq as u32),
            recovery_epoch_tag: None,
            recovery_owner_rtp_timestamp: None,
            is_keyframe: false,
            frame_recovery_disposition: Some("repairing".to_string()),
            frame_unrecoverable_reason: None,
            presentation_value_role: None,
            pixel_data: XbxEngineRenderPixelData::Rgba {
                bytes: Arc::<[u8]>::from([0u8; 16]),
            },
        });
    }

    // mailbox：未 drain 时只保留 latest-only 价值最高候选（steady delta 会 supersede 早期 keyframe）。
    assert_eq!(state.decoded_frame_queue_len(), 1);
    assert_eq!(
        state
            .peek_decoded_frame()
            .map(|frame| frame.surface.frame_seq),
        Some(4)
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
        recovery_epoch_tag: None,
        recovery_owner_rtp_timestamp: None,
        is_keyframe: true,
        frame_recovery_disposition: Some("repairing".to_string()),
        frame_unrecoverable_reason: None,
        presentation_value_role: None,
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
fn enqueue_steady_frame_clears_recovery_disposition_on_render_surface() {
    let decoder = SpyHardwareDecoder;
    let mut state = XbxVideoDecodeState::new_for_test(20, 30, Box::new(decoder));

    let kept = state.enqueue_decoded_frame(DecodedFrame {
        pts: std::time::Instant::now(),
        rtp_timestamp: 42,
        is_keyframe: false,
        recovery_epoch_tag: None,
        recovery_owner_rtp_timestamp: None,
        clean_anchor_commit_recovery_epoch: None,
        presentation_value_role: None,
        budget: Default::default(),
        frame_recovery_disposition: FrameRecoveryDisposition::Steady,
        frame_unrecoverable_reason: None,
        surface: XbxRenderFrame {
            width: 2,
            height: 2,
            frame_seq: 0,
            rendered_at_ms: 0.0,
            rtp_timestamp: None,
            recovery_epoch_tag: None,
            recovery_owner_rtp_timestamp: None,
            is_keyframe: false,
            frame_recovery_disposition: Some("repairing".to_string()),
            frame_unrecoverable_reason: None,
            presentation_value_role: None,
            pixel_data: XbxEngineRenderPixelData::Rgba {
                bytes: Arc::<[u8]>::from([0u8; 16]),
            },
        },
    });

    assert!(kept.is_none());
    let frame = state
        .peek_decoded_frame()
        .expect("steady frame should remain queued");
    assert_eq!(
        frame.frame_recovery_disposition,
        FrameRecoveryDisposition::Steady
    );
    assert_eq!(frame.surface.frame_recovery_disposition, None);
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
        recovery_epoch_tag: None,
        recovery_owner_rtp_timestamp: None,
        is_keyframe: true,
        frame_recovery_disposition: Some("repairing".to_string()),
        frame_unrecoverable_reason: None,
        presentation_value_role: None,
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

    // mailbox full 需要 current(inflight) + latest(candidate) 同时存在。
    state.enqueue_decoded_frame_for_test(XbxRenderFrame {
        width: 2,
        height: 2,
        frame_seq: 1,
        rendered_at_ms: 1.0,
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
    });
    let inflight = state.pop_decoded_frame(2.0).expect("inflight should exist");
    state.requeue_decoded_frame_front(inflight);
    state.enqueue_decoded_frame_for_test(XbxRenderFrame {
        width: 2,
        height: 2,
        frame_seq: 2,
        rendered_at_ms: 2.0,
        rtp_timestamp: Some(2),
        recovery_epoch_tag: None,
        recovery_owner_rtp_timestamp: None,
        is_keyframe: false,
        frame_recovery_disposition: Some("repairing".to_string()),
        frame_unrecoverable_reason: None,
        presentation_value_role: None,
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
        recovery_epoch_tag: None,
        recovery_owner_rtp_timestamp: None,
        is_keyframe: false,
        frame_recovery_disposition: Some("repairing".to_string()),
        frame_unrecoverable_reason: None,
        presentation_value_role: None,
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
        recovery_epoch_tag: None,
        recovery_owner_rtp_timestamp: None,
        is_keyframe: true,
        frame_recovery_disposition: Some("repairing".to_string()),
        frame_unrecoverable_reason: None,
        presentation_value_role: None,
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
        recovery_epoch_tag: None,
        recovery_owner_rtp_timestamp: None,
        is_keyframe: false,
        frame_recovery_disposition: Some("repairing".to_string()),
        frame_unrecoverable_reason: None,
        presentation_value_role: None,
        pixel_data: XbxEngineRenderPixelData::Rgba {
            bytes: Arc::<[u8]>::from([1u8; 16]),
        },
    });
    state.enqueue_decoded_frame_for_test(XbxRenderFrame {
        width: 2,
        height: 2,
        frame_seq: 3,
        rendered_at_ms: 3.0,
        rtp_timestamp: Some(3),
        recovery_epoch_tag: None,
        recovery_owner_rtp_timestamp: None,
        is_keyframe: false,
        frame_recovery_disposition: Some("repairing".to_string()),
        frame_unrecoverable_reason: None,
        presentation_value_role: None,
        pixel_data: XbxEngineRenderPixelData::Rgba {
            bytes: Arc::<[u8]>::from([2u8; 16]),
        },
    });

    let dropped = state.enqueue_decoded_frame(DecodedFrame {
        pts: Instant::now(),
        rtp_timestamp: 4,
        recovery_epoch_tag: None,
        recovery_owner_rtp_timestamp: None,
        is_keyframe: false,
        clean_anchor_commit_recovery_epoch: None,
        presentation_value_role: None,
        budget: crate::media::video::ingress::budget::FrameBudgetContext::default(),
        frame_recovery_disposition: FrameRecoveryDisposition::Repairing,
        frame_unrecoverable_reason: None,
        surface: XbxRenderFrame {
            width: 2,
            height: 2,
            frame_seq: 4,
            rendered_at_ms: 4.0,
            rtp_timestamp: Some(4),
            recovery_epoch_tag: None,
            recovery_owner_rtp_timestamp: None,
            is_keyframe: false,
            frame_recovery_disposition: Some("repairing".to_string()),
            frame_unrecoverable_reason: None,
            presentation_value_role: None,
            pixel_data: XbxEngineRenderPixelData::Rgba {
                bytes: Arc::<[u8]>::from([3u8; 16]),
            },
        },
    });

    // mailbox：anchor 候选在 host 节拍窗内抵御 continuation 突发，丢弃新入帧。
    assert_eq!(dropped.map(|frame| frame.surface.frame_seq), Some(4));
    assert_eq!(state.decoded_frame_drop_count(), 3);
    let decision = state
        .latest_decode_candidate_decision()
        .expect("candidate decision");
    assert_eq!(decision.state, XbxDecodeCandidateState::Backpressure);
    assert_eq!(decision.action, "drop");
    assert_eq!(decision.detail, "coalescedAfterDecode");
    assert_eq!(decision.frame_seq, Some(4));
}

#[test]
fn steady_supply_rapid_updates_supersede_to_latest_not_coalesce_incoming() {
    use crate::media::video::ingress::budget::{FrameBudgetContext, FrameBudgetLinkValue};

    let decoder = SpyHardwareDecoder;
    let mut state = XbxVideoDecodeState::new_for_test(20, 30, Box::new(decoder));
    state.set_mailbox_present_cadence(16.0);
    let budget = FrameBudgetContext {
        link_value: FrameBudgetLinkValue::Supply,
        ..FrameBudgetContext::default()
    };
    let now = Instant::now();

    assert!(state
        .enqueue_decoded_frame(DecodedFrame {
            pts: now,
            rtp_timestamp: 100,
            recovery_epoch_tag: None,
            recovery_owner_rtp_timestamp: None,
            is_keyframe: false,
            clean_anchor_commit_recovery_epoch: None,
            presentation_value_role: None,
            budget,
            frame_recovery_disposition: FrameRecoveryDisposition::Steady,
            frame_unrecoverable_reason: None,
            surface: XbxRenderFrame {
                width: 2,
                height: 2,
                frame_seq: 10,
                rendered_at_ms: 1_000.0,
                rtp_timestamp: Some(100),
                recovery_epoch_tag: None,
                recovery_owner_rtp_timestamp: None,
                is_keyframe: false,
                frame_recovery_disposition: Some("steady".to_string()),
                frame_unrecoverable_reason: None,
                presentation_value_role: None,
                pixel_data: XbxEngineRenderPixelData::Rgba {
                    bytes: Arc::<[u8]>::from([0u8; 16]),
                },
            },
        })
        .is_none());

    let dropped = state.enqueue_decoded_frame(DecodedFrame {
        pts: now + Duration::from_millis(5),
        rtp_timestamp: 101,
        recovery_epoch_tag: None,
        recovery_owner_rtp_timestamp: None,
        is_keyframe: false,
        clean_anchor_commit_recovery_epoch: None,
        presentation_value_role: None,
        budget,
        frame_recovery_disposition: FrameRecoveryDisposition::Steady,
        frame_unrecoverable_reason: None,
        surface: XbxRenderFrame {
            width: 2,
            height: 2,
            frame_seq: 11,
            rendered_at_ms: 1_005.0,
            rtp_timestamp: Some(101),
            recovery_epoch_tag: None,
            recovery_owner_rtp_timestamp: None,
            is_keyframe: false,
            frame_recovery_disposition: Some("steady".to_string()),
            frame_unrecoverable_reason: None,
            presentation_value_role: None,
            pixel_data: XbxEngineRenderPixelData::Rgba {
                bytes: Arc::<[u8]>::from([1u8; 16]),
            },
        },
    });

    assert_eq!(dropped.map(|frame| frame.surface.frame_seq), Some(10));
    assert_eq!(
        state
            .peek_decoded_frame()
            .map(|frame| frame.surface.frame_seq),
        Some(11)
    );
    let decision = state
        .latest_decode_candidate_decision()
        .expect("candidate decision");
    assert_eq!(decision.detail, "supersededAfterDecode");
    assert_eq!(decision.frame_seq, Some(10));
}

#[test]
fn bootstrap_config_change_idr_outranks_newer_steady_mailbox_candidate() {
    use crate::api::backend::XbxEnginePresentationValueRole;

    let decoder = SpyHardwareDecoder;
    let mut state = XbxVideoDecodeState::new_for_test(20, 30, Box::new(decoder));
    let now = Instant::now();

    state.enqueue_decoded_frame(DecodedFrame {
        pts: now,
        rtp_timestamp: 9_000,
        is_keyframe: true,
        recovery_epoch_tag: Some(1),
        recovery_owner_rtp_timestamp: None,
        clean_anchor_commit_recovery_epoch: Some(1),
        presentation_value_role: Some(XbxEnginePresentationValueRole::FreshAnchor),
        budget: crate::media::video::ingress::budget::FrameBudgetContext::default(),
        frame_recovery_disposition: FrameRecoveryDisposition::Repairing,
        frame_unrecoverable_reason: None,
        surface: XbxRenderFrame {
            width: 2,
            height: 2,
            frame_seq: 1,
            rendered_at_ms: 1.0,
            rtp_timestamp: Some(9_000),
            recovery_epoch_tag: Some(1),
            recovery_owner_rtp_timestamp: None,
            is_keyframe: true,
            frame_recovery_disposition: Some("repairing".to_string()),
            frame_unrecoverable_reason: None,
            presentation_value_role: Some("fresh_anchor".to_string()),
            pixel_data: XbxEngineRenderPixelData::Rgba {
                bytes: Arc::<[u8]>::from([1u8; 16]),
            },
        },
    });
    let dropped = state.enqueue_decoded_frame(DecodedFrame {
        pts: now + Duration::from_millis(16),
        rtp_timestamp: 9_016,
        is_keyframe: false,
        recovery_epoch_tag: Some(1),
        recovery_owner_rtp_timestamp: None,
        clean_anchor_commit_recovery_epoch: None,
        presentation_value_role: Some(XbxEnginePresentationValueRole::SteadyContinuation),
        budget: crate::media::video::ingress::budget::FrameBudgetContext::default(),
        frame_recovery_disposition: FrameRecoveryDisposition::Steady,
        frame_unrecoverable_reason: None,
        surface: XbxRenderFrame {
            width: 2,
            height: 2,
            frame_seq: 2,
            rendered_at_ms: 2.0,
            rtp_timestamp: Some(9_016),
            recovery_epoch_tag: Some(1),
            recovery_owner_rtp_timestamp: None,
            is_keyframe: false,
            frame_recovery_disposition: Some("steady".to_string()),
            frame_unrecoverable_reason: None,
            presentation_value_role: Some("steady_continuation".to_string()),
            pixel_data: XbxEngineRenderPixelData::Rgba {
                bytes: Arc::<[u8]>::from([2u8; 16]),
            },
        },
    });

    assert_eq!(dropped.map(|frame| frame.surface.frame_seq), Some(2));
    let kept = state
        .pop_decoded_frame(200.0)
        .expect("bootstrap idr should remain mailbox candidate");
    assert_eq!(kept.surface.frame_seq, 1);
    assert_eq!(kept.rtp_timestamp, 9_000);
}

#[test]
fn enqueue_decoded_frame_recovery_window_does_not_expand_decode_output_mailbox_capacity() {
    let decoder = SpyHardwareDecoder;
    let mut state = XbxVideoDecodeState::new_for_test(20, 30, Box::new(decoder));

    for seq in 1..=5 {
        let mut frame = DecodedFrame {
            pts: Instant::now(),
            rtp_timestamp: seq,
            recovery_epoch_tag: None,
            recovery_owner_rtp_timestamp: None,
            is_keyframe: seq == 1,
            clean_anchor_commit_recovery_epoch: None,
            presentation_value_role: None,
            budget: crate::media::video::ingress::budget::FrameBudgetContext::for_transport(
                crate::media::video::types::FrameValue::new(seq == 1, false, 1024),
                false,
                Some(30.0),
                Some(1_000.0),
                Some(1_016.0),
                false,
                FrameBudgetWindowSource::Recovery,
            ),
            frame_recovery_disposition: FrameRecoveryDisposition::Repairing,
            frame_unrecoverable_reason: None,
            surface: XbxRenderFrame {
                width: 2,
                height: 2,
                frame_seq: seq as u64,
                rendered_at_ms: seq as f64,
                rtp_timestamp: Some(seq),
                recovery_epoch_tag: None,
                recovery_owner_rtp_timestamp: None,
                is_keyframe: seq == 1,
                frame_recovery_disposition: Some("repairing".to_string()),
                frame_unrecoverable_reason: None,
                presentation_value_role: None,
                pixel_data: XbxEngineRenderPixelData::Rgba {
                    bytes: Arc::<[u8]>::from([seq as u8; 16]),
                },
            },
        };
        frame.pts = Instant::now();
        let dropped = state.enqueue_decoded_frame(frame);
        if seq == 1 {
            assert!(dropped.is_none(), "first recovery frame should be accepted");
        } else {
            assert!(
                dropped.is_some(),
                "mailbox keeps only latest; earlier candidate is superseded"
            );
        }
    }

    assert_eq!(state.decoded_frame_queue_len(), 1);
    assert!(state.decoded_frame_drop_count() >= 1);
}

#[test]
fn newer_recovery_epoch_decoded_frame_supersedes_older_epoch_candidate() {
    let decoder = SpyHardwareDecoder;
    let mut state = XbxVideoDecodeState::new_for_test(20, 30, Box::new(decoder));

    let older = DecodedFrame {
        pts: Instant::now(),
        rtp_timestamp: 100,
        recovery_epoch_tag: Some(3),
        recovery_owner_rtp_timestamp: None,
        is_keyframe: true,
        clean_anchor_commit_recovery_epoch: None,
        presentation_value_role: None,
        budget: crate::media::video::ingress::budget::FrameBudgetContext::default(),
        frame_recovery_disposition: FrameRecoveryDisposition::Repairing,
        frame_unrecoverable_reason: None,
        surface: XbxRenderFrame {
            width: 2,
            height: 2,
            frame_seq: 100,
            rendered_at_ms: 100.0,
            rtp_timestamp: Some(100),
            recovery_epoch_tag: Some(3),
            recovery_owner_rtp_timestamp: None,
            is_keyframe: true,
            frame_recovery_disposition: Some("repairing".to_string()),
            frame_unrecoverable_reason: None,
            presentation_value_role: None,
            pixel_data: XbxEngineRenderPixelData::Rgba {
                bytes: Arc::<[u8]>::from([1u8; 16]),
            },
        },
    };
    let newer = DecodedFrame {
        pts: Instant::now(),
        rtp_timestamp: 90,
        recovery_epoch_tag: Some(4),
        recovery_owner_rtp_timestamp: None,
        is_keyframe: false,
        clean_anchor_commit_recovery_epoch: None,
        presentation_value_role: None,
        budget: crate::media::video::ingress::budget::FrameBudgetContext::default(),
        frame_recovery_disposition: FrameRecoveryDisposition::Repairing,
        frame_unrecoverable_reason: None,
        surface: XbxRenderFrame {
            width: 2,
            height: 2,
            frame_seq: 90,
            rendered_at_ms: 90.0,
            rtp_timestamp: Some(90),
            recovery_epoch_tag: Some(4),
            recovery_owner_rtp_timestamp: None,
            is_keyframe: false,
            frame_recovery_disposition: Some("repairing".to_string()),
            frame_unrecoverable_reason: None,
            presentation_value_role: None,
            pixel_data: XbxEngineRenderPixelData::Rgba {
                bytes: Arc::<[u8]>::from([2u8; 16]),
            },
        },
    };

    assert!(state.enqueue_decoded_frame(older).is_none());
    let dropped = state
        .enqueue_decoded_frame(newer)
        .expect("older keyframe should be superseded by higher recovery epoch");

    assert_eq!(dropped.surface.frame_seq, 100);
    let kept = state.pop_decoded_frame(200.0).expect("kept frame");
    assert_eq!(kept.recovery_epoch_tag, Some(4));
    assert_eq!(kept.surface.frame_seq, 90);
}

#[test]
fn owner_frame_in_same_recovery_epoch_supersedes_non_owner_candidate() {
    let decoder = SpyHardwareDecoder;
    let mut state = XbxVideoDecodeState::new_for_test(20, 30, Box::new(decoder));

    let non_owner = DecodedFrame {
        pts: Instant::now(),
        rtp_timestamp: 121,
        recovery_epoch_tag: Some(5),
        recovery_owner_rtp_timestamp: Some(120),
        is_keyframe: false,
        clean_anchor_commit_recovery_epoch: None,
        presentation_value_role: None,
        budget: crate::media::video::ingress::budget::FrameBudgetContext::default(),
        frame_recovery_disposition: FrameRecoveryDisposition::Repairing,
        frame_unrecoverable_reason: None,
        surface: XbxRenderFrame {
            width: 2,
            height: 2,
            frame_seq: 121,
            rendered_at_ms: 121.0,
            rtp_timestamp: Some(121),
            recovery_epoch_tag: Some(5),
            recovery_owner_rtp_timestamp: Some(120),
            is_keyframe: false,
            frame_recovery_disposition: Some("repairing".to_string()),
            frame_unrecoverable_reason: None,
            presentation_value_role: None,
            pixel_data: XbxEngineRenderPixelData::Rgba {
                bytes: Arc::<[u8]>::from([3u8; 16]),
            },
        },
    };
    let owner = DecodedFrame {
        pts: Instant::now(),
        rtp_timestamp: 120,
        recovery_epoch_tag: Some(5),
        recovery_owner_rtp_timestamp: Some(120),
        is_keyframe: true,
        clean_anchor_commit_recovery_epoch: None,
        presentation_value_role: None,
        budget: crate::media::video::ingress::budget::FrameBudgetContext::default(),
        frame_recovery_disposition: FrameRecoveryDisposition::Repairing,
        frame_unrecoverable_reason: None,
        surface: XbxRenderFrame {
            width: 2,
            height: 2,
            frame_seq: 120,
            rendered_at_ms: 120.0,
            rtp_timestamp: Some(120),
            recovery_epoch_tag: Some(5),
            recovery_owner_rtp_timestamp: Some(120),
            is_keyframe: true,
            frame_recovery_disposition: Some("repairing".to_string()),
            frame_unrecoverable_reason: None,
            presentation_value_role: None,
            pixel_data: XbxEngineRenderPixelData::Rgba {
                bytes: Arc::<[u8]>::from([4u8; 16]),
            },
        },
    };

    assert!(state.enqueue_decoded_frame(non_owner).is_none());
    let dropped = state
        .enqueue_decoded_frame(owner)
        .expect("non-owner candidate should be superseded by owner keyframe");

    assert_eq!(dropped.rtp_timestamp, 121);
    let kept = state.pop_decoded_frame(200.0).expect("kept frame");
    assert_eq!(kept.rtp_timestamp, 120);
    assert_eq!(kept.recovery_owner_rtp_timestamp, Some(120));
}

#[test]
fn enqueue_decoded_frame_recovering_state_does_not_expand_decode_output_mailbox_capacity() {
    let decoder = SpyHardwareDecoder;
    let mut state = XbxVideoDecodeState::new_for_test(20, 30, Box::new(decoder));
    state.transition_recovery_state(
        XbxVideoRecoveryState::Recovering,
        XbxVideoRecoveryEvent::BootstrapKeyframeAccepted,
        "test",
        None,
        None,
        1.0,
    );

    for seq in 1..=5 {
        let dropped = state.enqueue_decoded_frame(DecodedFrame {
            pts: Instant::now(),
            rtp_timestamp: seq,
            recovery_epoch_tag: None,
            recovery_owner_rtp_timestamp: None,
            is_keyframe: false,
            clean_anchor_commit_recovery_epoch: None,
            presentation_value_role: None,
            budget: crate::media::video::ingress::budget::FrameBudgetContext::default(),
            frame_recovery_disposition: FrameRecoveryDisposition::Repairing,
            frame_unrecoverable_reason: None,
            surface: XbxRenderFrame {
                width: 2,
                height: 2,
                frame_seq: seq as u64,
                rendered_at_ms: seq as f64,
                rtp_timestamp: Some(seq),
                recovery_epoch_tag: None,
                recovery_owner_rtp_timestamp: None,
                is_keyframe: false,
                frame_recovery_disposition: Some("repairing".to_string()),
                frame_unrecoverable_reason: None,
                presentation_value_role: None,
                pixel_data: XbxEngineRenderPixelData::Rgba {
                    bytes: Arc::<[u8]>::from([seq as u8; 16]),
                },
            },
        });
        if seq == 1 {
            assert!(dropped.is_none());
        } else {
            assert!(
                dropped.is_some(),
                "mailbox keeps only latest even during recovering"
            );
        }
    }

    assert_eq!(state.decoded_frame_queue_len(), 1);
    assert!(state.decoded_frame_drop_count() >= 1);
}

#[test]
fn enqueue_decoded_frame_drops_stale_frame_before_queueing() {
    let decoder = SpyHardwareDecoder;
    let mut state = XbxVideoDecodeState::new_for_test(20, 30, Box::new(decoder));

    let dropped = state.enqueue_decoded_frame(DecodedFrame {
        pts: Instant::now() - Duration::from_millis(40),
        rtp_timestamp: 7,
        recovery_epoch_tag: None,
        recovery_owner_rtp_timestamp: None,
        is_keyframe: false,
        clean_anchor_commit_recovery_epoch: None,
        presentation_value_role: None,
        budget: crate::media::video::ingress::budget::FrameBudgetContext::default(),
        frame_recovery_disposition: FrameRecoveryDisposition::Repairing,
        frame_unrecoverable_reason: None,
        surface: XbxRenderFrame {
            width: 2,
            height: 2,
            frame_seq: 7,
            rendered_at_ms: 7.0,
            rtp_timestamp: Some(7),
            recovery_epoch_tag: None,
            recovery_owner_rtp_timestamp: None,
            is_keyframe: false,
            frame_recovery_disposition: Some("repairing".to_string()),
            frame_unrecoverable_reason: None,
            presentation_value_role: None,
            pixel_data: XbxEngineRenderPixelData::Rgba {
                bytes: Arc::<[u8]>::from([7u8; 16]),
            },
        },
    });

    assert_eq!(dropped.map(|frame| frame.surface.frame_seq), Some(7));
    assert_eq!(state.decoded_frame_queue_len(), 0);
    assert_eq!(state.decoded_frame_drop_count(), 1);
    let decision = state
        .latest_decode_candidate_decision()
        .expect("candidate decision");
    assert_eq!(decision.action, "drop");
    assert_eq!(decision.detail, "staleAfterDecode");
    assert_eq!(decision.frame_seq, Some(7));
}

#[test]
fn enqueue_decoded_frame_recovery_window_gets_extra_stale_slack() {
    let decoder = SpyHardwareDecoder;
    let mut state = XbxVideoDecodeState::new_for_test(20, 30, Box::new(decoder));

    let dropped = state.enqueue_decoded_frame(DecodedFrame {
        pts: Instant::now() - Duration::from_millis(40),
        rtp_timestamp: 9,
        recovery_epoch_tag: None,
        recovery_owner_rtp_timestamp: None,
        is_keyframe: false,
        clean_anchor_commit_recovery_epoch: None,
        presentation_value_role: None,
        budget: crate::media::video::ingress::budget::FrameBudgetContext::for_transport(
            crate::media::video::types::FrameValue::new(false, false, 1024),
            false,
            Some(30.0),
            Some(1_000.0),
            Some(1_016.0),
            false,
            FrameBudgetWindowSource::Recovery,
        ),
        frame_recovery_disposition: FrameRecoveryDisposition::Repairing,
        frame_unrecoverable_reason: None,
        surface: XbxRenderFrame {
            width: 2,
            height: 2,
            frame_seq: 9,
            rendered_at_ms: 9.0,
            rtp_timestamp: Some(9),
            recovery_epoch_tag: None,
            recovery_owner_rtp_timestamp: None,
            is_keyframe: false,
            frame_recovery_disposition: Some("repairing".to_string()),
            frame_unrecoverable_reason: None,
            presentation_value_role: None,
            pixel_data: XbxEngineRenderPixelData::Rgba {
                bytes: Arc::<[u8]>::from([9u8; 16]),
            },
        },
    });

    assert!(dropped.is_none());
    assert_eq!(state.decoded_frame_queue_len(), 1);
    assert_eq!(state.decoded_frame_drop_count(), 0);
}

#[test]
fn enqueue_decoded_frame_uses_30fps_mailbox_interval_hint_before_stale_drop() {
    let decoder = SpyHardwareDecoder;
    let mut state = XbxVideoDecodeState::new_for_test(20, 30, Box::new(decoder));

    let existing = DecodedFrame {
        pts: Instant::now() - Duration::from_millis(35),
        rtp_timestamp: 30,
        recovery_epoch_tag: None,
        recovery_owner_rtp_timestamp: None,
        is_keyframe: false,
        clean_anchor_commit_recovery_epoch: None,
        presentation_value_role: None,
        budget: crate::media::video::ingress::budget::FrameBudgetContext::for_transport(
            crate::media::video::types::FrameValue::new(false, false, 1024),
            false,
            Some(30.0),
            Some(1_000.0),
            Some(1_016.0),
            false,
            FrameBudgetWindowSource::Recovery,
        ),
        frame_recovery_disposition: FrameRecoveryDisposition::Repairing,
        frame_unrecoverable_reason: None,
        surface: XbxRenderFrame {
            width: 2,
            height: 2,
            frame_seq: 30,
            rendered_at_ms: 30.0,
            rtp_timestamp: Some(30),
            recovery_epoch_tag: None,
            recovery_owner_rtp_timestamp: None,
            is_keyframe: false,
            frame_recovery_disposition: Some("repairing".to_string()),
            frame_unrecoverable_reason: None,
            presentation_value_role: None,
            pixel_data: XbxEngineRenderPixelData::Rgba {
                bytes: Arc::<[u8]>::from([30u8; 16]),
            },
        },
    };
    assert!(state.enqueue_decoded_frame(existing).is_none());

    let dropped = state.enqueue_decoded_frame(DecodedFrame {
        pts: Instant::now() - Duration::from_millis(1),
        rtp_timestamp: 31,
        recovery_epoch_tag: None,
        recovery_owner_rtp_timestamp: None,
        is_keyframe: false,
        clean_anchor_commit_recovery_epoch: None,
        presentation_value_role: None,
        budget: crate::media::video::ingress::budget::FrameBudgetContext::default(),
        frame_recovery_disposition: FrameRecoveryDisposition::Repairing,
        frame_unrecoverable_reason: None,
        surface: XbxRenderFrame {
            width: 2,
            height: 2,
            frame_seq: 31,
            rendered_at_ms: 31.0,
            rtp_timestamp: Some(31),
            recovery_epoch_tag: None,
            recovery_owner_rtp_timestamp: None,
            is_keyframe: false,
            frame_recovery_disposition: Some("repairing".to_string()),
            frame_unrecoverable_reason: None,
            presentation_value_role: None,
            pixel_data: XbxEngineRenderPixelData::Rgba {
                bytes: Arc::<[u8]>::from([31u8; 16]),
            },
        },
    });

    assert!(
        dropped.is_some(),
        "newer 30fps frame should supersede old candidate, not be stale"
    );
    let decision = state
        .latest_decode_candidate_decision()
        .expect("candidate decision");
    assert_ne!(decision.detail, "staleAfterDecode");
}

#[test]
fn enqueue_decoded_frame_uses_60fps_mailbox_interval_hint_before_stale_drop() {
    let decoder = SpyHardwareDecoder;
    let mut state = XbxVideoDecodeState::new_for_test(20, 30, Box::new(decoder));

    let existing = DecodedFrame {
        pts: Instant::now() - Duration::from_millis(17),
        rtp_timestamp: 60,
        recovery_epoch_tag: None,
        recovery_owner_rtp_timestamp: None,
        is_keyframe: false,
        clean_anchor_commit_recovery_epoch: None,
        presentation_value_role: None,
        budget: crate::media::video::ingress::budget::FrameBudgetContext::default(),
        frame_recovery_disposition: FrameRecoveryDisposition::Repairing,
        frame_unrecoverable_reason: None,
        surface: XbxRenderFrame {
            width: 2,
            height: 2,
            frame_seq: 60,
            rendered_at_ms: 60.0,
            rtp_timestamp: Some(60),
            recovery_epoch_tag: None,
            recovery_owner_rtp_timestamp: None,
            is_keyframe: false,
            frame_recovery_disposition: Some("repairing".to_string()),
            frame_unrecoverable_reason: None,
            presentation_value_role: None,
            pixel_data: XbxEngineRenderPixelData::Rgba {
                bytes: Arc::<[u8]>::from([60u8; 16]),
            },
        },
    };
    assert!(state.enqueue_decoded_frame(existing).is_none());

    let dropped = state.enqueue_decoded_frame(DecodedFrame {
        pts: Instant::now() - Duration::from_millis(1),
        rtp_timestamp: 61,
        recovery_epoch_tag: None,
        recovery_owner_rtp_timestamp: None,
        is_keyframe: false,
        clean_anchor_commit_recovery_epoch: None,
        presentation_value_role: None,
        budget: crate::media::video::ingress::budget::FrameBudgetContext::default(),
        frame_recovery_disposition: FrameRecoveryDisposition::Repairing,
        frame_unrecoverable_reason: None,
        surface: XbxRenderFrame {
            width: 2,
            height: 2,
            frame_seq: 61,
            rendered_at_ms: 61.0,
            rtp_timestamp: Some(61),
            recovery_epoch_tag: None,
            recovery_owner_rtp_timestamp: None,
            is_keyframe: false,
            frame_recovery_disposition: Some("repairing".to_string()),
            frame_unrecoverable_reason: None,
            presentation_value_role: None,
            pixel_data: XbxEngineRenderPixelData::Rgba {
                bytes: Arc::<[u8]>::from([61u8; 16]),
            },
        },
    });

    assert!(
        dropped.is_some(),
        "newer 60fps frame should supersede old candidate, not be stale"
    );
    let decision = state
        .latest_decode_candidate_decision()
        .expect("candidate decision");
    assert_ne!(decision.detail, "staleAfterDecode");
}

#[test]
fn repairing_frame_is_kept_over_plain_delta_with_same_epoch() {
    let decoder = SpyHardwareDecoder;
    let mut state = XbxVideoDecodeState::new_for_test(20, 30, Box::new(decoder));

    let repairing = DecodedFrame {
        pts: Instant::now(),
        rtp_timestamp: 100,
        recovery_epoch_tag: Some(7),
        recovery_owner_rtp_timestamp: None,
        is_keyframe: false,
        clean_anchor_commit_recovery_epoch: None,
        presentation_value_role: None,
        budget: crate::media::video::ingress::budget::FrameBudgetContext::default(),
        frame_recovery_disposition: FrameRecoveryDisposition::Repairing,
        frame_unrecoverable_reason: None,
        surface: XbxRenderFrame {
            width: 2,
            height: 2,
            frame_seq: 100,
            rendered_at_ms: 100.0,
            rtp_timestamp: Some(100),
            recovery_epoch_tag: Some(7),
            recovery_owner_rtp_timestamp: None,
            is_keyframe: false,
            frame_recovery_disposition: Some("repairing".to_string()),
            frame_unrecoverable_reason: None,
            presentation_value_role: None,
            pixel_data: XbxEngineRenderPixelData::Rgba {
                bytes: Arc::<[u8]>::from([1u8; 16]),
            },
        },
    };
    let plain_delta = DecodedFrame {
        pts: Instant::now(),
        rtp_timestamp: 101,
        recovery_epoch_tag: Some(7),
        recovery_owner_rtp_timestamp: None,
        is_keyframe: false,
        clean_anchor_commit_recovery_epoch: None,
        presentation_value_role: None,
        budget: crate::media::video::ingress::budget::FrameBudgetContext::default(),
        frame_recovery_disposition: FrameRecoveryDisposition::UnrecoverableLate,
        frame_unrecoverable_reason: Some("continuationOnly".to_string()),
        surface: XbxRenderFrame {
            width: 2,
            height: 2,
            frame_seq: 101,
            rendered_at_ms: 101.0,
            rtp_timestamp: Some(101),
            recovery_epoch_tag: Some(7),
            recovery_owner_rtp_timestamp: None,
            is_keyframe: false,
            frame_recovery_disposition: Some("abandonedLate".to_string()),
            frame_unrecoverable_reason: Some("continuationOnly".to_string()),
            presentation_value_role: None,
            pixel_data: XbxEngineRenderPixelData::Rgba {
                bytes: Arc::<[u8]>::from([2u8; 16]),
            },
        },
    };

    assert!(state.enqueue_decoded_frame(repairing).is_none());
    let dropped = state
        .enqueue_decoded_frame(plain_delta)
        .expect("plain delta should lose to repairing frame");

    assert_eq!(dropped.surface.frame_seq, 101);
    let kept = state.pop_decoded_frame(200.0).expect("kept frame");
    assert_eq!(kept.surface.frame_seq, 100);
    assert_eq!(
        kept.frame_recovery_disposition,
        FrameRecoveryDisposition::Repairing
    );
}

#[test]
fn steady_continuation_does_not_replace_recovery_continuation_with_same_epoch() {
    let decoder = SpyHardwareDecoder;
    let mut state = XbxVideoDecodeState::new_for_test(20, 30, Box::new(decoder));

    let repairing = DecodedFrame {
        pts: Instant::now(),
        rtp_timestamp: 110,
        recovery_epoch_tag: Some(8),
        recovery_owner_rtp_timestamp: None,
        is_keyframe: false,
        clean_anchor_commit_recovery_epoch: None,
        presentation_value_role: None,
        budget: crate::media::video::ingress::budget::FrameBudgetContext::default(),
        frame_recovery_disposition: FrameRecoveryDisposition::Repairing,
        frame_unrecoverable_reason: None,
        surface: XbxRenderFrame {
            width: 2,
            height: 2,
            frame_seq: 110,
            rendered_at_ms: 110.0,
            rtp_timestamp: Some(110),
            recovery_epoch_tag: Some(8),
            recovery_owner_rtp_timestamp: None,
            is_keyframe: false,
            frame_recovery_disposition: Some("repairing".to_string()),
            frame_unrecoverable_reason: None,
            presentation_value_role: None,
            pixel_data: XbxEngineRenderPixelData::Rgba {
                bytes: Arc::<[u8]>::from([3u8; 16]),
            },
        },
    };
    let plain_delta = DecodedFrame {
        pts: Instant::now(),
        rtp_timestamp: 111,
        recovery_epoch_tag: Some(8),
        recovery_owner_rtp_timestamp: None,
        is_keyframe: false,
        clean_anchor_commit_recovery_epoch: None,
        presentation_value_role: None,
        budget: crate::media::video::ingress::budget::FrameBudgetContext::default(),
        frame_recovery_disposition: FrameRecoveryDisposition::Steady,
        frame_unrecoverable_reason: None,
        surface: XbxRenderFrame {
            width: 2,
            height: 2,
            frame_seq: 111,
            rendered_at_ms: 111.0,
            rtp_timestamp: Some(111),
            recovery_epoch_tag: Some(8),
            recovery_owner_rtp_timestamp: None,
            is_keyframe: false,
            frame_recovery_disposition: None,
            frame_unrecoverable_reason: None,
            presentation_value_role: None,
            pixel_data: XbxEngineRenderPixelData::Rgba {
                bytes: Arc::<[u8]>::from([4u8; 16]),
            },
        },
    };

    assert!(state.enqueue_decoded_frame(repairing).is_none());
    let dropped = state
        .enqueue_decoded_frame(plain_delta)
        .expect("steady continuation should be dropped");

    assert_eq!(dropped.surface.frame_seq, 111);
    let kept = state.pop_decoded_frame(200.0).expect("kept frame");
    assert_eq!(kept.surface.frame_seq, 110);
    assert_eq!(
        kept.frame_recovery_disposition,
        FrameRecoveryDisposition::Repairing
    );
}

#[test]
fn owner_rebuilding_supply_replaces_non_owner_rebuilding_supply_in_same_epoch() {
    let decoder = SpyHardwareDecoder;
    let mut state = XbxVideoDecodeState::new_for_test(20, 30, Box::new(decoder));

    let non_owner = DecodedFrame {
        pts: Instant::now(),
        rtp_timestamp: 220,
        recovery_epoch_tag: Some(12),
        recovery_owner_rtp_timestamp: Some(200),
        is_keyframe: false,
        clean_anchor_commit_recovery_epoch: None,
        presentation_value_role: None,
        budget: crate::media::video::ingress::budget::FrameBudgetContext::for_transport(
            crate::media::video::types::FrameValue::new(false, true, 1024),
            false,
            Some(30.0),
            Some(1_000.0),
            Some(1_016.0),
            false,
            FrameBudgetWindowSource::Recovery,
        ),
        frame_recovery_disposition: FrameRecoveryDisposition::Repairing,
        frame_unrecoverable_reason: None,
        surface: XbxRenderFrame {
            width: 2,
            height: 2,
            frame_seq: 220,
            rendered_at_ms: 220.0,
            rtp_timestamp: Some(220),
            recovery_epoch_tag: Some(12),
            recovery_owner_rtp_timestamp: Some(200),
            is_keyframe: false,
            frame_recovery_disposition: Some("rebuilding-supply".to_string()),
            frame_unrecoverable_reason: None,
            presentation_value_role: None,
            pixel_data: XbxEngineRenderPixelData::Rgba {
                bytes: Arc::<[u8]>::from([5u8; 16]),
            },
        },
    };
    let owner = DecodedFrame {
        pts: Instant::now(),
        rtp_timestamp: 200,
        recovery_epoch_tag: Some(12),
        recovery_owner_rtp_timestamp: Some(200),
        is_keyframe: false,
        clean_anchor_commit_recovery_epoch: None,
        presentation_value_role: None,
        budget: crate::media::video::ingress::budget::FrameBudgetContext::for_transport(
            crate::media::video::types::FrameValue::new(false, true, 1024),
            false,
            Some(30.0),
            Some(1_000.0),
            Some(1_016.0),
            false,
            FrameBudgetWindowSource::Recovery,
        ),
        frame_recovery_disposition: FrameRecoveryDisposition::Repairing,
        frame_unrecoverable_reason: None,
        surface: XbxRenderFrame {
            width: 2,
            height: 2,
            frame_seq: 200,
            rendered_at_ms: 200.0,
            rtp_timestamp: Some(200),
            recovery_epoch_tag: Some(12),
            recovery_owner_rtp_timestamp: Some(200),
            is_keyframe: false,
            frame_recovery_disposition: Some("rebuilding-supply".to_string()),
            frame_unrecoverable_reason: None,
            presentation_value_role: None,
            pixel_data: XbxEngineRenderPixelData::Rgba {
                bytes: Arc::<[u8]>::from([6u8; 16]),
            },
        },
    };

    assert!(state.enqueue_decoded_frame(non_owner).is_none());
    let dropped = state
        .enqueue_decoded_frame(owner)
        .expect("non-owner rebuilding-supply should be superseded by owner-matched candidate");

    assert_eq!(dropped.surface.frame_seq, 220);
    let kept = state.pop_decoded_frame(240.0).expect("kept frame");
    assert_eq!(kept.surface.frame_seq, 200);
    assert_eq!(kept.recovery_owner_rtp_timestamp, Some(200));
}

#[test]
fn decode_candidate_state_recovers_to_nominal_after_pressure_is_relieved() {
    let decoder = SpyHardwareDecoder;
    let mut state = XbxVideoDecodeState::new_for_test(20, 30, Box::new(decoder));

    for seq in 1..=4 {
        state.enqueue_decoded_frame_for_test(XbxRenderFrame {
            width: 2,
            height: 2,
            frame_seq: seq,
            rendered_at_ms: seq as f64,
            rtp_timestamp: Some(seq as u32),
            recovery_epoch_tag: None,
            recovery_owner_rtp_timestamp: None,
            is_keyframe: seq == 1,
            frame_recovery_disposition: Some("repairing".to_string()),
            frame_unrecoverable_reason: None,
            presentation_value_role: None,
            pixel_data: XbxEngineRenderPixelData::Rgba {
                bytes: Arc::<[u8]>::from([0u8; 16]),
            },
        });
    }
    let pressured = state
        .latest_decode_candidate_decision()
        .expect("backpressure decision");
    assert_eq!(pressured.state, XbxDecodeCandidateState::Backpressure);

    let _ = state.pop_decoded_frame(5.0);
    state.enqueue_decoded_frame_for_test(XbxRenderFrame {
        width: 2,
        height: 2,
        frame_seq: 4,
        rendered_at_ms: 4.0,
        rtp_timestamp: Some(4),
        recovery_epoch_tag: None,
        recovery_owner_rtp_timestamp: None,
        is_keyframe: false,
        frame_recovery_disposition: Some("repairing".to_string()),
        frame_unrecoverable_reason: None,
        presentation_value_role: None,
        pixel_data: XbxEngineRenderPixelData::Rgba {
            bytes: Arc::<[u8]>::from([1u8; 16]),
        },
    });
    let recovered = state
        .latest_decode_candidate_decision()
        .expect("recovered decision");
    assert_eq!(recovered.state, XbxDecodeCandidateState::Nominal);
    assert_eq!(recovered.action, "accept");
    assert_eq!(recovered.detail, "mailboxRecovered");
    assert_eq!(recovered.frame_seq, Some(4));
}

#[test]
fn ingress_demand_never_blocks_when_decode_mailbox_is_full() {
    let decoder = SpyHardwareDecoder;
    let mut state = XbxVideoDecodeState::new_for_test(20, 30, Box::new(decoder));

    state.enqueue_decoded_frame_for_test(XbxRenderFrame {
        width: 2,
        height: 2,
        frame_seq: 1,
        rendered_at_ms: 1.0,
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
    });
    let inflight = state.pop_decoded_frame(2.0).expect("inflight should exist");
    state.requeue_decoded_frame_front(inflight);
    state.enqueue_decoded_frame_for_test(XbxRenderFrame {
        width: 2,
        height: 2,
        frame_seq: 2,
        rendered_at_ms: 2.0,
        rtp_timestamp: Some(2),
        recovery_epoch_tag: None,
        recovery_owner_rtp_timestamp: None,
        is_keyframe: false,
        frame_recovery_disposition: Some("repairing".to_string()),
        frame_unrecoverable_reason: None,
        presentation_value_role: None,
        pixel_data: XbxEngineRenderPixelData::Rgba {
            bytes: Arc::<[u8]>::from([0u8; 16]),
        },
    });
    assert!(!state.ingress_demand().should_pull_output_first());

    let _ = state.pop_decoded_frame(3.0);
    assert!(!state.ingress_demand().should_pull_output_first());
}

#[test]
fn workload_snapshot_keeps_accepting_input_until_output_queue_is_full() {
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
        recovery_epoch_tag: None,
        recovery_owner_rtp_timestamp: None,
        is_keyframe: true,
        frame_recovery_disposition: Some("repairing".to_string()),
        frame_unrecoverable_reason: None,
        presentation_value_role: None,
        pixel_data: XbxEngineRenderPixelData::Rgba {
            bytes: Arc::<[u8]>::from([0u8; 16]),
        },
    });
    // 仅 enqueue 不会累积到深队列；要进入 DrainOutput，需要凑满 current+latest 两槽。
    let inflight = state.pop_decoded_frame(2.0).expect("inflight should exist");
    state.requeue_decoded_frame_front(inflight);
    state.enqueue_decoded_frame_for_test(XbxRenderFrame {
        width: 2,
        height: 2,
        frame_seq: 2,
        rendered_at_ms: 2.0,
        rtp_timestamp: Some(2),
        recovery_epoch_tag: None,
        recovery_owner_rtp_timestamp: None,
        is_keyframe: false,
        frame_recovery_disposition: Some("repairing".to_string()),
        frame_unrecoverable_reason: None,
        presentation_value_role: None,
        pixel_data: XbxEngineRenderPixelData::Rgba {
            bytes: Arc::<[u8]>::from([0u8; 16]),
        },
    });

    let full = state.workload_snapshot();
    assert_eq!(full.state, XbxDecodeWorkloadState::DrainOutput);
    assert_eq!(full.pending_output_queue_depth, 2);
    assert!(full.should_drain_output_first());

    let _ = state.pop_decoded_frame(3.0);
    let drained = state.workload_snapshot();
    assert_eq!(drained.state, XbxDecodeWorkloadState::AwaitingInput);
    assert_eq!(drained.pending_output_queue_depth, 1);
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
    ) -> Result<XbxVideoDecoderBackendDecodeOutcome, crate::XbxEngineRuntimeError> {
        self.decode_calls.fetch_add(1, Ordering::Relaxed);
        match self.scripted_results.pop_front().unwrap_or(Ok(None)) {
            Ok(frame) => Ok(XbxVideoDecoderBackendDecodeOutcome {
                frames: frame.into_iter().collect(),
                send_packet_status: None,
                receive_frame_status: None,
            }),
            Err(error) => Err(error),
        }
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
        recovery_epoch_tag: None,
        recovery_owner_rtp_timestamp: None,
        clean_anchor_commit_recovery_epoch: None,
        first_packet_sequence: None,
        frame_recovery_disposition: crate::media::video::types::FrameRecoveryDisposition::Repairing,
        frame_unrecoverable_reason: None,
        target_playout_instant: now + Duration::from_millis(16),
        h264: make_h264_inspection(is_keyframe),
        payload: Bytes::from_static(b"\x00\x00\x00\x01\x65"),
    }
}

fn make_non_idr_continuation_frame(rtp_timestamp: u32) -> EncodedFrame {
    let mut frame = make_encoded_frame(false);
    frame.rtp_timestamp = rtp_timestamp;
    frame.h264 = H264AccessUnitInspection {
        nals: vec![H264NalUnit {
            range: 0..1,
            unit_type: UnitType::SliceLayerWithoutPartitioningNonIdr,
        }],
        parameter_sets: None,
        width: Some(2560),
        height: Some(1440),
        is_idr: false,
        has_inband_sps: false,
        has_inband_pps: false,
        slice_headers_valid: true,
        parameter_sets_changed: false,
        config_changed: false,
        bootstrap_ready: false,
        bootstrap_reject_reason: Some(H264BootstrapRejectReason::NonIdrVcl),
        commit_state: H264AccessUnitInspector::test_commit_state(),
    };
    frame
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
        backend_name: "ffmpeg-software",
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
fn backend_failure_falls_back_to_software_decoder_and_updates_probe_snapshot() {
    let failing_decode_calls = Arc::new(AtomicUsize::new(0));
    let software_decode_calls = Arc::new(AtomicUsize::new(0));

    let decoder = ScriptedHardwareDecoder {
        backend_name: "ffmpeg-d3d11va",
        decode_calls: failing_decode_calls.clone(),
        scripted_results: VecDeque::from([Err(crate::XbxEngineRuntimeError::new(
            "xbxEngineCreateVideoFormatDescriptionFailed:status=-12909",
        ))]),
    };

    let software_decode_calls_for_factory = software_decode_calls.clone();
    let software_decoder_factory = Box::new(move || {
        (
            Box::new(ScriptedHardwareDecoder {
                backend_name: "ffmpeg-software",
                decode_calls: software_decode_calls_for_factory.clone(),
                scripted_results: VecDeque::from([Ok(Some(XbxRenderFrame {
                    width: 2,
                    height: 2,
                    frame_seq: 0,
                    rendered_at_ms: 0.0,
                    rtp_timestamp: None,
                    recovery_epoch_tag: None,
                    recovery_owner_rtp_timestamp: None,
                    is_keyframe: false,
                    frame_recovery_disposition: None,
                    frame_unrecoverable_reason: None,
                    presentation_value_role: None,
                    pixel_data: XbxEngineRenderPixelData::Rgba {
                        bytes: Arc::<[u8]>::from([1u8; 16]),
                    },
                }))]),
            }) as Box<dyn XbxVideoDecoderBackend>,
            XbxVideoDecoderProbeSummary {
                selected_backend_name: "ffmpeg-software".to_string(),
                selected_backend_kind: "software".to_string(),
                fallback_count: 0,
                fallback_summary: None,
            },
        )
    });
    let mut state = XbxVideoDecodeState::new_for_test_with_factories(
        20,
        30,
        Box::new(decoder),
        Box::new(|| {
            panic!("decoder reset factory should not be used in backend error soft fallback test");
        }),
        software_decoder_factory,
    );

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
        .is_some_and(|summary| summary.contains("ffmpeg-d3d11va(hardware/backend-error)")));
    let transition = state
        .latest_recovery_transition()
        .expect("recovery transition should exist");
    assert_eq!(transition.detail, "backendErrorSoftFallback");
    assert_eq!(transition.status, Some(-12909));
    assert_eq!(software_decode_calls.load(Ordering::Relaxed), 0);
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
            recovery_epoch_tag: None,
            recovery_owner_rtp_timestamp: None,
            is_keyframe: false,
            frame_recovery_disposition: None,
            frame_unrecoverable_reason: None,
            presentation_value_role: None,
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
    assert!(!outcome.overwritten_pending_frame);
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
                    recovery_epoch_tag: None,
                    recovery_owner_rtp_timestamp: None,
                    is_keyframe: false,
                    frame_recovery_disposition: None,
                    frame_unrecoverable_reason: None,
                    presentation_value_role: None,
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
                    recovery_epoch_tag: None,
                    recovery_owner_rtp_timestamp: None,
                    is_keyframe: false,
                    frame_recovery_disposition: None,
                    frame_unrecoverable_reason: None,
                    presentation_value_role: None,
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

    let _ = state.process_encoded_frame(third, 1_032.0);
    assert_eq!(state.recovery_state(), XbxVideoRecoveryState::Nominal);
    assert_eq!(replacement_decode_calls.load(Ordering::Relaxed), 2);

    // decode output mailbox：未被下游取走前只保留 value-aware latest 候选。
    let recovered = state
        .pop_decoded_frame(1_048.0)
        .expect("recovered frame should exist");
    assert_eq!(recovered.surface.frame_seq, 2);

    let mut render_state = XbxRenderState::default();
    render_state
        .present_frame(recovered.surface)
        .expect("recovered frame should render");
    assert_eq!(
        render_state
            .peek_latest_frame()
            .map(|frame| frame.frame_seq),
        Some(2)
    );
}

#[tokio::test]
async fn rtp_to_decode_to_pacer_to_renderer_pipeline_reaches_shadow_frame_and_armed_protection_signal(
) {
    let (tx, rx) = tokio::sync::mpsc::channel(64);
    let (transport_observation_tx, _transport_observation_rx) =
        tokio::sync::mpsc::unbounded_channel();
    let source_runtime_stats =
        Arc::new(std::sync::Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let mut source = crate::transport::rtc::receive::RtcVideoFrameSource::new(
        rx,
        transport_observation_tx,
        source_runtime_stats.clone(),
        16,
        Duration::from_millis(10),
        Duration::from_millis(20),
        Duration::from_millis(200),
        crate::transport::rtc::receive::test_nack_scheduler_config(),
        crate::transport::rtc::receive::test_transport_capability(),
    );
    source_runtime_stats
        .lock()
        .expect("source runtime stats lock")
        .transport_recovery_epoch = 1;
    source_runtime_stats
        .lock()
        .expect("source runtime stats lock")
        .transport_recovery_episode_active = true;
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
                recovery_epoch_tag: None,
                recovery_owner_rtp_timestamp: None,
                is_keyframe: false,
                frame_recovery_disposition: None,
                frame_unrecoverable_reason: None,
                presentation_value_role: None,
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
                recovery_epoch_tag: None,
                recovery_owner_rtp_timestamp: None,
                is_keyframe: false,
                frame_recovery_disposition: None,
                frame_unrecoverable_reason: None,
                presentation_value_role: None,
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
                recovery_epoch_tag: None,
                recovery_owner_rtp_timestamp: None,
                is_keyframe: false,
                frame_recovery_disposition: None,
                frame_unrecoverable_reason: None,
                presentation_value_role: None,
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
            std::sync::Arc::new(std::sync::Mutex::new(None)),
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
        assert_eq!(assembled.recovery_epoch_tag, Some(1));
        let encoded = assembled.into_encoded_frame(Instant::now());
        assert!(decode_state
            .process_encoded_frame(encoded, expected_timestamp as f64)
            .is_none());
        let decoded = decode_state
            .pop_decoded_frame(expected_timestamp as f64 + 1.0)
            .expect("decoded frame should be available");
        assert_eq!(decoded.recovery_epoch_tag, Some(1));
        assert_eq!(decoded.surface.recovery_epoch_tag, Some(1));

        let submit_deadline = Instant::now() + Duration::from_millis(150);
        let mut submitted = false;
        while Instant::now() < submit_deadline {
            match pacer.submit(decoded.clone()) {
                Ok(_) => {
                    submitted = true;
                    break;
                }
                Err(std::sync::mpsc::TrySendError::Full(_)) => {
                    tokio::time::sleep(Duration::from_millis(2)).await;
                }
                Err(err) => panic!("unexpected pacer submit failure: {err:?}"),
            }
        }
        assert!(submitted, "decoded frame should eventually reach pacer");
    }

    let render_deadline = Instant::now() + Duration::from_millis(500);
    let mut latest_rendered_at_ms = None;
    let mut renderer_submit_count = 0u64;
    while Instant::now() < render_deadline {
        {
            let render_state_guard = render_state.lock().expect("render state lock");
            let snapshot = render_state_guard.render_signal_snapshot(0.0);
            if let Some(rendered_at_ms) = snapshot.latest_present_time_ms {
                latest_rendered_at_ms = Some(rendered_at_ms);
            }
            // submit_count increments before present_frame completes; gate on mailbox seq instead.
            if render_state_guard
                .peek_latest_frame()
                .is_some_and(|frame| frame.frame_seq >= 2)
            {
                renderer_submit_count = runtime_stats
                    .lock()
                    .expect("runtime stats lock")
                    .video_renderer_submit_count_total;
                break;
            }
        }
        renderer_submit_count = runtime_stats
            .lock()
            .expect("runtime stats lock")
            .video_renderer_submit_count_total;
        tokio::time::sleep(Duration::from_millis(4)).await;
    }

    pacer.stop();
    renderer.stop();

    assert!(latest_rendered_at_ms.is_some());
    assert!(renderer_submit_count >= 2);
    let render_state_guard = render_state.lock().expect("render state lock");
    // Render mailbox keeps a single latest handoff slot (`RENDER_MAILBOX_CAPACITY = 1`).
    let latest_frame = render_state_guard
        .peek_latest_frame()
        .expect("latest staged frame should exist");
    assert!(
        (2..=3).contains(&latest_frame.frame_seq),
        "expected last accepted renderer frame 2 or 3 depending on pacer/renderer shutdown timing; got {}",
        latest_frame.frame_seq
    );
    assert_eq!(latest_frame.recovery_epoch_tag, Some(1));
    let stats = runtime_stats.lock().expect("runtime stats lock");
    assert!(stats.video_renderer_submit_count_total >= 2);
    let decision = stats
        .latest_render_mailbox_decision
        .clone()
        .expect("render candidate decision should exist");
    assert_eq!(decision.state, "latest-overwrite");
    assert_eq!(decision.detail, "mailboxOverwrite");
}

#[test]
fn invaliddata_on_delta_does_not_trigger_software_fallback() {
    use crate::media::video::decode::backend_ffmpeg::av_err_invaliddata;

    let invalid = av_err_invaliddata();
    let decode_calls = Arc::new(AtomicUsize::new(0));
    let mut scripted_results = VecDeque::new();
    scripted_results.push_back(Err(crate::XbxEngineRuntimeError::new(format!(
        "decode failed status={invalid}"
    ))));
    let decoder = ScriptedHardwareDecoder {
        backend_name: "scripted",
        decode_calls: decode_calls.clone(),
        scripted_results,
    };
    let decoder_factory = Box::new(|| {
        panic!("software fallback must not run for AVERROR_INVALIDDATA on delta");
    });
    let mut state =
        XbxVideoDecodeState::new_for_test_with_factory(20, 30, Box::new(decoder), decoder_factory);

    assert!(state
        .process_encoded_frame(make_encoded_frame(false), 1_000.0)
        .is_none());
    assert_eq!(state.decoder_backend_name(), "scripted");
    assert_eq!(state.hardware_decode_failure_streak(), 0);
}
