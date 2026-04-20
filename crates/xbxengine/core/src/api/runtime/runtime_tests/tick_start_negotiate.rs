use super::super::{
    XbxEngineReconnectTriggerSource, XbxEngineRuntime, XbxEngineRuntimeConfig,
    XbxEngineRuntimeState,
};
use super::fixtures::*;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use ohmygamepad_protocol::{LogicalPadId, OhMyGamepadRumbleTargetDto};
use xbxengine_protocol::{
    XbxEngineControlCommandDto, XbxEngineDisplayOptionsDto, XbxEngineDisplayStateDto,
    XbxEngineHostRequestDto, XbxEngineIceCandidateDto, XbxEngineInputEventDto,
    XbxEngineReconnectReasonDto, XbxEngineRenderProjectionDto, XbxEngineRuntimeCodecPreferenceDto,
    XbxEngineRuntimeEventDto, XbxEngineRuntimePhaseDto, XbxEngineRuntimeProjectionDto,
    XbxEngineRuntimeRecoveryDto, XbxEngineRuntimeVideoPipelineDto, XbxEngineTransportStateDto,
    XbxEngineViewportDto,
};

use crate::{
    PlaceholderXbxEngineMediaBackend, XbxEngineInputStatus, XbxEngineMediaNegotiation,
    XbxEngineMediaRuntimeStats,
};

#[test]
fn start_negotiates_remote_and_reaches_running() {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::new(RefCell::new(Vec::new()));
    let mut runtime = create_runtime(requests.clone(), events.clone());

    runtime
        .start(session(), viewport(), 0.75, None, None)
        .expect("runtime start should succeed");

    assert_eq!(runtime.state(), &XbxEngineRuntimeState::Running);
    assert_eq!(runtime.snapshot().audio_volume, 0.75);
    assert_eq!(
        runtime.snapshot().surface_id.as_deref(),
        Some("surface:viewport-1")
    );
    assert_eq!(runtime.snapshot().video_size, Some((1280, 720)));
    assert_eq!(runtime.snapshot().negotiation_attempt_count, 1);
    assert_eq!(
        runtime.snapshot().last_answer_sdp.as_deref(),
        Some("answer")
    );
    assert_eq!(
        requests.borrow().as_slice(),
        &[
            XbxEngineHostRequestDto::ExchangeOffer {
                session_id: "session-1".to_string(),
                channel: "media".to_string(),
                sdp: "v=0\r\no=session-1 initial-placeholder:1\r\n".to_string(),
                restart: false,
            },
            XbxEngineHostRequestDto::SubmitIce {
                session_id: "session-1".to_string(),
                candidates: vec![placeholder_local_candidate()],
                restart: false,
            },
            XbxEngineHostRequestDto::PollIce {
                session_id: "session-1".to_string(),
                restart: false,
            },
        ]
    );

    let phases: Vec<XbxEngineRuntimePhaseDto> = events
        .borrow()
        .iter()
        .filter_map(|event| match event {
            XbxEngineRuntimeEventDto::RuntimePhaseChanged { phase } => Some(phase.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        phases,
        vec![
            XbxEngineRuntimePhaseDto::Binding,
            XbxEngineRuntimePhaseDto::ExchangingOffer,
            XbxEngineRuntimePhaseDto::GatheringIce,
            XbxEngineRuntimePhaseDto::Connecting,
            XbxEngineRuntimePhaseDto::ExchangingIce,
        ]
    );
    let event_log = events.borrow();
    let media_surface_ready_index = event_log
        .iter()
        .position(|event| {
            matches!(
                event,
                XbxEngineRuntimeEventDto::MediaSurfaceReady { surface_id }
                if surface_id == "surface:viewport-1"
            )
        })
        .expect("media surface ready should be emitted");
    let exchanging_ice_index = event_log
        .iter()
        .position(|event| {
            matches!(
                event,
                XbxEngineRuntimeEventDto::RuntimePhaseChanged {
                    phase: XbxEngineRuntimePhaseDto::ExchangingIce,
                }
            )
        })
        .expect("exchanging ice phase should be emitted");
    assert!(
        media_surface_ready_index < exchanging_ice_index,
        "media surface ready should no longer wait for full ICE exchange"
    );
    assert!(event_log.iter().any(|event| matches!(
        event,
        XbxEngineRuntimeEventDto::TransportConnectionStateChanged {
            state: XbxEngineTransportStateDto::Connected,
        }
    )));
    assert!(event_log.iter().any(|event| matches!(
        event,
        XbxEngineRuntimeEventDto::MediaSurfaceReady { surface_id }
        if surface_id == "surface:viewport-1"
    )));
}

#[test]
fn runtime_tick_presents_frame_before_snapshotting_runtime_stats() {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::new(RefCell::new(Vec::new()));
    let call_order = Arc::new(Mutex::new(Vec::new()));
    let rendered_at_ms = 1_000.0;
    let frame = render_frame(7, rendered_at_ms);
    let backend = ScriptedMediaBackend::new(
        XbxEngineMediaNegotiation {
            local_offer_sdp: "offer".to_string(),
            local_candidates: Vec::new(),
            surface_id: "surface:viewport-1".to_string(),
            video_width: 1280,
            video_height: 720,
            first_frame_packet_arrival_time_ms: None,
            frame_decoded_time_ms: None,
            frame_rendered_time_ms: None,
            input_status: XbxEngineInputStatus::default(),
        },
        XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            latest_video_host_present_time_ms: Some(rendered_at_ms - 16.0),
            latest_video_decode_ok_time_ms: Some(rendered_at_ms - 12.0),
            latest_video_track_status: Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1280),
                video_height: Some(720),
                mime_type: Some("video/h264".to_string()),
                transport_state: XbxEngineTransportStateDto::Connected,
                video_bytes_total: 1,
                video_packet_count_total: 1,
                audio_bytes_total: 0,
                observed_at_ms: rendered_at_ms - 16.0,
            }),
            host_no_pending_pressure_level: Some("normal".to_string()),
            host_no_pending_streak: 0,
            video_present_submit_count_total: 1,
            ..Default::default()
        },
    )
    .with_call_order(call_order.clone())
    .with_latest_render_frame(frame.clone());
    let mut runtime = XbxEngineRuntime::with_media_backend(
        XbxEngineRuntimeConfig::default(),
        TestHostBridge::new(requests).with_call_order(call_order.clone()),
        TestEventSink::new(events),
        backend,
    );

    runtime
        .start(session(), viewport(), 1.0, None, None)
        .expect("runtime start should succeed");
    call_order.lock().expect("lock call order").clear();

    runtime.tick();

    assert_eq!(
        call_order.lock().expect("lock call order").as_slice(),
        &["take_frame", "present", "ack", "snapshot"]
    );
    assert_eq!(
        runtime.snapshot().frame_rendered_time_ms,
        Some(rendered_at_ms)
    );
}

#[test]
fn runtime_tick_prioritizes_present_before_budgeted_rumble_work() {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::new(RefCell::new(Vec::new()));
    let call_order = Arc::new(Mutex::new(Vec::new()));
    let rendered_at_ms = 1_000.0;
    let backend = ScriptedMediaBackend::new(
        XbxEngineMediaNegotiation {
            local_offer_sdp: "offer".to_string(),
            local_candidates: Vec::new(),
            surface_id: "surface:viewport-1".to_string(),
            video_width: 1280,
            video_height: 720,
            first_frame_packet_arrival_time_ms: None,
            frame_decoded_time_ms: None,
            frame_rendered_time_ms: None,
            input_status: XbxEngineInputStatus::default(),
        },
        XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            latest_video_host_present_time_ms: Some(rendered_at_ms - 16.0),
            latest_video_decode_ok_time_ms: Some(rendered_at_ms - 12.0),
            host_no_pending_pressure_level: Some("normal".to_string()),
            host_no_pending_streak: 0,
            video_present_submit_count_total: 1,
            ..Default::default()
        },
    )
    .with_call_order(call_order.clone())
    .with_latest_render_frame(render_frame(7, rendered_at_ms))
    .with_pending_gamepad_rumble_requests(vec![
        test_rumble_request(
            OhMyGamepadRumbleTargetDto::LogicalPad {
                pad_id: LogicalPadId::Pad0,
            },
            0.8,
        ),
        test_rumble_request(
            OhMyGamepadRumbleTargetDto::LogicalPad {
                pad_id: LogicalPadId::Pad1,
            },
            0.6,
        ),
    ]);
    let host_bridge = TestHostBridge::new(requests).with_call_order(call_order.clone());
    let rumble_requests = host_bridge.rumble_requests.clone();
    let mut runtime = XbxEngineRuntime::with_media_backend(
        XbxEngineRuntimeConfig::default(),
        host_bridge,
        TestEventSink::new(events),
        backend,
    );

    runtime
        .start(session(), viewport(), 1.0, None, None)
        .expect("runtime start should succeed");
    call_order.lock().expect("lock call order").clear();

    runtime.tick();

    assert_eq!(
        call_order.lock().expect("lock call order").as_slice(),
        &[
            "take_frame",
            "present",
            "ack",
            "snapshot",
            "rumble_submit",
            "rumble_submit",
        ]
    );
    assert_eq!(
        rumble_requests.lock().expect("lock rumble requests").len(),
        2
    );
}

#[test]
fn runtime_tick_submits_backend_rumble_requests_without_runtime_backlog() {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::new(RefCell::new(Vec::new()));
    let backend = ScriptedMediaBackend::new(
        XbxEngineMediaNegotiation {
            local_offer_sdp: "offer".to_string(),
            local_candidates: Vec::new(),
            surface_id: "surface:viewport-1".to_string(),
            video_width: 1280,
            video_height: 720,
            first_frame_packet_arrival_time_ms: None,
            frame_decoded_time_ms: None,
            frame_rendered_time_ms: None,
            input_status: XbxEngineInputStatus::default(),
        },
        XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            ..Default::default()
        },
    )
    .with_pending_gamepad_rumble_requests(vec![
        test_rumble_request(
            OhMyGamepadRumbleTargetDto::LogicalPad {
                pad_id: LogicalPadId::Pad0,
            },
            0.2,
        ),
        test_rumble_request(
            OhMyGamepadRumbleTargetDto::LogicalPad {
                pad_id: LogicalPadId::Pad0,
            },
            0.9,
        ),
        test_rumble_request(
            OhMyGamepadRumbleTargetDto::LogicalPad {
                pad_id: LogicalPadId::Pad1,
            },
            0.4,
        ),
    ]);
    let host_bridge = TestHostBridge::new(requests);
    let rumble_requests = host_bridge.rumble_requests.clone();
    let mut runtime = XbxEngineRuntime::with_media_backend(
        XbxEngineRuntimeConfig::default(),
        host_bridge,
        TestEventSink::new(events),
        backend,
    );

    runtime
        .start(session(), viewport(), 1.0, None, None)
        .expect("runtime start should succeed");

    runtime.tick();

    let rumble_requests = rumble_requests.lock().expect("lock rumble requests");
    assert_eq!(rumble_requests.len(), 3);
    assert_eq!(
        rumble_requests[0].target,
        OhMyGamepadRumbleTargetDto::LogicalPad {
            pad_id: LogicalPadId::Pad0,
        }
    );
    assert_eq!(rumble_requests[0].effect.strong_magnitude, 0.2);
    assert_eq!(
        rumble_requests[1].target,
        OhMyGamepadRumbleTargetDto::LogicalPad {
            pad_id: LogicalPadId::Pad0,
        }
    );
    assert_eq!(rumble_requests[1].effect.strong_magnitude, 0.9);
    assert_eq!(
        rumble_requests[2].target,
        OhMyGamepadRumbleTargetDto::LogicalPad {
            pad_id: LogicalPadId::Pad1,
        }
    );
}

#[test]
fn start_submits_offer_sdp_ice_without_waiting_for_gathering() {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::new(RefCell::new(Vec::new()));
    let backend = ScriptedMediaBackend::new(
        XbxEngineMediaNegotiation {
            local_offer_sdp: concat!(
                "v=0\r\n",
                "m=audio 9 UDP/TLS/RTP/SAVPF 111\r\n",
                "a=mid:0\r\n",
                "a=candidate:1 1 udp 2130706431 10.0.0.20 50000 typ host\r\n",
            )
            .to_string(),
            local_candidates: Vec::new(),
            surface_id: "surface:viewport-1".to_string(),
            video_width: 1280,
            video_height: 720,
            first_frame_packet_arrival_time_ms: Some(1.0),
            frame_decoded_time_ms: Some(2.0),
            frame_rendered_time_ms: Some(3.0),
            input_status: XbxEngineInputStatus::default(),
        },
        XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            ..Default::default()
        },
    )
    .with_local_ice_gathering_complete_true_after_calls(1);
    let mut runtime = XbxEngineRuntime::with_media_backend(
        XbxEngineRuntimeConfig::default(),
        TestHostBridge::new(requests.clone()),
        TestEventSink::new(events),
        backend,
    );

    runtime
        .start(session(), viewport(), 1.0, None, None)
        .expect("runtime start should succeed");

    assert_eq!(runtime.state(), &XbxEngineRuntimeState::Running);
    let request_log = requests.borrow();
    assert!(matches!(
        request_log.first(),
        Some(XbxEngineHostRequestDto::ExchangeOffer { .. })
    ));
    assert!(matches!(
        request_log.last(),
        Some(XbxEngineHostRequestDto::PollIce { .. })
    ));
    let submit_candidates = request_log
        .iter()
        .find_map(|request| match request {
            XbxEngineHostRequestDto::SubmitIce { candidates, .. } => Some(candidates),
            _ => None,
        })
        .expect("submit ice request should exist");
    assert!(
        !submit_candidates.is_empty(),
        "submit ice request should include candidates"
    );
    assert_eq!(
        submit_candidates[0],
        XbxEngineIceCandidateDto {
            candidate: "a=candidate:1 1 udp 2130706431 10.0.0.20 50000 typ host".to_string(),
            sdp_m_line_index: Some(0),
            sdp_mid: Some("0".to_string()),
        },
        "offer SDP candidate should be submitted with highest priority"
    );
}

#[test]
fn start_submits_offer_sdp_ice_even_if_gathering_completes_immediately() {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::new(RefCell::new(Vec::new()));
    let backend = ScriptedMediaBackend::new(
        XbxEngineMediaNegotiation {
            local_offer_sdp: concat!(
                "v=0\r\n",
                "m=audio 9 UDP/TLS/RTP/SAVPF 111\r\n",
                "a=mid:0\r\n",
                "a=candidate:1 1 udp 2130706431 10.0.0.20 50000 typ host\r\n",
            )
            .to_string(),
            local_candidates: Vec::new(),
            surface_id: "surface:viewport-1".to_string(),
            video_width: 1280,
            video_height: 720,
            first_frame_packet_arrival_time_ms: Some(1.0),
            frame_decoded_time_ms: Some(2.0),
            frame_rendered_time_ms: Some(3.0),
            input_status: XbxEngineInputStatus::default(),
        },
        XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            ..Default::default()
        },
    )
    .with_local_ice_gathering_complete_true_after_calls(0);
    let mut runtime = XbxEngineRuntime::with_media_backend(
        XbxEngineRuntimeConfig::default(),
        TestHostBridge::new(requests.clone()),
        TestEventSink::new(events),
        backend,
    );

    runtime
        .start(session(), viewport(), 1.0, None, None)
        .expect("runtime start should succeed");

    let request_log = requests.borrow();
    assert!(request_log
        .iter()
        .any(|request| matches!(request, XbxEngineHostRequestDto::SubmitIce { .. })));
    assert!(request_log
        .iter()
        .any(|request| matches!(request, XbxEngineHostRequestDto::PollIce { .. })));
}

#[test]
fn start_exits_ice_exchange_when_gathering_complete_and_remote_eoc_seen() {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::new(RefCell::new(Vec::new()));
    let backend = ScriptedMediaBackend::new(
        XbxEngineMediaNegotiation {
            local_offer_sdp: concat!(
                "v=0\r\n",
                "m=audio 9 UDP/TLS/RTP/SAVPF 111\r\n",
                "a=mid:0\r\n",
                "a=candidate:1 1 udp 2130706431 10.0.0.20 50000 typ host\r\n",
            )
            .to_string(),
            local_candidates: Vec::new(),
            surface_id: "surface:viewport-1".to_string(),
            video_width: 1280,
            video_height: 720,
            first_frame_packet_arrival_time_ms: Some(1.0),
            frame_decoded_time_ms: Some(2.0),
            frame_rendered_time_ms: Some(3.0),
            input_status: XbxEngineInputStatus::default(),
        },
        XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connecting,
            ..Default::default()
        },
    )
    .with_local_ice_gathering_complete_true_after_calls(0);
    let host_bridge = TestHostBridge::new(requests.clone())
        .with_poll_ice_batches(vec![vec![remote_end_of_candidates_marker()]]);
    let mut runtime = XbxEngineRuntime::with_media_backend(
        XbxEngineRuntimeConfig::default(),
        host_bridge,
        TestEventSink::new(events),
        backend,
    );

    runtime
        .start(session(), viewport(), 1.0, None, None)
        .expect("runtime start should succeed");

    assert_eq!(runtime.state(), &XbxEngineRuntimeState::Running);
    assert_eq!(
        runtime.snapshot().last_remote_candidates,
        vec![remote_end_of_candidates_marker()]
    );
    let poll_count = requests
        .borrow()
        .iter()
        .filter(|request| matches!(request, XbxEngineHostRequestDto::PollIce { .. }))
        .count();
    assert_eq!(poll_count, 1);
}

#[test]
fn start_runtime_control_consumes_execution_spec() {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::new(RefCell::new(Vec::new()));
    let mut runtime = create_runtime(requests, events);

    runtime
        .apply_control(XbxEngineControlCommandDto::StartRuntime {
            session: session(),
            viewport: viewport(),
            audio_volume: 0.6,
            runtime: Some(XbxEngineRuntimeProjectionDto {
                codec: Some(XbxEngineRuntimeCodecPreferenceDto {
                    mime_type: "video/H264".to_string(),
                    profiles: vec!["42e01f".to_string()],
                }),
                max_video_bitrate_kbps: Some(42_000),
                max_audio_bitrate_kbps: Some(192),
                target_video_width: 1920,
                target_video_height: 1080,
                force_mono_audio: false,
                prefer_ipv6: true,
                bwe_mode: "hybrid".to_string(),
                forced_remb_kbps: Some(24_000),
                adaptive_remb_enabled: false,
                remb_floor_kbps: 12_000,
                remb_ceiling_kbps: 60_000,
                remb_ramp_up_step_kbps: 3_000,
                remb_ramp_down_factor: 750,
                video_pipeline: XbxEngineRuntimeVideoPipelineDto {
                    feedback_interval_ms: 333,
                    nack_window_ms: 321,
                    nack_max_age_ms: 123,
                    nack_retry_interval_ms: 45,
                    nack_burst_count: 8,
                    nack_max_retry_count: 6,
                    jitter_buffer_min_delay_ms: 12,
                    jitter_buffer_max_delay_ms: 34,
                    jitter_buffer_max_packets: 789,
                    idle_timeout_ms: 222,
                    late_frame_drop_threshold_ms: 444,
                    backlog_drop_threshold_packets: 11,
                    jitter_early_emit_enabled: false,
                },
                recovery: XbxEngineRuntimeRecoveryDto {
                    first_frame_grace_ms: 7_000,
                    keyframe_request_stall_ms: 1_100,
                    keyframe_loss_burst_threshold: 4,
                    decoder_reset_after_keyframe_wait_ms: 333,
                    decoder_reset_request_cooldown_ms: 1_234,
                    reconnect_stall_ms: 3_210,
                    stall_recovery_cooldown_ms: 4_321,
                },
                polling_rate_hz: 120,
                vibration: true,
            }),
            render: Some(XbxEngineRenderProjectionDto {
                enable_audio_control: true,
                video_format: Some("nv12".to_string()),
                display_options: XbxEngineDisplayOptionsDto {
                    sharpness: 1.1,
                    saturation: 1.2,
                    contrast: 1.3,
                    brightness: 1.4,
                },
            }),
            ice_candidate_policy: None,
        })
        .expect("start runtime control should succeed");

    assert_eq!(runtime.state(), &XbxEngineRuntimeState::Running);
    assert_eq!(runtime.config.webrtc.negotiation.video_bitrate_kbps, 42_000);
    assert_eq!(runtime.config.webrtc.negotiation.audio_bitrate_kbps, 192);
    assert!(runtime.config.webrtc.negotiation.prefer_ipv6);
    assert_eq!(runtime.config.webrtc.negotiation.offer_profile, "42e01f");
    assert_eq!(runtime.config.webrtc.bwe_mode, "hybrid");
    assert_eq!(runtime.config.webrtc.forced_remb_kbps, Some(24_000));
    assert_eq!(runtime.config.webrtc.remb_floor_kbps, 12_000);
    assert_eq!(runtime.config.webrtc.remb_ceiling_kbps, 60_000);
    assert_eq!(
        runtime.config.webrtc.video_pipeline.feedback_interval_ms,
        333
    );
    assert_eq!(runtime.config.webrtc.video_pipeline.nack_max_age_ms, 123);
    assert_eq!(runtime.config.webrtc.video_pipeline.idle_timeout_ms, 222);
    assert_eq!(
        runtime
            .config
            .webrtc
            .video_pipeline
            .late_frame_drop_threshold_ms,
        444
    );
    assert_eq!(
        runtime.config.webrtc.recovery.keyframe_loss_burst_threshold,
        4
    );
    assert_eq!(runtime.config.webrtc.recovery.reconnect_stall_ms, 3_210);
    assert_eq!(
        runtime.snapshot().display_state,
        Some(XbxEngineDisplayStateDto {
            display_options: XbxEngineDisplayOptionsDto {
                sharpness: 1.1,
                saturation: 1.2,
                contrast: 1.3,
                brightness: 1.4,
            },
        })
    );
}

#[test]
fn video_track_status_changes_are_emitted_and_snapshotted() {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::new(RefCell::new(Vec::new()));
    let backend = ScriptedMediaBackend::new(
        XbxEngineMediaNegotiation {
            local_offer_sdp: "offer".to_string(),
            local_candidates: Vec::new(),
            surface_id: "surface:viewport-1".to_string(),
            video_width: 2560,
            video_height: 1440,
            first_frame_packet_arrival_time_ms: Some(1.0),
            frame_decoded_time_ms: Some(2.0),
            frame_rendered_time_ms: Some(3.0),
            input_status: XbxEngineInputStatus::default(),
        },
        XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            latest_video_track_status: Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: None,
                video_height: None,
                mime_type: Some("video/h264".to_string()),
                transport_state: XbxEngineTransportStateDto::Connected,
                video_bytes_total: 0,
                video_packet_count_total: 0,
                audio_bytes_total: 0,
                observed_at_ms: 100.0,
            }),
            ..Default::default()
        },
    );
    let runtime_stats = backend.runtime_stats.clone();
    let mut runtime = XbxEngineRuntime::with_media_backend(
        XbxEngineRuntimeConfig::default(),
        TestHostBridge::new(requests),
        TestEventSink::new(events.clone()),
        backend,
    );

    runtime
        .start(session(), viewport(), 1.0, None, None)
        .expect("runtime start should succeed");

    overwrite_runtime_stats(
        &runtime_stats,
        XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connecting,
            latest_video_track_status: Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: None,
                video_height: None,
                mime_type: Some("video/h264".to_string()),
                transport_state: XbxEngineTransportStateDto::Connected,
                video_bytes_total: 0,
                video_packet_count_total: 0,
                audio_bytes_total: 0,
                observed_at_ms: 101.0,
            }),
            ..Default::default()
        },
    );

    runtime.tick();

    assert!(matches!(
        runtime
            .snapshot()
            .latest_video_track_status
            .as_ref()
            .map(|status| status.state.as_str()),
        Some("remoteTrackAttached")
    ));
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        XbxEngineRuntimeEventDto::MediaVideoTrackStatusChanged { status }
        if status.state == "remoteTrackAttached"
    )));
}

#[test]
fn reconnect_keeps_remote_session_alive_before_restart_negotiation() {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::new(RefCell::new(Vec::new()));
    let mut runtime = create_runtime(requests.clone(), events.clone());

    runtime
        .start(session(), viewport(), 1.0, None, None)
        .expect("runtime start should succeed");
    requests.borrow_mut().clear();
    events.borrow_mut().clear();

    runtime
        .request_reconnect(
            XbxEngineReconnectReasonDto::MediaStalled,
            XbxEngineReconnectTriggerSource::Policy,
        )
        .expect("runtime reconnect should succeed");

    assert_eq!(runtime.snapshot().negotiation_attempt_count, 2);
    assert_eq!(
        requests.borrow().as_slice(),
        &[
            XbxEngineHostRequestDto::KeepAliveRemoteSession {
                session_id: "session-1".to_string(),
            },
            XbxEngineHostRequestDto::ExchangeOffer {
                session_id: "session-1".to_string(),
                channel: "media".to_string(),
                sdp: "v=0\r\no=session-1 restart-placeholder:2\r\n".to_string(),
                restart: true,
            },
            XbxEngineHostRequestDto::SubmitIce {
                session_id: "session-1".to_string(),
                candidates: vec![placeholder_local_candidate()],
                restart: true,
            },
            XbxEngineHostRequestDto::PollIce {
                session_id: "session-1".to_string(),
                restart: true,
            },
        ]
    );
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        XbxEngineRuntimeEventDto::RuntimePhaseChanged {
            phase: XbxEngineRuntimePhaseDto::Reconnecting,
        }
    )));
}

#[test]
fn reconnect_settled_keyframe_is_deferred_when_keyframe_is_already_in_flight() {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::new(RefCell::new(Vec::new()));
    let backend = ScriptedMediaBackend::new(
        XbxEngineMediaNegotiation {
            local_offer_sdp: "offer".to_string(),
            local_candidates: Vec::new(),
            surface_id: "surface:viewport-1".to_string(),
            video_width: 1280,
            video_height: 720,
            first_frame_packet_arrival_time_ms: None,
            frame_decoded_time_ms: None,
            frame_rendered_time_ms: None,
            input_status: XbxEngineInputStatus::default(),
        },
        XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            ..Default::default()
        },
    );
    let keyframe_calls = backend.keyframe_request_calls.clone();
    let mut runtime = XbxEngineRuntime::with_media_backend(
        XbxEngineRuntimeConfig::default(),
        TestHostBridge::new(requests).without_default_remote_end_of_candidates(),
        TestEventSink::new(events),
        backend,
    );
    runtime
        .start(session(), viewport(), 1.0, None, None)
        .expect("runtime start should succeed");
    runtime.snapshot.last_recovery_action = Some("requestKeyframe".to_string());
    runtime.snapshot.last_recovery_action_at_ms = Some(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis() as f64)
            .unwrap_or(0.0),
    );

    runtime
        .request_reconnect(
            XbxEngineReconnectReasonDto::MediaStalled,
            XbxEngineReconnectTriggerSource::Policy,
        )
        .expect("runtime reconnect should succeed");

    assert_eq!(*keyframe_calls.lock().expect("lock keyframe calls"), 0);
    assert_eq!(
        runtime.snapshot().last_recovery_reason.as_deref(),
        Some("reconnectSettled:keyframeDeferred:keyframeInFlight")
    );
}

#[test]
fn reconnect_settled_keyframe_is_deferred_during_cooldown_window() {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::new(RefCell::new(Vec::new()));
    let backend = ScriptedMediaBackend::new(
        XbxEngineMediaNegotiation {
            local_offer_sdp: "offer".to_string(),
            local_candidates: Vec::new(),
            surface_id: "surface:viewport-1".to_string(),
            video_width: 1280,
            video_height: 720,
            first_frame_packet_arrival_time_ms: None,
            frame_decoded_time_ms: None,
            frame_rendered_time_ms: None,
            input_status: XbxEngineInputStatus::default(),
        },
        XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            ..Default::default()
        },
    );
    let keyframe_calls = backend.keyframe_request_calls.clone();
    let mut runtime = XbxEngineRuntime::with_media_backend(
        XbxEngineRuntimeConfig::default(),
        TestHostBridge::new(requests).without_default_remote_end_of_candidates(),
        TestEventSink::new(events),
        backend,
    );
    runtime
        .start(session(), viewport(), 1.0, None, None)
        .expect("runtime start should succeed");
    runtime.health.last_keyframe_request_at_ms = Some(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis() as f64)
            .unwrap_or(0.0),
    );

    runtime
        .request_reconnect(
            XbxEngineReconnectReasonDto::MediaStalled,
            XbxEngineReconnectTriggerSource::Policy,
        )
        .expect("runtime reconnect should succeed");

    assert_eq!(*keyframe_calls.lock().expect("lock keyframe calls"), 0);
    assert_eq!(
        runtime.snapshot().last_recovery_reason.as_deref(),
        Some("reconnectSettled:keyframeDeferred:cooldown")
    );
}

#[test]
fn control_commands_update_local_runtime_snapshot() {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::new(RefCell::new(Vec::new()));
    let mut runtime = create_runtime(requests, events);

    runtime
        .apply_control(XbxEngineControlCommandDto::AttachViewport {
            viewport: viewport(),
        })
        .expect("viewport attach should succeed");
    runtime
        .apply_control(XbxEngineControlCommandDto::SetAudioVolume { value: 0.42 })
        .expect("volume update should succeed");
    runtime
        .apply_control(XbxEngineControlCommandDto::SetKeyboardPointerEnabled { enabled: true })
        .expect("keyboard pointer enable should succeed");
    runtime
        .apply_control(XbxEngineControlCommandDto::ApplyDisplayState {
            state: XbxEngineDisplayStateDto {
                display_options: XbxEngineDisplayOptionsDto {
                    sharpness: 1.0,
                    saturation: 1.1,
                    contrast: 1.2,
                    brightness: 1.3,
                },
            },
        })
        .expect("display state update should succeed");
    runtime
        .apply_control(XbxEngineControlCommandDto::PushKeyboardPointerInput {
            event: XbxEngineInputEventDto::Keyboard {
                at_ms: 1,
                event: "down".to_string(),
                code: "KeyK".to_string(),
                key: "k".to_string(),
                repeat: false,
                ctrl_key: false,
                shift_key: false,
                alt_key: false,
                meta_key: false,
            },
        })
        .expect("input forwarding should succeed");
    runtime
        .apply_control(XbxEngineControlCommandDto::PressControllerButton {
            button: "nexus".to_string(),
            duration_ms: 120,
        })
        .expect("controller button forwarding should succeed");

    assert_eq!(runtime.snapshot().audio_volume, 0.42);
    assert!(runtime.snapshot().keyboard_pointer_enabled);
    assert_eq!(
        runtime.snapshot().viewport,
        Some(XbxEngineViewportDto {
            viewport_id: "viewport-1".to_string(),
        })
    );
    assert_eq!(
        runtime.snapshot().last_pressed_controller_button,
        Some(("nexus".to_string(), 120))
    );
    assert!(matches!(
        runtime.snapshot().last_keyboard_pointer_event,
        Some(XbxEngineInputEventDto::Keyboard { .. })
    ));
}

#[test]
fn microphone_toggle_renegotiates_chat_offer() {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::new(RefCell::new(Vec::new()));
    let mut runtime = create_runtime(requests.clone(), events.clone());
    runtime
        .start(session(), viewport(), 1.0, None, None)
        .expect("runtime start should succeed");
    requests.borrow_mut().clear();
    events.borrow_mut().clear();

    runtime
        .apply_control(XbxEngineControlCommandDto::StartMicrophone)
        .expect("microphone start should succeed");

    assert!(runtime.snapshot().microphone_capturing);
    assert!(requests.borrow().iter().any(|request| matches!(
        request,
        XbxEngineHostRequestDto::ExchangeOffer { channel, restart, .. }
        if channel == "chat" && !restart
    )));
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        XbxEngineRuntimeEventDto::ChatStateChanged {
            capturing: true,
            paused: false,
        }
    )));

    requests.borrow_mut().clear();
    events.borrow_mut().clear();
    runtime
        .apply_control(XbxEngineControlCommandDto::StopMicrophone)
        .expect("microphone stop should succeed");

    assert!(!runtime.snapshot().microphone_capturing);
    assert!(requests.borrow().iter().any(|request| matches!(
        request,
        XbxEngineHostRequestDto::ExchangeOffer { channel, restart, .. }
        if channel == "chat" && !restart
    )));
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        XbxEngineRuntimeEventDto::ChatStateChanged {
            capturing: false,
            paused: true,
        }
    )));
}

#[test]
fn default_media_backend_is_swappable_but_preserves_runtime_contract() {
    let backend = PlaceholderXbxEngineMediaBackend::default();
    let requests = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::new(RefCell::new(Vec::new()));
    let mut runtime = XbxEngineRuntime::with_media_backend(
        XbxEngineRuntimeConfig::default(),
        TestHostBridge::new(requests).without_default_remote_end_of_candidates(),
        TestEventSink::new(events),
        backend,
    );

    runtime
        .start(session(), viewport(), 0.3, None, None)
        .expect("runtime start should succeed");
    runtime
        .apply_control(XbxEngineControlCommandDto::StopMicrophone)
        .expect("microphone stop should succeed");

    assert_eq!(
        runtime.snapshot().last_answer_sdp.as_deref(),
        Some("answer")
    );
    assert_eq!(runtime.snapshot().last_remote_candidates, Vec::new());
    assert_eq!(runtime.snapshot().audio_volume, 0.3);
    assert!(!runtime.snapshot().microphone_capturing);
}

#[test]
fn runtime_syncs_input_status_from_media_backend() {
    let backend =
        PlaceholderXbxEngineMediaBackend::with_input_backend(Box::new(TestInputBackend::default()));
    let requests = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::new(RefCell::new(Vec::new()));
    let mut runtime = XbxEngineRuntime::with_media_backend(
        XbxEngineRuntimeConfig::default(),
        TestHostBridge::new(requests),
        TestEventSink::new(events),
        backend,
    );

    runtime
        .start(session(), viewport(), 1.0, None, None)
        .expect("runtime start should succeed");
    runtime
        .apply_control(XbxEngineControlCommandDto::PressControllerButton {
            button: "nexus".to_string(),
            duration_ms: 120,
        })
        .expect("button press should succeed");

    assert_eq!(runtime.snapshot().input_device_count, 2);
    assert_eq!(runtime.snapshot().input_pad_count, 1);
    assert!(runtime.snapshot().input_route_attached);
}

#[test]
fn start_failure_rolls_back_to_previous_stable_state() {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::new(RefCell::new(Vec::new()));
    let fail_request_kind = Rc::new(RefCell::new(Some("ExchangeOffer")));
    let backend = ScriptedMediaBackend::new(
        XbxEngineMediaNegotiation {
            local_offer_sdp: "offer".to_string(),
            local_candidates: Vec::new(),
            surface_id: "surface:viewport-1".to_string(),
            video_width: 1280,
            video_height: 720,
            first_frame_packet_arrival_time_ms: Some(1.0),
            frame_decoded_time_ms: Some(2.0),
            frame_rendered_time_ms: Some(3.0),
            input_status: XbxEngineInputStatus {
                device_count: 1,
                pad_count: 1,
                route_attached: true,
            },
        },
        XbxEngineMediaRuntimeStats::default(),
    );
    let stop_calls = backend.stop_calls.clone();
    let mut runtime = XbxEngineRuntime::with_media_backend(
        XbxEngineRuntimeConfig::default(),
        TestHostBridge::with_failures(requests.clone(), fail_request_kind),
        TestEventSink::new(events),
        backend,
    );

    let error = runtime
        .start(session(), viewport(), 0.75, None, None)
        .expect_err("runtime start should fail");

    assert_eq!(error.to_string(), "hostBridgeFailure:ExchangeOffer");
    assert_eq!(runtime.state(), &XbxEngineRuntimeState::Idle);
    assert_eq!(runtime.snapshot().viewport, None);
    assert_eq!(runtime.snapshot().surface_id, None);
    assert_eq!(runtime.snapshot().negotiation_attempt_count, 0);
    assert_eq!(*stop_calls.lock().expect("lock stop calls"), 1);
    assert_eq!(
        requests.borrow().as_slice(),
        &[XbxEngineHostRequestDto::ExchangeOffer {
            session_id: "session-1".to_string(),
            channel: "media".to_string(),
            sdp: "offer".to_string(),
            restart: false,
        }]
    );
}

#[test]
fn reconnect_failure_restores_running_state() {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::new(RefCell::new(Vec::new()));
    let fail_request_kind = Rc::new(RefCell::new(None));
    let mut runtime = XbxEngineRuntime::with_media_backend(
        XbxEngineRuntimeConfig::default(),
        TestHostBridge::with_failures(requests.clone(), fail_request_kind.clone()),
        TestEventSink::new(events),
        PlaceholderXbxEngineMediaBackend::default(),
    );

    runtime
        .start(session(), viewport(), 1.0, None, None)
        .expect("runtime start should succeed");
    *fail_request_kind.borrow_mut() = Some("KeepAliveRemoteSession");

    let error = runtime
        .request_reconnect(
            XbxEngineReconnectReasonDto::MediaStalled,
            XbxEngineReconnectTriggerSource::Policy,
        )
        .expect_err("runtime reconnect should fail");

    assert_eq!(
        error.to_string(),
        "hostBridgeFailure:KeepAliveRemoteSession"
    );
    assert_eq!(runtime.state(), &XbxEngineRuntimeState::Running);
    assert_eq!(runtime.snapshot().negotiation_attempt_count, 1);
    assert_eq!(
        runtime.snapshot().last_answer_sdp.as_deref(),
        Some("answer")
    );
}

#[test]
fn reconnect_cancellation_stops_before_restart_offer() {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::new(RefCell::new(Vec::new()));
    let bridge = TestHostBridge::new(requests.clone());
    bridge
        .cancel_after_request_kind
        .borrow_mut()
        .replace("KeepAliveRemoteSession");
    let mut runtime = XbxEngineRuntime::with_media_backend(
        XbxEngineRuntimeConfig::default(),
        bridge,
        TestEventSink::new(events),
        PlaceholderXbxEngineMediaBackend::default(),
    );

    runtime
        .start(session(), viewport(), 1.0, None, None)
        .expect("runtime start should succeed");
    requests.borrow_mut().clear();

    let error = runtime
        .request_reconnect(
            XbxEngineReconnectReasonDto::MediaStalled,
            XbxEngineReconnectTriggerSource::Policy,
        )
        .expect_err("runtime reconnect should be cancelled");

    assert!(error.is_cancelled());
    assert_eq!(
        requests.borrow().as_slice(),
        &[XbxEngineHostRequestDto::KeepAliveRemoteSession {
            session_id: "session-1".to_string(),
        }]
    );
    assert_eq!(runtime.state(), &XbxEngineRuntimeState::Running);
}

#[test]
fn reconnect_restores_chat_negotiation_when_microphone_is_on() {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::new(RefCell::new(Vec::new()));
    let mut runtime = create_runtime(requests.clone(), events);

    runtime
        .start(session(), viewport(), 1.0, None, None)
        .expect("runtime start should succeed");
    runtime
        .apply_control(XbxEngineControlCommandDto::StartMicrophone)
        .expect("microphone start should succeed");
    requests.borrow_mut().clear();

    runtime
        .request_reconnect(
            XbxEngineReconnectReasonDto::MediaStalled,
            XbxEngineReconnectTriggerSource::Policy,
        )
        .expect("runtime reconnect should succeed");

    let request_list = requests.borrow();
    assert!(request_list.iter().any(|request| matches!(
        request,
        XbxEngineHostRequestDto::ExchangeOffer { channel, restart, .. }
        if channel == "media" && *restart
    )));
    assert!(request_list.iter().any(|request| matches!(
        request,
        XbxEngineHostRequestDto::ExchangeOffer { channel, restart, .. }
        if channel == "chat" && !restart
    )));
    assert_eq!(runtime.state(), &XbxEngineRuntimeState::Running);
}

#[test]
fn reconnect_restores_microphone_capture_when_microphone_is_on() {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::new(RefCell::new(Vec::new()));
    let backend = ScriptedMediaBackend::new(
        XbxEngineMediaNegotiation {
            local_offer_sdp: "offer".to_string(),
            local_candidates: Vec::new(),
            surface_id: "surface:viewport-1".to_string(),
            video_width: 1280,
            video_height: 720,
            first_frame_packet_arrival_time_ms: None,
            frame_decoded_time_ms: None,
            frame_rendered_time_ms: None,
            input_status: XbxEngineInputStatus::default(),
        },
        XbxEngineMediaRuntimeStats::default(),
    );
    let microphone_calls = backend.microphone_capturing_calls.clone();
    let mut runtime = XbxEngineRuntime::with_media_backend(
        XbxEngineRuntimeConfig::default(),
        TestHostBridge::new(requests.clone()),
        TestEventSink::new(events),
        backend,
    );

    runtime
        .start(session(), viewport(), 1.0, None, None)
        .expect("runtime start should succeed");
    runtime
        .apply_control(XbxEngineControlCommandDto::StartMicrophone)
        .expect("microphone start should succeed");
    microphone_calls
        .lock()
        .expect("lock microphone calls")
        .clear();
    requests.borrow_mut().clear();

    runtime
        .request_reconnect(
            XbxEngineReconnectReasonDto::MediaStalled,
            XbxEngineReconnectTriggerSource::Policy,
        )
        .expect("runtime reconnect should succeed");

    assert_eq!(
        microphone_calls
            .lock()
            .expect("lock microphone calls")
            .as_slice(),
        &[true]
    );
    assert!(requests.borrow().iter().any(|request| matches!(
        request,
        XbxEngineHostRequestDto::ExchangeOffer { channel, restart, .. }
        if channel == "chat" && !restart
    )));
}

#[test]
fn runtime_requests_decoder_reset_when_decode_stall_signal_is_active() {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::new(RefCell::new(Vec::new()));
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0);
    let backend = ScriptedMediaBackend::new(
        XbxEngineMediaNegotiation {
            local_offer_sdp: "offer".to_string(),
            local_candidates: Vec::new(),
            surface_id: "surface:viewport-1".to_string(),
            video_width: 1280,
            video_height: 720,
            first_frame_packet_arrival_time_ms: None,
            frame_decoded_time_ms: None,
            frame_rendered_time_ms: None,
            input_status: XbxEngineInputStatus::default(),
        },
        XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            latest_video_packet_arrival_time_ms: Some(now_ms - 20.0),
            latest_video_host_present_time_ms: Some(now_ms - 100.0),
            latest_video_decode_ok_time_ms: Some(now_ms - 100.0),
            video_decoder_stalled: Some(true),
            video_renderer_stalled: Some(false),
            inbound_video_packet_count_total: 200,
            ..Default::default()
        },
    );
    let runtime_stats = backend.runtime_stats.clone();
    let keyframe_calls = backend.keyframe_request_calls.clone();
    let decoder_reset_calls = backend.decoder_reset_calls.clone();
    let mut runtime = XbxEngineRuntime::with_media_backend(
        browser_runtime_config(),
        TestHostBridge::new(requests),
        TestEventSink::new(events),
        backend,
    );

    runtime
        .start(session(), viewport(), 1.0, None, None)
        .expect("runtime start should succeed");
    runtime.health.connected_at_ms = Some(now_ms - 10_000.0);
    runtime.health.last_frame_seq = 10;
    runtime.health.last_frame_rendered_at_ms = Some(now_ms - 100.0);
    runtime.health.inbound_video_packet_count_total = 200;
    runtime.health.last_video_packet_arrival_at_ms = Some(now_ms - 20.0);
    runtime.health.last_keyframe_request_at_ms = Some(now_ms - 700.0);
    runtime.health.keyframe_requested_for_current_stall = true;

    overwrite_runtime_stats(
        &runtime_stats,
        XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            latest_video_packet_arrival_time_ms: Some(now_ms - 20.0),
            latest_video_host_present_time_ms: Some(now_ms - 100.0),
            latest_video_decode_ok_time_ms: Some(now_ms - 100.0),
            video_decoder_stalled: Some(true),
            video_renderer_stalled: Some(false),
            inbound_video_packet_count_total: 200,
            ..Default::default()
        },
    );

    runtime.tick();

    assert_eq!(*keyframe_calls.lock().expect("lock keyframe calls"), 0);
    assert_eq!(
        *decoder_reset_calls
            .lock()
            .expect("lock decoder reset calls"),
        1
    );
}

#[test]
fn rust_owned_runtime_skips_legacy_runtime_recovery_loop() {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::new(RefCell::new(Vec::new()));
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0);
    let backend = ScriptedMediaBackend::new(
        XbxEngineMediaNegotiation {
            local_offer_sdp: "offer".to_string(),
            local_candidates: Vec::new(),
            surface_id: "surface:viewport-1".to_string(),
            video_width: 1280,
            video_height: 720,
            first_frame_packet_arrival_time_ms: None,
            frame_decoded_time_ms: None,
            frame_rendered_time_ms: None,
            input_status: XbxEngineInputStatus::default(),
        },
        XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            latest_video_packet_arrival_time_ms: Some(now_ms - 20.0),
            latest_video_host_present_time_ms: Some(now_ms - 3_000.0),
            latest_video_decode_ok_time_ms: Some(now_ms - 3_000.0),
            video_decoder_stalled: Some(true),
            video_renderer_stalled: Some(false),
            inbound_video_packet_count_total: 500,
            ..Default::default()
        },
    );
    let keyframe_calls = backend.keyframe_request_calls.clone();
    let decoder_reset_calls = backend.decoder_reset_calls.clone();
    let mut runtime = XbxEngineRuntime::with_media_backend(
        XbxEngineRuntimeConfig::default(),
        TestHostBridge::new(requests),
        TestEventSink::new(events),
        backend,
    );

    runtime
        .start(session(), viewport(), 1.0, None, None)
        .expect("runtime start should succeed");
    runtime.health.connected_at_ms = Some(now_ms - 10_000.0);
    runtime.health.last_frame_seq = 30;
    runtime.health.last_frame_rendered_at_ms = Some(now_ms - 3_000.0);
    runtime.health.inbound_video_packet_count_total = 500;
    runtime.health.last_video_packet_arrival_at_ms = Some(now_ms - 20.0);

    runtime.tick();

    assert_eq!(*keyframe_calls.lock().expect("lock keyframe calls"), 0);
    assert_eq!(
        *decoder_reset_calls
            .lock()
            .expect("lock decoder reset calls"),
        0
    );
}

#[test]
fn runtime_prefers_explicit_decoder_stall_signal_from_stats() {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::new(RefCell::new(Vec::new()));
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0);
    let backend = ScriptedMediaBackend::new(
        XbxEngineMediaNegotiation {
            local_offer_sdp: "offer".to_string(),
            local_candidates: Vec::new(),
            surface_id: "surface:viewport-1".to_string(),
            video_width: 1280,
            video_height: 720,
            first_frame_packet_arrival_time_ms: None,
            frame_decoded_time_ms: None,
            frame_rendered_time_ms: None,
            input_status: XbxEngineInputStatus::default(),
        },
        XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            latest_video_packet_arrival_time_ms: Some(now_ms - 20.0),
            latest_video_host_present_time_ms: Some(now_ms - 100.0),
            latest_video_decode_ok_time_ms: Some(now_ms - 100.0),
            video_decoder_stalled: Some(true),
            video_renderer_stalled: Some(false),
            inbound_video_packet_count_total: 300,
            ..Default::default()
        },
    );
    let runtime_stats = backend.runtime_stats.clone();
    let keyframe_calls = backend.keyframe_request_calls.clone();
    let decoder_reset_calls = backend.decoder_reset_calls.clone();
    let mut runtime = XbxEngineRuntime::with_media_backend(
        browser_runtime_config(),
        TestHostBridge::new(requests),
        TestEventSink::new(events),
        backend,
    );

    runtime
        .start(session(), viewport(), 1.0, None, None)
        .expect("runtime start should succeed");
    runtime.health.connected_at_ms = Some(now_ms - 10_000.0);
    runtime.health.last_frame_seq = 20;
    runtime.health.last_frame_rendered_at_ms = Some(now_ms - 100.0);
    runtime.health.inbound_video_packet_count_total = 300;
    runtime.health.last_video_packet_arrival_at_ms = Some(now_ms - 20.0);

    overwrite_runtime_stats(
        &runtime_stats,
        XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            latest_video_packet_arrival_time_ms: Some(now_ms - 20.0),
            latest_video_host_present_time_ms: Some(now_ms - 100.0),
            latest_video_decode_ok_time_ms: Some(now_ms - 100.0),
            video_decoder_stalled: Some(true),
            video_renderer_stalled: Some(false),
            inbound_video_packet_count_total: 300,
            ..Default::default()
        },
    );

    runtime.tick();

    assert_eq!(*keyframe_calls.lock().expect("lock keyframe calls"), 1);
    assert_eq!(
        *decoder_reset_calls
            .lock()
            .expect("lock decoder reset calls"),
        0
    );
}

#[test]
fn runtime_does_not_use_rendered_snapshot_as_present_freshness_signal() {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::new(RefCell::new(Vec::new()));
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0);
    let backend = ScriptedMediaBackend::new(
        XbxEngineMediaNegotiation {
            local_offer_sdp: "offer".to_string(),
            local_candidates: Vec::new(),
            surface_id: "surface:viewport-1".to_string(),
            video_width: 1280,
            video_height: 720,
            first_frame_packet_arrival_time_ms: None,
            frame_decoded_time_ms: None,
            frame_rendered_time_ms: Some(now_ms - 80.0),
            input_status: XbxEngineInputStatus::default(),
        },
        XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            latest_video_packet_arrival_time_ms: Some(now_ms - 20.0),
            latest_video_host_present_time_ms: None,
            latest_video_decode_ok_time_ms: None,
            video_decoder_stalled: Some(true),
            video_renderer_stalled: Some(false),
            inbound_video_packet_count_total: 300,
            ..Default::default()
        },
    );
    let keyframe_calls = backend.keyframe_request_calls.clone();
    let decoder_reset_calls = backend.decoder_reset_calls.clone();
    let mut runtime = XbxEngineRuntime::with_media_backend(
        browser_runtime_config(),
        TestHostBridge::new(requests),
        TestEventSink::new(events),
        backend,
    );

    runtime
        .start(session(), viewport(), 1.0, None, None)
        .expect("runtime start should succeed");
    runtime.health.connected_at_ms = Some(now_ms - 10_000.0);
    runtime.health.last_frame_seq = 20;
    runtime.health.last_frame_rendered_at_ms = Some(now_ms - 80.0);
    runtime.health.inbound_video_packet_count_total = 300;
    runtime.health.last_video_packet_arrival_at_ms = Some(now_ms - 20.0);

    runtime.tick();

    assert_eq!(*keyframe_calls.lock().expect("lock keyframe calls"), 1);
    assert_eq!(
        *decoder_reset_calls
            .lock()
            .expect("lock decoder reset calls"),
        0
    );
}

#[test]
fn runtime_suppresses_recovery_when_decode_activity_is_fresh() {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::new(RefCell::new(Vec::new()));
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0);
    let backend = ScriptedMediaBackend::new(
        XbxEngineMediaNegotiation {
            local_offer_sdp: "offer".to_string(),
            local_candidates: Vec::new(),
            surface_id: "surface:viewport-1".to_string(),
            video_width: 1280,
            video_height: 720,
            first_frame_packet_arrival_time_ms: None,
            frame_decoded_time_ms: None,
            frame_rendered_time_ms: None,
            input_status: XbxEngineInputStatus::default(),
        },
        XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            latest_video_packet_arrival_time_ms: Some(now_ms - 20.0),
            latest_video_host_present_time_ms: Some(now_ms - 120.0),
            latest_video_decode_ok_time_ms: Some(now_ms - 80.0),
            video_decoder_stalled: Some(true),
            video_renderer_stalled: Some(false),
            inbound_video_packet_count_total: 300,
            ..Default::default()
        },
    );
    let keyframe_calls = backend.keyframe_request_calls.clone();
    let decoder_reset_calls = backend.decoder_reset_calls.clone();
    let mut runtime = XbxEngineRuntime::with_media_backend(
        browser_runtime_config(),
        TestHostBridge::new(requests),
        TestEventSink::new(events),
        backend,
    );

    runtime
        .start(session(), viewport(), 1.0, None, None)
        .expect("runtime start should succeed");
    runtime.health.connected_at_ms = Some(now_ms - 10_000.0);
    runtime.health.last_frame_seq = 20;
    runtime.health.last_frame_rendered_at_ms = Some(now_ms - 120.0);
    runtime.health.last_video_packet_arrival_at_ms = Some(now_ms - 20.0);
    runtime.health.inbound_video_packet_count_total = 300;

    runtime.tick();

    assert_eq!(*keyframe_calls.lock().expect("lock keyframe calls"), 0);
    assert_eq!(
        *decoder_reset_calls
            .lock()
            .expect("lock decoder reset calls"),
        0
    );
}

#[test]
fn runtime_requires_stable_stall_window_before_requesting_keyframe() {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::new(RefCell::new(Vec::new()));
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0);
    let backend = ScriptedMediaBackend::new(
        XbxEngineMediaNegotiation {
            local_offer_sdp: "offer".to_string(),
            local_candidates: Vec::new(),
            surface_id: "surface:viewport-1".to_string(),
            video_width: 1280,
            video_height: 720,
            first_frame_packet_arrival_time_ms: None,
            frame_decoded_time_ms: None,
            frame_rendered_time_ms: None,
            input_status: XbxEngineInputStatus::default(),
        },
        XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            latest_video_packet_arrival_time_ms: Some(now_ms - 15.0),
            latest_video_host_present_time_ms: Some(now_ms - 470.0),
            latest_video_decode_ok_time_ms: Some(now_ms - 470.0),
            video_decoder_stalled: Some(true),
            video_renderer_stalled: Some(false),
            inbound_video_packet_count_total: 320,
            ..Default::default()
        },
    );
    let keyframe_calls = backend.keyframe_request_calls.clone();
    let mut runtime = XbxEngineRuntime::with_media_backend(
        browser_runtime_config(),
        TestHostBridge::new(requests),
        TestEventSink::new(events),
        backend,
    );

    runtime
        .start(session(), viewport(), 1.0, None, None)
        .expect("runtime start should succeed");
    runtime.health.connected_at_ms = Some(now_ms - 10_000.0);
    runtime.health.last_frame_seq = 20;
    runtime.health.last_frame_rendered_at_ms = Some(now_ms - 470.0);
    runtime.health.last_video_packet_arrival_at_ms = Some(now_ms - 15.0);
    runtime.health.inbound_video_packet_count_total = 320;

    runtime.tick();
    assert_eq!(*keyframe_calls.lock().expect("lock keyframe calls"), 0);
}

#[test]
fn runtime_recovery_sequence_stays_keyframe_then_decoder_reset_then_reconnect() {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::new(RefCell::new(Vec::new()));
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0);
    let backend = ScriptedMediaBackend::new(
        XbxEngineMediaNegotiation {
            local_offer_sdp: "offer".to_string(),
            local_candidates: Vec::new(),
            surface_id: "surface:viewport-1".to_string(),
            video_width: 1280,
            video_height: 720,
            first_frame_packet_arrival_time_ms: None,
            frame_decoded_time_ms: None,
            frame_rendered_time_ms: None,
            input_status: XbxEngineInputStatus::default(),
        },
        XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            latest_video_packet_arrival_time_ms: Some(now_ms - 20.0),
            latest_video_host_present_time_ms: Some(now_ms - 2_000.0),
            latest_video_decode_ok_time_ms: Some(now_ms - 2_000.0),
            video_decoder_stalled: Some(true),
            video_renderer_stalled: Some(false),
            inbound_video_packet_count_total: 500,
            ..Default::default()
        },
    );
    let runtime_stats = backend.runtime_stats.clone();
    let keyframe_calls = backend.keyframe_request_calls.clone();
    let decoder_reset_calls = backend.decoder_reset_calls.clone();
    let mut runtime = XbxEngineRuntime::with_media_backend(
        browser_runtime_config(),
        TestHostBridge::new(requests.clone()),
        TestEventSink::new(events),
        backend,
    );

    runtime
        .start(session(), viewport(), 1.0, None, None)
        .expect("runtime start should succeed");
    requests.borrow_mut().clear();
    runtime.health.connected_at_ms = Some(now_ms - 10_000.0);
    runtime.health.last_frame_seq = 30;
    runtime.health.last_frame_rendered_at_ms = Some(now_ms - 2_000.0);
    runtime.health.inbound_video_packet_count_total = 500;
    runtime.health.last_video_packet_arrival_at_ms = Some(now_ms - 20.0);

    // tick-1: 先触发 keyframe
    runtime.tick();
    assert_eq!(*keyframe_calls.lock().expect("lock keyframe calls"), 1);
    assert_eq!(
        *decoder_reset_calls
            .lock()
            .expect("lock decoder reset calls"),
        0
    );

    // tick-2: 满足等待窗口后触发 decoder reset
    runtime.health.last_keyframe_request_at_ms = Some(now_ms - 700.0);
    runtime.health.keyframe_requested_for_current_stall = true;
    runtime.tick();
    assert_eq!(*keyframe_calls.lock().expect("lock keyframe calls"), 1);
    assert_eq!(
        *decoder_reset_calls
            .lock()
            .expect("lock decoder reset calls"),
        1
    );

    // tick-3: 扩大 stall 时长，触发 reconnect
    overwrite_runtime_stats(
        &runtime_stats,
        XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            latest_video_packet_arrival_time_ms: Some(now_ms - 20.0),
            latest_video_host_present_time_ms: Some(now_ms - 5_000.0),
            latest_video_decode_ok_time_ms: Some(now_ms - 5_000.0),
            video_decoder_stalled: Some(true),
            video_renderer_stalled: Some(false),
            inbound_video_packet_count_total: 500,
            ..Default::default()
        },
    );
    runtime.health.last_frame_rendered_at_ms = Some(now_ms - 5_000.0);
    runtime.health.keyframe_requested_for_current_stall = true;
    runtime.health.decoder_reset_requested_for_current_stall = true;
    runtime.tick();

    let request_list = requests.borrow();
    assert!(request_list.iter().any(|request| matches!(
        request,
        XbxEngineHostRequestDto::KeepAliveRemoteSession { .. }
    )));
    assert!(request_list.iter().any(|request| matches!(
        request,
        XbxEngineHostRequestDto::ExchangeOffer { channel, restart, .. }
        if channel == "media" && *restart
    )));
}

#[test]
fn runtime_does_not_emit_error_when_keyframe_request_is_only_control_not_ready() {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::new(RefCell::new(Vec::new()));
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0);
    let backend = ScriptedMediaBackend::new(
        XbxEngineMediaNegotiation {
            local_offer_sdp: "offer".to_string(),
            local_candidates: Vec::new(),
            surface_id: "surface:viewport-1".to_string(),
            video_width: 1280,
            video_height: 720,
            first_frame_packet_arrival_time_ms: None,
            frame_decoded_time_ms: None,
            frame_rendered_time_ms: None,
            input_status: XbxEngineInputStatus::default(),
        },
        XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            latest_video_packet_arrival_time_ms: Some(now_ms - 20.0),
            latest_video_host_present_time_ms: Some(now_ms - 2_000.0),
            latest_video_decode_ok_time_ms: Some(now_ms - 2_000.0),
            video_decoder_stalled: Some(true),
            video_renderer_stalled: Some(false),
            inbound_video_packet_count_total: 500,
            ..Default::default()
        },
    )
    .with_keyframe_error_message("xbxEngineRtcControlChannelNotReadyForKeyframe");
    let mut runtime = XbxEngineRuntime::with_media_backend(
        browser_runtime_config(),
        TestHostBridge::new(requests),
        TestEventSink::new(events.clone()),
        backend,
    );

    runtime
        .start(session(), viewport(), 1.0, None, None)
        .expect("runtime start should succeed");
    runtime.health.connected_at_ms = Some(now_ms - 10_000.0);
    runtime.health.last_frame_seq = 30;
    runtime.health.last_frame_rendered_at_ms = Some(now_ms - 2_000.0);
    runtime.health.inbound_video_packet_count_total = 500;
    runtime.health.last_video_packet_arrival_at_ms = Some(now_ms - 20.0);

    runtime.tick();

    assert!(
        !events.borrow().iter().any(|event| matches!(
            event,
            XbxEngineRuntimeEventDto::ErrorReported { code, .. }
            if code == "requestVideoKeyframeFailed"
        )),
        "control not ready should be treated as pending replay, not runtime error"
    );
}

#[test]
fn runtime_does_not_emit_error_when_decoder_reset_is_only_control_not_ready() {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::new(RefCell::new(Vec::new()));
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0);
    let backend = ScriptedMediaBackend::new(
        XbxEngineMediaNegotiation {
            local_offer_sdp: "offer".to_string(),
            local_candidates: Vec::new(),
            surface_id: "surface:viewport-1".to_string(),
            video_width: 1280,
            video_height: 720,
            first_frame_packet_arrival_time_ms: None,
            frame_decoded_time_ms: None,
            frame_rendered_time_ms: None,
            input_status: XbxEngineInputStatus::default(),
        },
        XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            latest_video_packet_arrival_time_ms: Some(now_ms - 20.0),
            latest_video_host_present_time_ms: Some(now_ms - 2_000.0),
            latest_video_decode_ok_time_ms: Some(now_ms - 2_000.0),
            video_decoder_stalled: Some(true),
            video_renderer_stalled: Some(false),
            inbound_video_packet_count_total: 500,
            ..Default::default()
        },
    )
    .with_decoder_reset_error_message("xbxEngineRtcControlChannelNotReadyForDecoderReset");
    let mut runtime = XbxEngineRuntime::with_media_backend(
        browser_runtime_config(),
        TestHostBridge::new(requests),
        TestEventSink::new(events.clone()),
        backend,
    );

    runtime
        .start(session(), viewport(), 1.0, None, None)
        .expect("runtime start should succeed");
    runtime.health.connected_at_ms = Some(now_ms - 10_000.0);
    runtime.health.last_frame_seq = 30;
    runtime.health.last_frame_rendered_at_ms = Some(now_ms - 2_000.0);
    runtime.health.inbound_video_packet_count_total = 500;
    runtime.health.last_video_packet_arrival_at_ms = Some(now_ms - 20.0);
    runtime.health.last_keyframe_request_at_ms = Some(now_ms - 700.0);
    runtime.health.keyframe_requested_for_current_stall = true;

    runtime.tick();

    assert!(
        !events.borrow().iter().any(|event| matches!(
            event,
            XbxEngineRuntimeEventDto::ErrorReported { code, .. }
            if code == "requestDecoderResetFailed"
        )),
        "control not ready should be treated as pending replay, not runtime error"
    );
}
