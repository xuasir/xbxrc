use std::cell::Cell;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use ohmygamepad_protocol::{
    OhMyGamepadRumbleEffectDto, OhMyGamepadRumbleRequestDto, OhMyGamepadRumbleTargetDto,
};
use xbxengine_protocol::{
    XbxEngineDisplayStateDto, XbxEngineHostRequestDto, XbxEngineHostResponseDto,
    XbxEngineIceCandidateDto, XbxEngineInputEventDto, XbxEngineRuntimeEventDto,
    XbxEngineSessionDto, XbxEngineTargetTypeDto, XbxEngineTransportStateDto, XbxEngineViewportDto,
};

use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::connection::RtcConnectionService;
use crate::transport::rtc::facts::{
    ConnectionLifecycleStateFact, SessionCommand, TransportCommand,
};
use crate::transport::rtc::projection::{
    BweProjection, ConnectionProjection, DiagnosticsProjection, MediaProjection,
    RecoveryProjection, TransportSnapshot,
};
use crate::transport::rtc::session::actor::SessionActor;
use crate::transport::rtc::session::clock::SystemSessionClock;
use crate::transport::rtc::session::policy::RtcSessionPolicy;
use crate::transport::rtc::stack::TestRtcTransportSessionBridge as RtcTransportSessionBridge;
use crate::transport::rtc::stream::video_source::test_fixtures::{
    LocalIngressHealthyBaseline, LocalIngressReplayFixture, LocalIngressReplayPacket,
    LocalIngressReplayProfile,
};
use crate::transport::rtc::stream::RtcMediaService;
use crate::{
    XbxEngineInputBackend, XbxEngineInputStatus, XbxEngineMediaBackend, XbxEngineMediaNegotiation,
    XbxEngineMediaNegotiationRequest, XbxEngineMediaRuntimeStats, XbxEngineRenderFrame,
    XbxEngineRenderPixelData,
};

use super::super::{
    XbxEngineEventSink, XbxEngineHostBridge, XbxEngineRuntime, XbxEngineRuntimeConfig,
    XbxEngineRuntimeError,
};

pub(crate) fn transport_commands(commands: Vec<SessionCommand>) -> Vec<TransportCommand> {
    commands
        .into_iter()
        .filter_map(|command| match command {
            SessionCommand::Transport(command) => Some(command),
            SessionCommand::LocalDecoderReset { .. } => None,
        })
        .collect()
}

#[derive(Clone)]
pub(crate) struct TestHostBridge {
    pub(crate) requests: Rc<RefCell<Vec<XbxEngineHostRequestDto>>>,
    pub(crate) fail_request_kind: Rc<RefCell<Option<&'static str>>>,
    pub(crate) fail_keepalive_message: Rc<RefCell<Option<String>>>,
    pub(crate) poll_ice_batches: Rc<RefCell<Vec<Vec<XbxEngineIceCandidateDto>>>>,
    pub(crate) default_remote_end_of_candidates: Rc<Cell<bool>>,
    pub(crate) cancellation_epoch: Rc<Cell<u64>>,
    pub(crate) cancel_after_request_kind: Rc<RefCell<Option<&'static str>>>,
    pub(crate) call_order: Arc<Mutex<Vec<&'static str>>>,
    pub(crate) rumble_requests: Rc<Mutex<Vec<OhMyGamepadRumbleRequestDto>>>,
    pub(crate) clear_rumble_calls: Rc<Mutex<usize>>,
}

impl TestHostBridge {
    pub(crate) fn new(requests: Rc<RefCell<Vec<XbxEngineHostRequestDto>>>) -> Self {
        Self {
            requests,
            fail_request_kind: Rc::new(RefCell::new(None)),
            fail_keepalive_message: Rc::new(RefCell::new(None)),
            poll_ice_batches: Rc::new(RefCell::new(Vec::new())),
            default_remote_end_of_candidates: Rc::new(Cell::new(true)),
            cancellation_epoch: Rc::new(Cell::new(0)),
            cancel_after_request_kind: Rc::new(RefCell::new(None)),
            call_order: Arc::new(Mutex::new(Vec::new())),
            rumble_requests: Rc::new(Mutex::new(Vec::new())),
            clear_rumble_calls: Rc::new(Mutex::new(0)),
        }
    }

    pub(crate) fn with_failures(
        requests: Rc<RefCell<Vec<XbxEngineHostRequestDto>>>,
        fail_request_kind: Rc<RefCell<Option<&'static str>>>,
    ) -> Self {
        Self {
            requests,
            fail_request_kind,
            fail_keepalive_message: Rc::new(RefCell::new(None)),
            poll_ice_batches: Rc::new(RefCell::new(Vec::new())),
            default_remote_end_of_candidates: Rc::new(Cell::new(true)),
            cancellation_epoch: Rc::new(Cell::new(0)),
            cancel_after_request_kind: Rc::new(RefCell::new(None)),
            call_order: Arc::new(Mutex::new(Vec::new())),
            rumble_requests: Rc::new(Mutex::new(Vec::new())),
            clear_rumble_calls: Rc::new(Mutex::new(0)),
        }
    }

    pub(crate) fn with_call_order(mut self, call_order: Arc<Mutex<Vec<&'static str>>>) -> Self {
        self.call_order = call_order;
        self
    }

    pub(crate) fn with_keepalive_failure_message(self, message: impl Into<String>) -> Self {
        *self.fail_keepalive_message.borrow_mut() = Some(message.into());
        self
    }

    pub(crate) fn with_poll_ice_batches(self, batches: Vec<Vec<XbxEngineIceCandidateDto>>) -> Self {
        *self.poll_ice_batches.borrow_mut() = batches;
        self
    }

    pub(crate) fn without_default_remote_end_of_candidates(self) -> Self {
        self.default_remote_end_of_candidates.set(false);
        self
    }
}

impl XbxEngineHostBridge for TestHostBridge {
    fn current_cancellation_epoch(&self) -> u64 {
        self.cancellation_epoch.get()
    }

    fn present_frame(
        &mut self,
        _viewport: &xbxengine_protocol::XbxEngineViewportDto,
        _surface_id: Option<&str>,
        _frame: &XbxEngineRenderFrame,
    ) -> Result<(), XbxEngineRuntimeError> {
        if let Ok(mut order) = self.call_order.lock() {
            order.push("present");
        }
        Ok(())
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
            XbxEngineHostRequestDto::PollIce { .. } => {
                let candidates = if self.poll_ice_batches.borrow().is_empty() {
                    if self.default_remote_end_of_candidates.get() {
                        vec![remote_end_of_candidates_marker()]
                    } else {
                        Vec::new()
                    }
                } else {
                    self.poll_ice_batches.borrow_mut().remove(0)
                };
                XbxEngineHostResponseDto::IcePolled { candidates }
            }
            XbxEngineHostRequestDto::KeepAliveRemoteSession { .. } => {
                XbxEngineHostResponseDto::KeepAliveAccepted
            }
            XbxEngineHostRequestDto::CloseRemoteSession { .. } => {
                XbxEngineHostResponseDto::RemoteSessionClosed
            }
        })
    }

    fn submit_gamepad_rumble_request(
        &mut self,
        request: OhMyGamepadRumbleRequestDto,
    ) -> Result<(), XbxEngineRuntimeError> {
        if let Ok(mut order) = self.call_order.lock() {
            order.push("rumble_submit");
        }
        self.rumble_requests
            .lock()
            .expect("lock rumble requests")
            .push(request);
        Ok(())
    }

    fn clear_pending_gamepad_rumble_requests(&mut self) -> Result<(), XbxEngineRuntimeError> {
        if let Ok(mut order) = self.call_order.lock() {
            order.push("rumble_clear");
        }
        let mut clear_calls = self
            .clear_rumble_calls
            .lock()
            .expect("lock clear rumble calls");
        *clear_calls += 1;
        Ok(())
    }
}

#[derive(Clone)]
pub(crate) struct TestEventSink {
    pub(crate) events: Rc<RefCell<Vec<XbxEngineRuntimeEventDto>>>,
}

impl TestEventSink {
    pub(crate) fn new(events: Rc<RefCell<Vec<XbxEngineRuntimeEventDto>>>) -> Self {
        Self { events }
    }
}

impl XbxEngineEventSink for TestEventSink {
    fn emit(&mut self, event: XbxEngineRuntimeEventDto) {
        self.events.borrow_mut().push(event);
    }
}

#[derive(Default)]
pub(crate) struct TestInputBackend {
    pub(crate) attached_session_id: Option<String>,
    pub(crate) press_count: usize,
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

pub(crate) fn create_runtime(
    requests: Rc<RefCell<Vec<XbxEngineHostRequestDto>>>,
    events: Rc<RefCell<Vec<XbxEngineRuntimeEventDto>>>,
) -> XbxEngineRuntime<TestHostBridge, TestEventSink> {
    XbxEngineRuntime::new(
        XbxEngineRuntimeConfig::default(),
        TestHostBridge::new(requests),
        TestEventSink::new(events),
    )
}

pub(crate) fn legacy_runtime_config() -> XbxEngineRuntimeConfig {
    let mut config = XbxEngineRuntimeConfig::default();
    config.runtime_name = "browser".to_string();
    config
}

pub(crate) fn overwrite_runtime_stats(
    runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    value: XbxEngineMediaRuntimeStats,
) {
    RuntimeStatsSink::new(runtime_stats.clone()).update(|stats| {
        *stats = value;
    });
}

#[derive(Clone)]
pub(crate) struct ScriptedMediaBackend {
    pub(crate) negotiation: XbxEngineMediaNegotiation,
    pub(crate) runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    pub(crate) latest_render_frame: Arc<Mutex<Option<XbxEngineRenderFrame>>>,
    pub(crate) pending_runtime_recovery_action:
        Arc<Mutex<Option<crate::XbxEnginePendingRuntimeRecoveryAction>>>,
    pub(crate) microphone_capturing_calls: Arc<Mutex<Vec<bool>>>,
    pub(crate) local_ice_gathering_complete_calls: Arc<Mutex<usize>>,
    pub(crate) local_ice_gathering_complete_true_after_calls: usize,
    pub(crate) keyframe_request_calls: Arc<Mutex<usize>>,
    pub(crate) decoder_reset_calls: Arc<Mutex<usize>>,
    pub(crate) fail_video_keyframe: Arc<Mutex<Option<String>>>,
    pub(crate) fail_decoder_reset: Arc<Mutex<Option<String>>>,
    pub(crate) stop_calls: Arc<Mutex<usize>>,
    pub(crate) call_order: Arc<Mutex<Vec<&'static str>>>,
    pub(crate) pending_gamepad_rumble_requests: Arc<Mutex<VecDeque<OhMyGamepadRumbleRequestDto>>>,
}

impl ScriptedMediaBackend {
    pub(crate) fn new(
        negotiation: XbxEngineMediaNegotiation,
        runtime_stats: XbxEngineMediaRuntimeStats,
    ) -> Self {
        Self {
            negotiation,
            runtime_stats: Arc::new(Mutex::new(runtime_stats)),
            latest_render_frame: Arc::new(Mutex::new(None)),
            pending_runtime_recovery_action: Arc::new(Mutex::new(None)),
            microphone_capturing_calls: Arc::new(Mutex::new(Vec::new())),
            local_ice_gathering_complete_calls: Arc::new(Mutex::new(0)),
            local_ice_gathering_complete_true_after_calls: 0,
            keyframe_request_calls: Arc::new(Mutex::new(0)),
            decoder_reset_calls: Arc::new(Mutex::new(0)),
            fail_video_keyframe: Arc::new(Mutex::new(None)),
            fail_decoder_reset: Arc::new(Mutex::new(None)),
            stop_calls: Arc::new(Mutex::new(0)),
            call_order: Arc::new(Mutex::new(Vec::new())),
            pending_gamepad_rumble_requests: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    pub(crate) fn with_call_order(mut self, call_order: Arc<Mutex<Vec<&'static str>>>) -> Self {
        self.call_order = call_order;
        self
    }

    pub(crate) fn with_latest_render_frame(self, frame: XbxEngineRenderFrame) -> Self {
        *self
            .latest_render_frame
            .lock()
            .expect("lock latest render frame") = Some(frame);
        self
    }

    pub(crate) fn with_local_ice_gathering_complete_true_after_calls(
        mut self,
        calls: usize,
    ) -> Self {
        self.local_ice_gathering_complete_true_after_calls = calls;
        self
    }

    pub(crate) fn with_pending_gamepad_rumble_requests(
        self,
        requests: Vec<OhMyGamepadRumbleRequestDto>,
    ) -> Self {
        *self
            .pending_gamepad_rumble_requests
            .lock()
            .expect("lock pending rumble requests") = requests.into_iter().collect();
        self
    }

    pub(crate) fn with_keyframe_error_message(self, message: impl Into<String>) -> Self {
        *self
            .fail_video_keyframe
            .lock()
            .expect("lock keyframe failure message") = Some(message.into());
        self
    }

    pub(crate) fn with_decoder_reset_error_message(self, message: impl Into<String>) -> Self {
        *self
            .fail_decoder_reset
            .lock()
            .expect("lock decoder reset failure message") = Some(message.into());
        self
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
        if self.negotiation.local_candidates.is_empty() {
            Ok(vec![placeholder_local_candidate()])
        } else {
            Ok(self.negotiation.local_candidates.clone())
        }
    }

    fn local_ice_gathering_complete(&self) -> Result<bool, XbxEngineRuntimeError> {
        let mut calls = self
            .local_ice_gathering_complete_calls
            .lock()
            .expect("lock local ice gathering calls");
        *calls += 1;
        Ok(*calls > self.local_ice_gathering_complete_true_after_calls)
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
        if let Ok(mut order) = self.call_order.lock() {
            order.push("snapshot");
        }
        Ok(self
            .runtime_stats
            .lock()
            .expect("lock runtime stats")
            .clone())
    }

    fn update_host_video_present_metrics(
        &mut self,
        metrics: crate::XbxEngineHostVideoPresentMetrics,
    ) -> Result<(), XbxEngineRuntimeError> {
        let mut runtime_stats = self.runtime_stats.lock().expect("lock runtime stats");
        runtime_stats.latest_video_host_present_time_ms = metrics.latest_host_present_time_ms;
        runtime_stats.host_display_tick_epoch = metrics.display_tick_epoch;
        runtime_stats.video_present_epoch = metrics.present_epoch;
        runtime_stats.host_cadence_phase = metrics.cadence_phase;
        runtime_stats.video_present_fps = metrics.present_fps;
        runtime_stats.video_present_submit_count_total = metrics.present_submit_count_total;
        runtime_stats.video_present_drop_count_total = metrics.present_drop_count_total;
        runtime_stats.video_present_overwrite_count_total = metrics.present_overwrite_count_total;
        runtime_stats.host_no_pending_take_count_total = metrics.no_pending_take_count_total;
        runtime_stats.host_no_pending_streak = metrics.no_pending_streak;
        runtime_stats.host_no_pending_max_streak = metrics.no_pending_max_streak;
        runtime_stats.host_no_pending_pressure_level = Some(if metrics.no_pending_streak >= 180 {
            "critical".to_string()
        } else if metrics.no_pending_streak >= 60 {
            "high".to_string()
        } else if metrics.no_pending_streak >= 20 {
            "elevated".to_string()
        } else {
            "normal".to_string()
        });
        runtime_stats.video_present_descriptor_upload_mode = metrics.descriptor_upload_mode;
        runtime_stats.video_present_descriptor_metal_import_count_total =
            metrics.descriptor_metal_import_count_total;
        runtime_stats.video_present_descriptor_cpu_upload_count_total =
            metrics.descriptor_cpu_upload_count_total;
        Ok(())
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

    fn take_pending_gamepad_rumble_requests(
        &mut self,
    ) -> Result<Vec<OhMyGamepadRumbleRequestDto>, XbxEngineRuntimeError> {
        Ok(self
            .pending_gamepad_rumble_requests
            .lock()
            .expect("lock pending rumble requests")
            .drain(..)
            .collect())
    }

    fn take_latest_render_frame(
        &mut self,
    ) -> Result<Option<crate::XbxEngineRenderFrame>, XbxEngineRuntimeError> {
        if let Ok(mut order) = self.call_order.lock() {
            order.push("take_frame");
        }
        Ok(self
            .latest_render_frame
            .lock()
            .expect("lock latest render frame")
            .clone())
    }

    fn acknowledge_latest_render_frame(
        &mut self,
        frame_seq: u64,
    ) -> Result<bool, XbxEngineRuntimeError> {
        if let Ok(mut order) = self.call_order.lock() {
            order.push("ack");
        }
        let mut latest_render_frame = self
            .latest_render_frame
            .lock()
            .expect("lock latest render frame");
        let Some(frame) = latest_render_frame.as_ref() else {
            return Ok(false);
        };
        if frame.frame_seq != frame_seq {
            return Ok(false);
        }
        let frame_rendered_at_ms = frame.rendered_at_ms;
        latest_render_frame.take();
        self.runtime_stats
            .lock()
            .expect("lock runtime stats")
            .latest_video_host_present_time_ms = Some(frame_rendered_at_ms);
        Ok(true)
    }

    fn request_video_keyframe(&mut self) -> Result<(), XbxEngineRuntimeError> {
        if let Some(message) = self
            .fail_video_keyframe
            .lock()
            .expect("lock keyframe failure message")
            .clone()
        {
            return Err(XbxEngineRuntimeError::new(message));
        }
        *self
            .keyframe_request_calls
            .lock()
            .expect("lock keyframe calls") += 1;
        Ok(())
    }

    fn request_decoder_reset(&mut self) -> Result<(), XbxEngineRuntimeError> {
        if let Some(message) = self
            .fail_decoder_reset
            .lock()
            .expect("lock decoder reset failure message")
            .clone()
        {
            return Err(XbxEngineRuntimeError::new(message));
        }
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

pub(crate) fn session() -> XbxEngineSessionDto {
    XbxEngineSessionDto {
        session_id: "session-1".to_string(),
        target_type: XbxEngineTargetTypeDto::Cloud,
        turn_server: None,
    }
}

pub(crate) fn viewport() -> XbxEngineViewportDto {
    XbxEngineViewportDto {
        viewport_id: "viewport-1".to_string(),
    }
}

pub(crate) fn placeholder_local_candidate() -> XbxEngineIceCandidateDto {
    XbxEngineIceCandidateDto {
        candidate: "candidate:placeholder 1 udp 2130706431 127.0.0.1 60000 typ host".to_string(),
        sdp_m_line_index: Some(0),
        sdp_mid: Some("0".to_string()),
    }
}

pub(crate) fn remote_end_of_candidates_marker() -> XbxEngineIceCandidateDto {
    XbxEngineIceCandidateDto {
        candidate: "a=end-of-candidates".to_string(),
        sdp_m_line_index: None,
        sdp_mid: None,
    }
}

pub(crate) fn render_frame(frame_seq: u64, rendered_at_ms: f64) -> XbxEngineRenderFrame {
    XbxEngineRenderFrame {
        width: 1280,
        height: 720,
        frame_seq,
        rendered_at_ms,
        rtp_timestamp: Some(1_234_567),
        is_keyframe: true,
        frame_recovery_disposition: Some("frame-complete-candidate".to_string()),
        frame_unrecoverable_reason: None,
        pixel_data: XbxEngineRenderPixelData::Rgba {
            bytes: vec![0, 0, 0, 255].into(),
        },
    }
}

pub(crate) fn test_rumble_request(
    target: OhMyGamepadRumbleTargetDto,
    strength: f32,
) -> OhMyGamepadRumbleRequestDto {
    OhMyGamepadRumbleRequestDto {
        target,
        effect: OhMyGamepadRumbleEffectDto {
            start_delay_ms: 0,
            duration_ms: 16,
            strong_magnitude: strength,
            weak_magnitude: strength,
            left_trigger: 0.0,
            right_trigger: 0.0,
            repeat: 0,
        },
    }
}

pub(crate) enum PendingReconnectCandidateObservationKind<'a> {
    VideoEscalation { recovery_chain_value: &'a str },
    RuntimeObservation { label: &'a str, summary: &'a str },
}

pub(crate) struct PendingReconnectCandidateMatrixCase<'a> {
    pub(crate) observation_id: u64,
    pub(crate) reason: &'a str,
    pub(crate) reason_domain: crate::XbxEngineRecoveryReasonDomain,
    pub(crate) transport_state: XbxEngineTransportStateDto,
    pub(crate) observation_kind: PendingReconnectCandidateObservationKind<'a>,
    pub(crate) expected_reconnect_request_count: usize,
    pub(crate) expected_last_action: Option<&'a str>,
    pub(crate) expected_last_reason: &'a str,
}

pub(crate) struct PendingReconnectCandidateMatrixOutcome {
    pub(crate) reconnect_request_count: usize,
    pub(crate) last_recovery_action: Option<String>,
    pub(crate) last_recovery_reason: Option<String>,
}

pub(crate) fn drive_pending_reconnect_candidate_matrix_case(
    case: &PendingReconnectCandidateMatrixCase<'_>,
) -> PendingReconnectCandidateMatrixOutcome {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::new(RefCell::new(Vec::new()));
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0);
    let mut runtime_stats = XbxEngineMediaRuntimeStats {
        transport_state: case.transport_state.clone(),
        latest_video_packet_arrival_time_ms: Some(now_ms - 20.0),
        inbound_video_packet_count_total: 500,
        ..Default::default()
    };
    match case.observation_kind {
        PendingReconnectCandidateObservationKind::VideoEscalation {
            recovery_chain_value,
        } => {
            runtime_stats.latest_video_escalation_observation =
                Some(crate::XbxEngineVideoEscalationObservation {
                    observation_id: case.observation_id,
                    reason: case.reason.to_string(),
                    action: "requestReconnectCandidate".to_string(),
                    recovery_stage: "reconnecting".to_string(),
                    recovery_chain_value: recovery_chain_value.to_string(),
                    recovery_failure_cost: "high".to_string(),
                    recovery_window_source: "reconnect-window".to_string(),
                    observed_at_ms: now_ms,
                });
        }
        PendingReconnectCandidateObservationKind::RuntimeObservation { label, summary } => {
            runtime_stats.latest_observation_label = Some(label.to_string());
            runtime_stats.latest_observation_summary = Some(summary.to_string());
        }
    }
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
        runtime_stats,
    );
    *backend
        .pending_runtime_recovery_action
        .lock()
        .expect("lock pending runtime recovery action") = Some(
        crate::XbxEnginePendingRuntimeRecoveryAction::RequestReconnectCandidate {
            observation_id: case.observation_id,
            reason: case.reason.to_string(),
            reason_domain: case.reason_domain,
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

    let reconnect_request_count = requests
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
    PendingReconnectCandidateMatrixOutcome {
        reconnect_request_count,
        last_recovery_action: runtime.snapshot().last_recovery_action.clone(),
        last_recovery_reason: runtime.snapshot().last_recovery_reason.clone(),
    }
}

pub(crate) fn build_transport_session_bridge(
    runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    runtime_config: Arc<Mutex<XbxEngineRuntimeConfig>>,
    pending_runtime_recovery_action: Arc<
        Mutex<Option<crate::XbxEnginePendingRuntimeRecoveryAction>>,
    >,
) -> RtcTransportSessionBridge<'static> {
    let runtime_stats = Box::leak(Box::new(runtime_stats));
    let runtime_config = Box::leak(Box::new(runtime_config));
    let pending_runtime_recovery_action = Box::leak(Box::new(pending_runtime_recovery_action));
    let connection = Box::leak(Box::new(Arc::new(Mutex::new(
        RtcConnectionService::default(),
    ))));
    let media = Box::leak(Box::new(Arc::new(Mutex::new(RtcMediaService::default()))));
    let local_decoder_reset_handle = Box::leak(Box::new(Arc::new(Mutex::new(None))));
    let transport_session = Box::leak(Box::new(Arc::new(Mutex::new(SessionActor::new(
        SystemSessionClock,
        RtcSessionPolicy::new(runtime_config.clone(), runtime_stats.clone()),
    )))));
    let transport_fact_sink = Box::leak(Box::new(Arc::new(Mutex::new(Vec::new()))));
    RtcTransportSessionBridge::new(
        runtime_stats,
        runtime_config,
        pending_runtime_recovery_action,
        connection,
        media,
        local_decoder_reset_handle,
        transport_session,
        transport_fact_sink,
    )
}

pub(crate) fn repair_overflow_runtime_replay_profile(
    repair_limit: usize,
) -> LocalIngressReplayProfile {
    LocalIngressReplayProfile {
        channel_capacity: 1,
        packets: (10u16..=(11 + repair_limit as u16))
            .map(|seq| LocalIngressReplayPacket {
                payload_type: 124,
                sequence_number: seq,
                timestamp: 4_000 + u32::from(seq),
                payload: vec![0x41, 0x88, 0x81, 0x00],
            })
            .collect(),
        baseline: LocalIngressHealthyBaseline {
            now_ms: 9_000.0,
            frame_rtp_timestamp: 4_016,
        },
    }
}

pub(crate) fn build_recovering_snapshot(
    _fixture: &LocalIngressReplayFixture,
    observation_id: u64,
    now_ms: f64,
    frame_count: u64,
    diagnosis_label: &str,
) -> TransportSnapshot {
    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Recovering;
    connection.control_channel_open = true;
    connection.latest_transport_path = Some("Direct".to_string());
    connection.latest_rtt_ms = Some(220.0);
    connection.last_observed_at_ms = Some(now_ms);
    TransportSnapshot::new(
        observation_id,
        now_ms,
        connection,
        MediaProjection {
            frame_count,
            ..MediaProjection::default()
        },
        RecoveryProjection {
            latest_diagnosis_label: Some(diagnosis_label.to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(now_ms),
            ..Default::default()
        },
        BweProjection {
            latest_rtt_ms: Some(220.0),
            latest_loss_ratio_1s: Some(0.05),
            latest_actual_video_bitrate_kbps: Some(5_600.0),
            latest_observed_remb_kbps: Some(6_800),
            latest_transport_path: Some("Direct".to_string()),
            latest_sample_tick_ms: Some(now_ms),
            target_remb_kbps: Some(6_800),
            last_observed_at_ms: Some(now_ms),
        },
        DiagnosticsProjection::default(),
    )
}

pub(crate) fn build_connecting_startup_snapshot(
    observation_id: u64,
    now_ms: f64,
    diagnosis_label: &str,
    rtt_ms: f64,
    actual_bitrate_kbps: f64,
) -> TransportSnapshot {
    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Connecting;
    connection.control_channel_open = true;
    connection.latest_transport_path = Some("Direct".to_string());
    connection.latest_rtt_ms = Some(rtt_ms);
    connection.last_observed_at_ms = Some(now_ms);
    TransportSnapshot::new(
        observation_id,
        now_ms,
        connection,
        MediaProjection::default(),
        RecoveryProjection {
            latest_diagnosis_label: Some(diagnosis_label.to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(now_ms),
            ..Default::default()
        },
        BweProjection {
            latest_rtt_ms: Some(rtt_ms),
            latest_loss_ratio_1s: Some(0.0),
            latest_actual_video_bitrate_kbps: Some(actual_bitrate_kbps),
            latest_observed_remb_kbps: Some(12_000),
            latest_transport_path: Some("Direct".to_string()),
            latest_sample_tick_ms: Some(now_ms),
            target_remb_kbps: Some(12_000),
            last_observed_at_ms: Some(now_ms),
        },
        DiagnosticsProjection::default(),
    )
}

pub(crate) fn count_media_restart_requests(
    requests: &Rc<RefCell<Vec<XbxEngineHostRequestDto>>>,
) -> usize {
    requests
        .borrow()
        .iter()
        .filter(|request| {
            matches!(
                request,
                XbxEngineHostRequestDto::ExchangeOffer { channel, restart, .. }
                if channel == "media" && *restart
            )
        })
        .count()
}
