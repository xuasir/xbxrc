use std::cell::Cell;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use xbxengine_protocol::{
    XbxEngineControlCommandDto, XbxEngineDisplayOptionsDto, XbxEngineDisplayStateDto,
    XbxEngineHostRequestDto, XbxEngineHostResponseDto, XbxEngineIceCandidateDto,
    XbxEngineInputEventDto, XbxEngineReconnectReasonDto, XbxEngineRenderProjectionDto,
    XbxEngineRuntimeCodecPreferenceDto, XbxEngineRuntimeEventDto, XbxEngineRuntimePhaseDto,
    XbxEngineRuntimeProjectionDto, XbxEngineRuntimeRecoveryDto, XbxEngineRuntimeVideoPipelineDto,
    XbxEngineSessionDto, XbxEngineTargetTypeDto, XbxEngineTransportStateDto, XbxEngineViewportDto,
};

use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::{
    PlaceholderXbxEngineMediaBackend, XbxEngineInputBackend, XbxEngineInputStatus,
    XbxEngineMediaBackend, XbxEngineMediaNegotiation, XbxEngineMediaNegotiationRequest,
    XbxEngineMediaRuntimeStats,
};

use super::{
    XbxEngineEventSink, XbxEngineHostBridge, XbxEngineRuntime, XbxEngineRuntimeConfig,
    XbxEngineRuntimeError, XbxEngineRuntimeState,
};

#[derive(Clone)]
struct TestHostBridge {
    requests: Rc<RefCell<Vec<XbxEngineHostRequestDto>>>,
    fail_request_kind: Rc<RefCell<Option<&'static str>>>,
    fail_keepalive_message: Rc<RefCell<Option<String>>>,
    cancellation_epoch: Rc<Cell<u64>>,
    cancel_after_request_kind: Rc<RefCell<Option<&'static str>>>,
}

impl TestHostBridge {
    fn new(requests: Rc<RefCell<Vec<XbxEngineHostRequestDto>>>) -> Self {
        Self {
            requests,
            fail_request_kind: Rc::new(RefCell::new(None)),
            fail_keepalive_message: Rc::new(RefCell::new(None)),
            cancellation_epoch: Rc::new(Cell::new(0)),
            cancel_after_request_kind: Rc::new(RefCell::new(None)),
        }
    }

    fn with_failures(
        requests: Rc<RefCell<Vec<XbxEngineHostRequestDto>>>,
        fail_request_kind: Rc<RefCell<Option<&'static str>>>,
    ) -> Self {
        Self {
            requests,
            fail_request_kind,
            fail_keepalive_message: Rc::new(RefCell::new(None)),
            cancellation_epoch: Rc::new(Cell::new(0)),
            cancel_after_request_kind: Rc::new(RefCell::new(None)),
        }
    }

    fn with_keepalive_failure_message(self, message: impl Into<String>) -> Self {
        *self.fail_keepalive_message.borrow_mut() = Some(message.into());
        self
    }
}

impl XbxEngineHostBridge for TestHostBridge {
    fn current_cancellation_epoch(&self) -> u64 {
        self.cancellation_epoch.get()
    }

    fn request(
        &mut self,
        request: XbxEngineHostRequestDto,
    ) -> Result<XbxEngineHostResponseDto, XbxEngineRuntimeError> {
        self.requests.borrow_mut().push(request.clone());
        let request_kind = match &request {
            XbxEngineHostRequestDto::ExchangeOffer { .. } => "ExchangeOffer",
            XbxEngineHostRequestDto::SubmitIce { .. } => "SubmitIce",
            XbxEngineHostRequestDto::PollIce { .. } => "PollIce",
            XbxEngineHostRequestDto::KeepAliveRemoteSession { .. } => "KeepAliveRemoteSession",
            XbxEngineHostRequestDto::CloseRemoteSession { .. } => "CloseRemoteSession",
        };
        if self
            .fail_request_kind
            .borrow()
            .is_some_and(|kind| kind == request_kind)
        {
            if request_kind == "KeepAliveRemoteSession" {
                if let Some(message) = self.fail_keepalive_message.borrow().clone() {
                    return Err(XbxEngineRuntimeError::new(message));
                }
            }
            return Err(XbxEngineRuntimeError::new(format!(
                "hostBridgeFailure:{request_kind}"
            )));
        }
        if self
            .cancel_after_request_kind
            .borrow()
            .is_some_and(|kind| kind == request_kind)
        {
            self.cancellation_epoch
                .set(self.cancellation_epoch.get().saturating_add(1));
        }
        Ok(match request {
            XbxEngineHostRequestDto::ExchangeOffer { .. } => {
                XbxEngineHostResponseDto::OfferExchanged {
                    answer_sdp: "answer".to_string(),
                }
            }
            XbxEngineHostRequestDto::SubmitIce { .. } => XbxEngineHostResponseDto::IceSubmitted,
            XbxEngineHostRequestDto::PollIce { .. } => XbxEngineHostResponseDto::IcePolled {
                candidates: Vec::new(),
            },
            XbxEngineHostRequestDto::KeepAliveRemoteSession { .. } => {
                XbxEngineHostResponseDto::KeepAliveAccepted
            }
            XbxEngineHostRequestDto::CloseRemoteSession { .. } => {
                XbxEngineHostResponseDto::RemoteSessionClosed
            }
        })
    }
}

#[derive(Clone)]
struct TestEventSink {
    events: Rc<RefCell<Vec<XbxEngineRuntimeEventDto>>>,
}

impl TestEventSink {
    fn new(events: Rc<RefCell<Vec<XbxEngineRuntimeEventDto>>>) -> Self {
        Self { events }
    }
}

impl XbxEngineEventSink for TestEventSink {
    fn emit(&mut self, event: XbxEngineRuntimeEventDto) {
        self.events.borrow_mut().push(event);
    }
}

#[derive(Default)]
struct TestInputBackend {
    attached_session_id: Option<String>,
    press_count: usize,
}

impl XbxEngineInputBackend for TestInputBackend {
    fn attach_session(
        &mut self,
        session_id: &str,
    ) -> Result<XbxEngineInputStatus, XbxEngineRuntimeError> {
        self.attached_session_id = Some(session_id.to_string());
        Ok(XbxEngineInputStatus {
            device_count: 2,
            pad_count: 1,
            route_attached: true,
        })
    }

    fn press_controller_button(
        &mut self,
        _button: &str,
        _duration_ms: u64,
    ) -> Result<XbxEngineInputStatus, XbxEngineRuntimeError> {
        self.press_count += 1;
        Ok(XbxEngineInputStatus {
            device_count: 2,
            pad_count: 1,
            route_attached: true,
        })
    }

    fn snapshot_status(&self) -> Result<XbxEngineInputStatus, XbxEngineRuntimeError> {
        Ok(XbxEngineInputStatus {
            device_count: 2,
            pad_count: 1,
            route_attached: self.attached_session_id.is_some(),
        })
    }

    fn stop(&mut self) -> Result<(), XbxEngineRuntimeError> {
        self.attached_session_id = None;
        Ok(())
    }
}

fn create_runtime(
    requests: Rc<RefCell<Vec<XbxEngineHostRequestDto>>>,
    events: Rc<RefCell<Vec<XbxEngineRuntimeEventDto>>>,
) -> XbxEngineRuntime<TestHostBridge, TestEventSink> {
    XbxEngineRuntime::new(
        XbxEngineRuntimeConfig::default(),
        TestHostBridge::new(requests),
        TestEventSink::new(events),
    )
}

fn overwrite_runtime_stats(
    runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    value: XbxEngineMediaRuntimeStats,
) {
    RuntimeStatsSink::new(runtime_stats.clone()).update(|stats| {
        *stats = value;
    });
}

#[derive(Clone)]
struct ScriptedMediaBackend {
    negotiation: XbxEngineMediaNegotiation,
    runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    pending_runtime_recovery_action:
        Arc<Mutex<Option<crate::XbxEnginePendingRuntimeRecoveryAction>>>,
    microphone_capturing_calls: Arc<Mutex<Vec<bool>>>,
    keyframe_request_calls: Arc<Mutex<usize>>,
    decoder_reset_calls: Arc<Mutex<usize>>,
    stop_calls: Arc<Mutex<usize>>,
}

impl ScriptedMediaBackend {
    fn new(
        negotiation: XbxEngineMediaNegotiation,
        runtime_stats: XbxEngineMediaRuntimeStats,
    ) -> Self {
        Self {
            negotiation,
            runtime_stats: Arc::new(Mutex::new(runtime_stats)),
            pending_runtime_recovery_action: Arc::new(Mutex::new(None)),
            microphone_capturing_calls: Arc::new(Mutex::new(Vec::new())),
            keyframe_request_calls: Arc::new(Mutex::new(0)),
            decoder_reset_calls: Arc::new(Mutex::new(0)),
            stop_calls: Arc::new(Mutex::new(0)),
        }
    }
}

impl XbxEngineMediaBackend for ScriptedMediaBackend {
    fn sync_runtime_config(
        &mut self,
        _runtime_config: &XbxEngineRuntimeConfig,
    ) -> Result<(), XbxEngineRuntimeError> {
        Ok(())
    }

    fn negotiate(
        &mut self,
        _request: XbxEngineMediaNegotiationRequest,
    ) -> Result<XbxEngineMediaNegotiation, XbxEngineRuntimeError> {
        Ok(self.negotiation.clone())
    }

    fn create_offer(&mut self) -> Result<String, XbxEngineRuntimeError> {
        Ok(self.negotiation.local_offer_sdp.clone())
    }

    fn apply_remote_description(
        &mut self,
        _answer_sdp: String,
        _remote_candidates: Vec<XbxEngineIceCandidateDto>,
    ) -> Result<(), XbxEngineRuntimeError> {
        Ok(())
    }

    fn add_remote_ice_candidates(
        &mut self,
        _remote_candidates: Vec<XbxEngineIceCandidateDto>,
    ) -> Result<(), XbxEngineRuntimeError> {
        Ok(())
    }

    fn local_candidates_snapshot(
        &self,
    ) -> Result<Vec<XbxEngineIceCandidateDto>, XbxEngineRuntimeError> {
        Ok(self.negotiation.local_candidates.clone())
    }

    fn local_ice_gathering_complete(&self) -> Result<bool, XbxEngineRuntimeError> {
        Ok(true)
    }

    fn apply_display_state(
        &mut self,
        _state: XbxEngineDisplayStateDto,
    ) -> Result<(), XbxEngineRuntimeError> {
        Ok(())
    }

    fn set_audio_volume(&mut self, _value: f32) -> Result<(), XbxEngineRuntimeError> {
        Ok(())
    }

    fn set_microphone_capturing(&mut self, capturing: bool) -> Result<(), XbxEngineRuntimeError> {
        self.microphone_capturing_calls
            .lock()
            .expect("lock microphone calls")
            .push(capturing);
        Ok(())
    }

    fn press_controller_button(
        &mut self,
        _button: String,
        _duration_ms: u64,
    ) -> Result<(), XbxEngineRuntimeError> {
        Ok(())
    }

    fn set_keyboard_pointer_enabled(
        &mut self,
        _enabled: bool,
    ) -> Result<(), XbxEngineRuntimeError> {
        Ok(())
    }

    fn push_keyboard_pointer_input(
        &mut self,
        _event: XbxEngineInputEventDto,
    ) -> Result<(), XbxEngineRuntimeError> {
        Ok(())
    }

    fn current_input_status(&self) -> Result<crate::XbxEngineInputStatus, XbxEngineRuntimeError> {
        Ok(self.negotiation.input_status.clone())
    }

    fn snapshot_runtime_stats(&self) -> Result<XbxEngineMediaRuntimeStats, XbxEngineRuntimeError> {
        Ok(self
            .runtime_stats
            .lock()
            .expect("lock runtime stats")
            .clone())
    }

    fn take_pending_runtime_recovery_action(
        &mut self,
    ) -> Result<Option<crate::XbxEnginePendingRuntimeRecoveryAction>, XbxEngineRuntimeError> {
        Ok(self
            .pending_runtime_recovery_action
            .lock()
            .expect("lock pending runtime recovery action")
            .take())
    }

    fn take_latest_render_frame(
        &mut self,
    ) -> Result<Option<crate::XbxEngineRenderFrame>, XbxEngineRuntimeError> {
        Ok(None)
    }

    fn request_video_keyframe(&mut self) -> Result<(), XbxEngineRuntimeError> {
        *self
            .keyframe_request_calls
            .lock()
            .expect("lock keyframe calls") += 1;
        Ok(())
    }

    fn request_decoder_reset(&mut self) -> Result<(), XbxEngineRuntimeError> {
        *self
            .decoder_reset_calls
            .lock()
            .expect("lock decoder reset calls") += 1;
        Ok(())
    }

    fn stop(&mut self) -> Result<(), XbxEngineRuntimeError> {
        *self.stop_calls.lock().expect("lock stop calls") += 1;
        Ok(())
    }
}

fn session() -> XbxEngineSessionDto {
    XbxEngineSessionDto {
        session_id: "session-1".to_string(),
        target_type: XbxEngineTargetTypeDto::Cloud,
        turn_server: None,
    }
}

fn viewport() -> XbxEngineViewportDto {
    XbxEngineViewportDto {
        viewport_id: "viewport-1".to_string(),
    }
}

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
                candidates: Vec::new(),
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
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        XbxEngineRuntimeEventDto::TransportConnectionStateChanged {
            state: XbxEngineTransportStateDto::Connected,
        }
    )));
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        XbxEngineRuntimeEventDto::MediaSurfaceReady { surface_id }
        if surface_id == "surface:viewport-1"
    )));
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
        })
        .expect("start runtime control should succeed");

    assert_eq!(runtime.state(), &XbxEngineRuntimeState::Running);
    assert_eq!(runtime.config.webrtc.negotiation.video_bitrate_kbps, 42_000);
    assert_eq!(runtime.config.webrtc.negotiation.audio_bitrate_kbps, 192);
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
        .request_reconnect(XbxEngineReconnectReasonDto::MediaStalled)
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
                candidates: Vec::new(),
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
        TestHostBridge::new(requests),
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
        .request_reconnect(XbxEngineReconnectReasonDto::MediaStalled)
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
        .request_reconnect(XbxEngineReconnectReasonDto::MediaStalled)
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
        .request_reconnect(XbxEngineReconnectReasonDto::MediaStalled)
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
        .request_reconnect(XbxEngineReconnectReasonDto::MediaStalled)
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
            inbound_video_packet_count_total: 200,
            ..Default::default()
        },
    );
    let runtime_stats = backend.runtime_stats.clone();
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
    runtime.health.last_frame_seq = 10;
    runtime.health.last_frame_rendered_at_ms = Some(now_ms - 3_000.0);
    runtime.health.inbound_video_packet_count_total = 200;
    runtime.health.last_video_packet_arrival_at_ms = Some(now_ms - 20.0);
    runtime.health.last_keyframe_request_at_ms = Some(now_ms - 600.0);
    runtime.health.keyframe_requested_for_current_stall = true;

    overwrite_runtime_stats(
        &runtime_stats,
        XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            latest_video_packet_arrival_time_ms: Some(now_ms - 20.0),
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
            latest_video_present_time_ms: Some(now_ms - 100.0),
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
        XbxEngineRuntimeConfig::default(),
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
            latest_video_present_time_ms: Some(now_ms - 100.0),
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
            latest_video_present_time_ms: Some(now_ms - 120.0),
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
        XbxEngineRuntimeConfig::default(),
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
            latest_video_present_time_ms: Some(now_ms - 470.0),
            latest_video_decode_ok_time_ms: Some(now_ms - 470.0),
            video_decoder_stalled: Some(true),
            video_renderer_stalled: Some(false),
            inbound_video_packet_count_total: 320,
            ..Default::default()
        },
    );
    let keyframe_calls = backend.keyframe_request_calls.clone();
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
            latest_video_present_time_ms: Some(now_ms - 2_000.0),
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
        XbxEngineRuntimeConfig::default(),
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
            latest_video_present_time_ms: Some(now_ms - 5_000.0),
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
fn runtime_consumes_pending_transport_reconnect_candidate_once() {
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
            inbound_video_packet_count_total: 500,
            latest_video_escalation_observation: Some(crate::XbxEngineVideoEscalationObservation {
                observation_id: 42,
                reason: "transportExpiredDeadline".to_string(),
                action: "requestReconnectCandidate".to_string(),
                observed_at_ms: now_ms,
            }),
            ..Default::default()
        },
    );
    *backend
        .pending_runtime_recovery_action
        .lock()
        .expect("lock pending runtime recovery action") = Some(
        crate::XbxEnginePendingRuntimeRecoveryAction::RequestReconnectCandidate {
            observation_id: 42,
            reason: "transportExpiredDeadline".to_string(),
        },
    );
    let mut runtime = XbxEngineRuntime::with_media_backend(
        XbxEngineRuntimeConfig::default(),
        TestHostBridge::new(requests.clone()),
        TestEventSink::new(events),
        backend,
    );

    runtime
        .start(session(), viewport(), 1.0, None, None)
        .expect("runtime start should succeed");
    requests.borrow_mut().clear();

    runtime.tick();
    let reconnect_request_count_after_first_tick = requests
        .borrow()
        .iter()
        .filter(|request| {
            matches!(
                request,
                XbxEngineHostRequestDto::ExchangeOffer { channel, restart, .. }
                if channel == "media" && *restart
            )
        })
        .count();
    assert_eq!(reconnect_request_count_after_first_tick, 1);
    assert_eq!(
        runtime.snapshot().last_recovery_reason.as_deref(),
        Some("transportReconnectCandidate:transportExpiredDeadline")
    );

    runtime.tick();
    let reconnect_request_count_after_second_tick = requests
        .borrow()
        .iter()
        .filter(|request| {
            matches!(
                request,
                XbxEngineHostRequestDto::ExchangeOffer { channel, restart, .. }
                if channel == "media" && *restart
            )
        })
        .count();
    assert_eq!(reconnect_request_count_after_second_tick, 1);
}

#[test]
fn runtime_stops_reconnect_loop_when_keepalive_reports_session_not_active() {
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
            inbound_video_packet_count_total: 500,
            latest_video_escalation_observation: Some(crate::XbxEngineVideoEscalationObservation {
                observation_id: 42,
                reason: "transportExpiredDeadline".to_string(),
                action: "requestReconnectCandidate".to_string(),
                observed_at_ms: now_ms,
            }),
            ..Default::default()
        },
    );
    *backend
        .pending_runtime_recovery_action
        .lock()
        .expect("lock pending runtime recovery action") = Some(
        crate::XbxEnginePendingRuntimeRecoveryAction::RequestReconnectCandidate {
            observation_id: 42,
            reason: "transportExpiredDeadline".to_string(),
        },
    );
    let fail_request_kind = Rc::new(RefCell::new(Some("KeepAliveRemoteSession")));
    let host_bridge = TestHostBridge::with_failures(requests.clone(), fail_request_kind)
        .with_keepalive_failure_message(
            "keepAliveRemoteSession:streaming:HTTP 410 SessionNotActive",
        );
    let mut runtime = XbxEngineRuntime::with_media_backend(
        XbxEngineRuntimeConfig::default(),
        host_bridge,
        TestEventSink::new(events.clone()),
        backend,
    );

    runtime
        .start(session(), viewport(), 1.0, None, None)
        .expect("runtime start should succeed");

    runtime.tick();

    assert_eq!(runtime.state(), &XbxEngineRuntimeState::Stopped);
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        XbxEngineRuntimeEventDto::ErrorReported { code, message }
        if code == "recoverTransportReconnectSessionNotActive"
            && message.contains("HTTP 410")
    )));
}

#[test]
fn runtime_waits_for_real_frame_before_populating_first_frame_timestamps() {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::new(RefCell::new(Vec::new()));
    let backend = ScriptedMediaBackend::new(
        XbxEngineMediaNegotiation {
            local_offer_sdp: "offer".to_string(),
            local_candidates: Vec::new(),
            surface_id: "surface:viewport-1".to_string(),
            video_width: 1920,
            video_height: 1080,
            first_frame_packet_arrival_time_ms: None,
            frame_decoded_time_ms: None,
            frame_rendered_time_ms: None,
            input_status: XbxEngineInputStatus::default(),
        },
        XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            latest_video_frame: None,
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

    assert_eq!(runtime.snapshot().first_frame_packet_arrival_time_ms, None);
    assert_eq!(runtime.snapshot().frame_decoded_time_ms, None);
    assert!(!events.borrow().iter().any(|event| matches!(
        event,
        XbxEngineRuntimeEventDto::StatsVideoFrameProcessed { .. }
    )));

    let frame_time_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0);
    overwrite_runtime_stats(
        &runtime_stats,
        XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            latest_video_frame: Some(crate::XbxEngineVideoFrameStats {
                width: 1920,
                height: 1080,
                frame_seq: 1,
                fps: 60.0,
                rendered_at_ms: frame_time_ms,
            }),
            ..Default::default()
        },
    );

    runtime.tick();

    assert_eq!(
        runtime.snapshot().first_frame_packet_arrival_time_ms,
        Some(frame_time_ms)
    );
    assert_eq!(
        runtime.snapshot().frame_decoded_time_ms,
        Some(frame_time_ms)
    );
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        XbxEngineRuntimeEventDto::StatsVideoFrameProcessed {
            first_frame_packet_arrival_time_ms,
            frame_decoded_time_ms,
            frame_rendered_time_ms,
        }
        if *first_frame_packet_arrival_time_ms == frame_time_ms
            && *frame_decoded_time_ms == frame_time_ms
            && *frame_rendered_time_ms == frame_time_ms
    )));
}

#[test]
fn runtime_syncs_keyframe_request_count_from_media_stats() {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::new(RefCell::new(Vec::new()));
    let backend = ScriptedMediaBackend::new(
        XbxEngineMediaNegotiation {
            local_offer_sdp: "offer".to_string(),
            local_candidates: Vec::new(),
            surface_id: "surface:viewport-1".to_string(),
            video_width: 1920,
            video_height: 1080,
            first_frame_packet_arrival_time_ms: None,
            frame_decoded_time_ms: None,
            frame_rendered_time_ms: None,
            input_status: XbxEngineInputStatus::default(),
        },
        XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            video_pli_request_count_total: 3,
            ..Default::default()
        },
    );
    let runtime_stats = backend.runtime_stats.clone();
    let mut runtime = XbxEngineRuntime::with_media_backend(
        XbxEngineRuntimeConfig::default(),
        TestHostBridge::new(requests),
        TestEventSink::new(events),
        backend,
    );

    runtime
        .start(session(), viewport(), 1.0, None, None)
        .expect("runtime start should succeed");
    assert_eq!(runtime.snapshot().recovery_keyframe_request_count, 3);

    runtime_stats
        .lock()
        .expect("lock runtime stats")
        .video_pli_request_count_total = 5;
    runtime.tick();

    assert_eq!(runtime.snapshot().recovery_keyframe_request_count, 5);
}
