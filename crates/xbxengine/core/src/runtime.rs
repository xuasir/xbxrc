use std::error::Error;
use std::fmt::{Display, Formatter};

use xbxengine_protocol::{
    XbxEngineControlCommandDto, XbxEngineDisplayStateDto, XbxEngineHostRequestDto,
    XbxEngineHostResponseDto, XbxEngineIceCandidateDto, XbxEngineInputEventDto,
    XbxEngineReconnectReasonDto, XbxEngineRuntimeEventDto, XbxEngineRuntimePhaseDto,
    XbxEngineSessionDto, XbxEngineStatsDto, XbxEngineTransportStateDto, XbxEngineViewportDto,
};

use crate::{
    build_xbxengine_stats, PlaceholderXbxEngineMediaBackend, XbxEngineMediaBackend,
    XbxEngineMediaNegotiation, XbxEngineMediaNegotiationRequest, XbxEngineMediaRuntimeStats,
    XbxEngineRecoveryAction, XbxEngineRenderFrame, XbxEngineRuntimeHealth,
};

const DIAGNOSTICS_WINDOW_MS: f64 = 1_000.0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum XbxEngineRuntimeState {
    Idle,
    Starting,
    Running,
    Reconnecting,
    Stopped,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XbxEngineRuntimeConfig {
    pub runtime_name: String,
    pub webrtc: XbxEngineWebRtcRuntimeConfig,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XbxEngineWebRtcRuntimeConfig {
    pub forced_remb_kbps: Option<u32>,
    pub adaptive_remb_enabled: bool,
    pub video_pipeline: XbxEngineVideoPipelineRuntimeConfig,
    pub rtt_diagnostics: XbxEngineRttDiagnosticsRuntimeConfig,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XbxEngineVideoPipelineRuntimeConfig {
    pub nack_window_ms: u64,
    pub nack_retry_interval_ms: u64,
    pub nack_max_retry_count: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XbxEngineRttDiagnosticsRuntimeConfig {
    pub enabled: bool,
    pub log_interval_ms: u64,
}

impl Default for XbxEngineWebRtcRuntimeConfig {
    fn default() -> Self {
        Self {
            forced_remb_kbps: None,
            adaptive_remb_enabled: true,
            video_pipeline: XbxEngineVideoPipelineRuntimeConfig::default(),
            rtt_diagnostics: XbxEngineRttDiagnosticsRuntimeConfig::default(),
        }
    }
}

impl Default for XbxEngineVideoPipelineRuntimeConfig {
    fn default() -> Self {
        Self {
            // 云游戏稳态下放宽 finalize 窗口，降低过早判损。
            nack_window_ms: 400,
            nack_retry_interval_ms: 60,
            nack_max_retry_count: 5,
        }
    }
}

impl Default for XbxEngineRttDiagnosticsRuntimeConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            log_interval_ms: 5_000,
        }
    }
}

impl Default for XbxEngineRuntimeConfig {
    fn default() -> Self {
        Self {
            runtime_name: "rust-owned".to_string(),
            webrtc: XbxEngineWebRtcRuntimeConfig::default(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct XbxEngineRuntimeSnapshot {
    pub audio_volume: f32,
    pub keyboard_pointer_enabled: bool,
    pub microphone_capturing: bool,
    pub microphone_paused: bool,
    pub display_state: Option<XbxEngineDisplayStateDto>,
    pub viewport: Option<XbxEngineViewportDto>,
    pub surface_id: Option<String>,
    pub video_size: Option<(u32, u32)>,
    pub last_keyboard_pointer_event: Option<XbxEngineInputEventDto>,
    pub last_pressed_controller_button: Option<(String, u64)>,
    pub negotiation_attempt_count: usize,
    pub last_offer_sdp: Option<String>,
    pub last_answer_sdp: Option<String>,
    pub last_remote_candidates: Vec<XbxEngineIceCandidateDto>,
    pub input_device_count: usize,
    pub input_pad_count: usize,
    pub input_route_attached: bool,
    pub first_frame_packet_arrival_time_ms: Option<f64>,
    pub frame_decoded_time_ms: Option<f64>,
    pub frame_rendered_time_ms: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XbxEngineRuntimeError {
    message: String,
}

impl XbxEngineRuntimeError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl Display for XbxEngineRuntimeError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl Error for XbxEngineRuntimeError {}

pub trait XbxEngineHostBridge {
    fn request(
        &mut self,
        request: XbxEngineHostRequestDto,
    ) -> Result<XbxEngineHostResponseDto, XbxEngineRuntimeError>;
}

pub trait XbxEngineEventSink {
    fn emit(&mut self, event: XbxEngineRuntimeEventDto);
}

#[derive(Default)]
pub struct NoopXbxEngineHostBridge;

impl XbxEngineHostBridge for NoopXbxEngineHostBridge {
    fn request(
        &mut self,
        _request: XbxEngineHostRequestDto,
    ) -> Result<XbxEngineHostResponseDto, XbxEngineRuntimeError> {
        Err(XbxEngineRuntimeError::new("xbxEngineHostBridgeUnavailable"))
    }
}

#[derive(Default)]
pub struct NoopXbxEngineEventSink;

impl XbxEngineEventSink for NoopXbxEngineEventSink {
    fn emit(&mut self, _event: XbxEngineRuntimeEventDto) {}
}

/**
 * `xbxengine` 保持高内聚：连接状态机、媒体协商、输入和恢复逻辑都在这里收口。
 * 当前先围绕可替换 media backend 组织控制流，后续接入 GStreamer 后只替换 backend。
 */
pub struct XbxEngineRuntime<
    THostBridge,
    TEventSink,
    TMediaBackend = PlaceholderXbxEngineMediaBackend,
> {
    config: XbxEngineRuntimeConfig,
    host_bridge: THostBridge,
    event_sink: TEventSink,
    media_backend: TMediaBackend,
    state: XbxEngineRuntimeState,
    session: Option<XbxEngineSessionDto>,
    snapshot: XbxEngineRuntimeSnapshot,
    health: XbxEngineRuntimeHealth,
    diagnostics_window_started_at_ms: f64,
    diagnostics_window_start_frame_seq: u64,
    diagnostics_window_start_inbound_bytes: u64,
    diagnostics_window_start_inbound_video_bytes: u64,
    diagnostics_window_start_inbound_primary_video_bytes: u64,
    diagnostics_window_start_inbound_audio_bytes: u64,
    diagnostics_window_start_inbound_video_packets: u64,
    diagnostics_window_start_video_loss_finalized_count: u64,
    diagnostics_window_start_video_loss_recovered_count: u64,
    diagnostics_window_start_video_loss_late_recovered_count: u64,
}

impl<THostBridge, TEventSink>
    XbxEngineRuntime<THostBridge, TEventSink, PlaceholderXbxEngineMediaBackend>
where
    THostBridge: XbxEngineHostBridge,
    TEventSink: XbxEngineEventSink,
{
    pub fn new(
        config: XbxEngineRuntimeConfig,
        host_bridge: THostBridge,
        event_sink: TEventSink,
    ) -> Self {
        Self::with_media_backend(
            config,
            host_bridge,
            event_sink,
            PlaceholderXbxEngineMediaBackend::default(),
        )
    }
}

impl<THostBridge, TEventSink, TMediaBackend>
    XbxEngineRuntime<THostBridge, TEventSink, TMediaBackend>
where
    THostBridge: XbxEngineHostBridge,
    TEventSink: XbxEngineEventSink,
    TMediaBackend: XbxEngineMediaBackend,
{
    pub fn with_media_backend(
        config: XbxEngineRuntimeConfig,
        host_bridge: THostBridge,
        event_sink: TEventSink,
        media_backend: TMediaBackend,
    ) -> Self {
        Self {
            config,
            host_bridge,
            event_sink,
            media_backend,
            state: XbxEngineRuntimeState::Idle,
            session: None,
            snapshot: XbxEngineRuntimeSnapshot::default(),
            health: XbxEngineRuntimeHealth::default(),
            diagnostics_window_started_at_ms: 0.0,
            diagnostics_window_start_frame_seq: 0,
            diagnostics_window_start_inbound_bytes: 0,
            diagnostics_window_start_inbound_video_bytes: 0,
            diagnostics_window_start_inbound_primary_video_bytes: 0,
            diagnostics_window_start_inbound_audio_bytes: 0,
            diagnostics_window_start_inbound_video_packets: 0,
            diagnostics_window_start_video_loss_finalized_count: 0,
            diagnostics_window_start_video_loss_recovered_count: 0,
            diagnostics_window_start_video_loss_late_recovered_count: 0,
        }
    }

    pub fn config(&self) -> &XbxEngineRuntimeConfig {
        &self.config
    }

    pub fn state(&self) -> &XbxEngineRuntimeState {
        &self.state
    }

    pub fn snapshot(&self) -> &XbxEngineRuntimeSnapshot {
        &self.snapshot
    }

    pub fn snapshot_stats(&self) -> XbxEngineStatsDto {
        let runtime_stats = self.media_backend.snapshot_runtime_stats().ok();
        build_xbxengine_stats(&self.snapshot, runtime_stats.as_ref())
    }

    /**
     * Rust 原生窗口宿主需要直接消费最新渲染帧，
     * 这里显式暴露一个“只取最新、不做排队”的只读出口。
     */
    pub fn take_latest_render_frame(
        &mut self,
    ) -> Result<Option<XbxEngineRenderFrame>, XbxEngineRuntimeError> {
        self.media_backend.take_latest_render_frame()
    }

    pub fn start(
        &mut self,
        session: XbxEngineSessionDto,
        viewport: XbxEngineViewportDto,
        audio_volume: f32,
    ) -> Result<(), XbxEngineRuntimeError> {
        let previous_state = self.state.clone();
        let previous_session = self.session.clone();
        let previous_snapshot = self.snapshot.clone();
        let previous_health = self.health.clone();

        self.state = XbxEngineRuntimeState::Starting;
        self.session = Some(session);
        self.snapshot.viewport = Some(viewport);
        self.snapshot.audio_volume = audio_volume;
        self.health = XbxEngineRuntimeHealth::default();
        self.reset_diagnostics_window();

        let start_result = (|| {
            self.media_backend.set_audio_volume(audio_volume)?;
            self.emit_phase(XbxEngineRuntimePhaseDto::Binding);
            self.negotiate_remote(false)?;
            Ok(())
        })();

        match start_result {
            Ok(()) => {
                self.state = XbxEngineRuntimeState::Running;
                Ok(())
            }
            Err(error) => {
                // 启动失败后回到上一个稳定态，避免 runtime 卡死在中间态。
                let _ = self.media_backend.stop();
                self.state = previous_state;
                self.session = previous_session;
                self.snapshot = previous_snapshot;
                self.health = previous_health;
                Err(error)
            }
        }
    }

    pub fn request_reconnect(
        &mut self,
        _reason: XbxEngineReconnectReasonDto,
    ) -> Result<(), XbxEngineRuntimeError> {
        let previous_state = self.state.clone();
        let previous_session = self.session.clone();
        let previous_snapshot = self.snapshot.clone();
        let previous_health = self.health.clone();
        let session_id = self.require_session_id()?;
        let reconnect_started_at_ms = now_ms_f64();
        self.state = XbxEngineRuntimeState::Reconnecting;
        self.health.mark_reconnect_started(reconnect_started_at_ms);
        self.emit_phase(XbxEngineRuntimePhaseDto::Reconnecting);

        let reconnect_result = (|| {
            let _ = self
                .host_bridge
                .request(XbxEngineHostRequestDto::KeepAliveRemoteSession { session_id })?;
            self.negotiate_remote(true)?;
            Ok(())
        })();

        match reconnect_result {
            Ok(()) => {
                self.state = XbxEngineRuntimeState::Running;
                Ok(())
            }
            Err(error) => {
                self.state = previous_state;
                self.session = previous_session;
                self.snapshot = previous_snapshot;
                self.health = previous_health;
                self.health
                    .restore_reconnect_marker(reconnect_started_at_ms);
                Err(error)
            }
        }
    }

    pub fn stop(&mut self) {
        if let Some(session) = self.session.take() {
            if let Err(error) =
                self.host_bridge
                    .request(XbxEngineHostRequestDto::CloseRemoteSession {
                        session_id: session.session_id,
                        reason: Some("runtimeStopped".to_string()),
                    })
            {
                self.emit_error("closeRemoteSessionFailed", error.to_string());
            }
        }
        if let Err(error) = self.media_backend.stop() {
            self.emit_error("stopMediaBackendFailed", error.to_string());
        }
        self.snapshot.viewport = None;
        self.snapshot.surface_id = None;
        self.snapshot.video_size = None;
        self.health = XbxEngineRuntimeHealth::default();
        self.reset_diagnostics_window();
        self.state = XbxEngineRuntimeState::Stopped;
        self.emit_transport_state(XbxEngineTransportStateDto::Closed);
    }

    pub fn tick(&mut self) {
        if !matches!(
            self.state,
            XbxEngineRuntimeState::Running | XbxEngineRuntimeState::Reconnecting
        ) {
            return;
        }

        let runtime_stats = match self.media_backend.snapshot_runtime_stats() {
            Ok(stats) => stats,
            Err(error) => {
                self.emit_error("snapshotMediaRuntimeStatsFailed", error.to_string());
                return;
            }
        };

        self.sync_transport_state(&runtime_stats);
        self.sync_video_packet_stats(&runtime_stats);
        self.sync_video_frame_stats(&runtime_stats);
        self.maybe_emit_diagnostics_pulse(&runtime_stats);
        self.maybe_recover_media_stall(&runtime_stats);
    }

    pub fn apply_control(
        &mut self,
        command: XbxEngineControlCommandDto,
    ) -> Result<(), XbxEngineRuntimeError> {
        match command {
            XbxEngineControlCommandDto::StartRuntime {
                session,
                viewport,
                audio_volume,
            } => self.start(session, viewport, audio_volume),
            XbxEngineControlCommandDto::StopRuntime => {
                self.stop();
                Ok(())
            }
            XbxEngineControlCommandDto::RequestReconnect { reason } => {
                self.request_reconnect(reason)
            }
            XbxEngineControlCommandDto::AttachViewport { viewport } => {
                self.snapshot.viewport = Some(viewport);
                Ok(())
            }
            XbxEngineControlCommandDto::DetachViewport => {
                self.snapshot.viewport = None;
                Ok(())
            }
            XbxEngineControlCommandDto::ApplyDisplayState { state } => {
                self.media_backend.apply_display_state(state.clone())?;
                self.snapshot.display_state = Some(state);
                Ok(())
            }
            XbxEngineControlCommandDto::SetAudioVolume { value } => {
                self.media_backend.set_audio_volume(value)?;
                self.snapshot.audio_volume = value;
                Ok(())
            }
            XbxEngineControlCommandDto::StartMicrophone => {
                self.media_backend.set_microphone_capturing(true)?;
                self.snapshot.microphone_capturing = true;
                self.snapshot.microphone_paused = false;
                self.event_sink
                    .emit(XbxEngineRuntimeEventDto::ChatStateChanged {
                        capturing: true,
                        paused: false,
                    });
                Ok(())
            }
            XbxEngineControlCommandDto::StopMicrophone => {
                self.media_backend.set_microphone_capturing(false)?;
                self.snapshot.microphone_capturing = false;
                self.snapshot.microphone_paused = false;
                self.event_sink
                    .emit(XbxEngineRuntimeEventDto::ChatStateChanged {
                        capturing: false,
                        paused: false,
                    });
                Ok(())
            }
            XbxEngineControlCommandDto::PressControllerButton {
                button,
                duration_ms,
            } => {
                self.media_backend
                    .press_controller_button(button.clone(), duration_ms)?;
                let input_status = self.media_backend.current_input_status()?;
                self.record_input_status(&input_status);
                self.snapshot.last_pressed_controller_button = Some((button, duration_ms));
                Ok(())
            }
            XbxEngineControlCommandDto::SetKeyboardPointerEnabled { enabled } => {
                self.media_backend.set_keyboard_pointer_enabled(enabled)?;
                self.snapshot.keyboard_pointer_enabled = enabled;
                Ok(())
            }
            XbxEngineControlCommandDto::PushKeyboardPointerInput { event } => {
                self.media_backend
                    .push_keyboard_pointer_input(event.clone())?;
                self.snapshot.last_keyboard_pointer_event = Some(event);
                Ok(())
            }
        }
    }

    fn negotiate_remote(&mut self, restart: bool) -> Result<(), XbxEngineRuntimeError> {
        let negotiation = self
            .media_backend
            .negotiate(XbxEngineMediaNegotiationRequest {
                session: self.require_session()?.clone(),
                viewport: self.require_viewport()?.clone(),
                restart,
            })?;

        self.snapshot.negotiation_attempt_count += 1;
        self.snapshot.last_offer_sdp = Some(negotiation.local_offer_sdp.clone());

        self.emit_phase(XbxEngineRuntimePhaseDto::ExchangingOffer);
        let answer_sdp = Self::extract_offer_response(self.host_bridge.request(
            XbxEngineHostRequestDto::ExchangeOffer {
                session_id: self.require_session_id()?,
                channel: "media".to_string(),
                sdp: negotiation.local_offer_sdp.clone(),
                restart,
            },
        )?)?;

        self.emit_phase(XbxEngineRuntimePhaseDto::GatheringIce);
        self.emit_phase(XbxEngineRuntimePhaseDto::ExchangingIce);
        let remote_candidates = Self::extract_ice_response(self.host_bridge.request(
            XbxEngineHostRequestDto::ExchangeIce {
                session_id: self.require_session_id()?,
                candidates: negotiation.local_candidates.clone(),
                restart,
            },
        )?)?;

        self.media_backend
            .apply_remote_description(answer_sdp.clone(), remote_candidates.clone())?;
        self.snapshot.last_answer_sdp = Some(answer_sdp);
        self.snapshot.last_remote_candidates = remote_candidates;
        self.record_media_ready(&negotiation);
        self.record_input_status(&negotiation.input_status);

        self.emit_phase(XbxEngineRuntimePhaseDto::Connecting);
        self.health.observed_transport_state = XbxEngineTransportStateDto::Connecting;
        self.emit_transport_state(XbxEngineTransportStateDto::Connecting);
        if let Ok(runtime_stats) = self.media_backend.snapshot_runtime_stats() {
            self.sync_transport_state(&runtime_stats);
            self.sync_video_frame_stats(&runtime_stats);
        }
        Ok(())
    }

    fn require_session(&self) -> Result<&XbxEngineSessionDto, XbxEngineRuntimeError> {
        self.session
            .as_ref()
            .ok_or_else(|| XbxEngineRuntimeError::new("xbxEngineSessionMissing"))
    }

    fn require_session_id(&self) -> Result<String, XbxEngineRuntimeError> {
        self.require_session()
            .map(|session| session.session_id.clone())
    }

    fn require_viewport(&self) -> Result<&XbxEngineViewportDto, XbxEngineRuntimeError> {
        self.snapshot
            .viewport
            .as_ref()
            .ok_or_else(|| XbxEngineRuntimeError::new("xbxEngineViewportMissing"))
    }

    fn record_media_ready(&mut self, negotiation: &XbxEngineMediaNegotiation) {
        self.snapshot.surface_id = Some(negotiation.surface_id.clone());
        self.snapshot.video_size = Some((negotiation.video_width, negotiation.video_height));
        self.snapshot.first_frame_packet_arrival_time_ms =
            negotiation.first_frame_packet_arrival_time_ms;
        self.snapshot.frame_decoded_time_ms = negotiation.frame_decoded_time_ms;
        self.snapshot.frame_rendered_time_ms = negotiation.frame_rendered_time_ms;
        self.event_sink
            .emit(XbxEngineRuntimeEventDto::MediaSurfaceReady {
                surface_id: negotiation.surface_id.clone(),
            });
        self.event_sink
            .emit(XbxEngineRuntimeEventDto::MediaVideoReady {
                width: negotiation.video_width,
                height: negotiation.video_height,
            });
    }

    fn record_input_status(&mut self, status: &crate::XbxEngineInputStatus) {
        self.snapshot.input_device_count = status.device_count;
        self.snapshot.input_pad_count = status.pad_count;
        self.snapshot.input_route_attached = status.route_attached;
    }

    fn extract_offer_response(
        response: XbxEngineHostResponseDto,
    ) -> Result<String, XbxEngineRuntimeError> {
        match response {
            XbxEngineHostResponseDto::OfferExchanged { answer_sdp } => Ok(answer_sdp),
            _ => Err(XbxEngineRuntimeError::new(
                "xbxEngineHostBridgeInvalidOfferResponse",
            )),
        }
    }

    fn extract_ice_response(
        response: XbxEngineHostResponseDto,
    ) -> Result<Vec<XbxEngineIceCandidateDto>, XbxEngineRuntimeError> {
        match response {
            XbxEngineHostResponseDto::IceExchanged { candidates } => Ok(candidates),
            _ => Err(XbxEngineRuntimeError::new(
                "xbxEngineHostBridgeInvalidIceResponse",
            )),
        }
    }

    fn emit_phase(&mut self, phase: XbxEngineRuntimePhaseDto) {
        self.event_sink
            .emit(XbxEngineRuntimeEventDto::RuntimePhaseChanged { phase });
    }

    fn emit_transport_state(&mut self, state: XbxEngineTransportStateDto) {
        self.event_sink
            .emit(XbxEngineRuntimeEventDto::TransportConnectionStateChanged { state });
    }

    fn emit_error(&mut self, code: impl Into<String>, message: impl Into<String>) {
        self.event_sink
            .emit(XbxEngineRuntimeEventDto::ErrorReported {
                code: code.into(),
                message: message.into(),
            });
    }

    fn sync_transport_state(&mut self, stats: &XbxEngineMediaRuntimeStats) {
        let now_ms = now_ms_f64();
        if !self
            .health
            .sync_transport_state(&stats.transport_state, now_ms)
        {
            return;
        }
        self.emit_transport_state(stats.transport_state.clone());
    }

    fn sync_video_frame_stats(&mut self, stats: &XbxEngineMediaRuntimeStats) {
        let Some(frame) = stats.latest_video_frame.as_ref() else {
            return;
        };
        let had_advanced_frame = frame.frame_seq > self.health.last_frame_seq;
        let video_size_changed = self.health.record_video_frame(
            frame.width,
            frame.height,
            frame.frame_seq,
            frame.rendered_at_ms,
        );
        if !had_advanced_frame {
            return;
        }
        self.snapshot.video_size = Some((frame.width, frame.height));
        self.snapshot.frame_rendered_time_ms = Some(frame.rendered_at_ms);

        if self.snapshot.first_frame_packet_arrival_time_ms.is_none() {
            self.snapshot.first_frame_packet_arrival_time_ms = Some(frame.rendered_at_ms);
        }
        if self.snapshot.frame_decoded_time_ms.is_none() {
            self.snapshot.frame_decoded_time_ms = Some(frame.rendered_at_ms);
        }

        if video_size_changed.is_some() {
            self.event_sink
                .emit(XbxEngineRuntimeEventDto::MediaVideoReady {
                    width: frame.width,
                    height: frame.height,
                });
        }

        self.event_sink
            .emit(XbxEngineRuntimeEventDto::StatsVideoFrameProcessed {
                first_frame_packet_arrival_time_ms: self
                    .snapshot
                    .first_frame_packet_arrival_time_ms
                    .unwrap_or(frame.rendered_at_ms),
                frame_decoded_time_ms: self
                    .snapshot
                    .frame_decoded_time_ms
                    .unwrap_or(frame.rendered_at_ms),
                frame_rendered_time_ms: frame.rendered_at_ms,
            });
    }

    fn sync_video_packet_stats(&mut self, stats: &XbxEngineMediaRuntimeStats) {
        let Some(arrived_at_ms) = stats.latest_video_packet_arrival_time_ms else {
            return;
        };
        self.health
            .record_video_packet_activity(stats.inbound_video_packet_count_total, arrived_at_ms);
        if self.snapshot.first_frame_packet_arrival_time_ms.is_none() {
            self.snapshot.first_frame_packet_arrival_time_ms = Some(arrived_at_ms);
        }
    }

    fn maybe_emit_diagnostics_pulse(&mut self, stats: &XbxEngineMediaRuntimeStats) {
        let now_ms = now_ms_f64();
        if self.diagnostics_window_started_at_ms <= 0.0 {
            self.diagnostics_window_started_at_ms = now_ms;
            self.diagnostics_window_start_frame_seq = self.health.last_frame_seq;
            self.diagnostics_window_start_inbound_bytes = stats.inbound_bytes_total;
            self.diagnostics_window_start_inbound_video_bytes = stats.inbound_video_bytes_total;
            self.diagnostics_window_start_inbound_primary_video_bytes =
                stats.inbound_primary_video_bytes_total;
            self.diagnostics_window_start_inbound_audio_bytes = stats.inbound_audio_bytes_total;
            self.diagnostics_window_start_inbound_video_packets =
                stats.inbound_video_packet_count_total;
            self.diagnostics_window_start_video_loss_finalized_count =
                stats.video_loss_finalized_count_total;
            self.diagnostics_window_start_video_loss_recovered_count =
                stats.video_loss_recovered_count_total;
            self.diagnostics_window_start_video_loss_late_recovered_count =
                stats.video_loss_late_recovered_count_total;
            return;
        }

        let elapsed_ms = now_ms - self.diagnostics_window_started_at_ms;
        if elapsed_ms < DIAGNOSTICS_WINDOW_MS {
            return;
        }

        let frames_in_window = self
            .health
            .last_frame_seq
            .saturating_sub(self.diagnostics_window_start_frame_seq);
        let fps = if elapsed_ms > 0.0 {
            (frames_in_window as f64 * 1_000.0) / elapsed_ms
        } else {
            0.0
        };
        let render_idle_ms = self
            .health
            .last_frame_rendered_at_ms
            .map(|rendered_at_ms| (now_ms - rendered_at_ms).max(0.0));
        let inbound_bytes_in_window = stats
            .inbound_bytes_total
            .saturating_sub(self.diagnostics_window_start_inbound_bytes);
        let inbound_video_bytes_in_window = stats
            .inbound_video_bytes_total
            .saturating_sub(self.diagnostics_window_start_inbound_video_bytes);
        let inbound_primary_video_bytes_in_window = stats
            .inbound_primary_video_bytes_total
            .saturating_sub(self.diagnostics_window_start_inbound_primary_video_bytes);
        let inbound_audio_bytes_in_window = stats
            .inbound_audio_bytes_total
            .saturating_sub(self.diagnostics_window_start_inbound_audio_bytes);
        let inbound_kbps = if elapsed_ms > 0.0 {
            (inbound_bytes_in_window as f64 * 8.0) / elapsed_ms
        } else {
            0.0
        };
        let inbound_video_kbps = if elapsed_ms > 0.0 {
            (inbound_video_bytes_in_window as f64 * 8.0) / elapsed_ms
        } else {
            0.0
        };
        let inbound_primary_video_kbps = if elapsed_ms > 0.0 {
            (inbound_primary_video_bytes_in_window as f64 * 8.0) / elapsed_ms
        } else {
            0.0
        };
        let inbound_audio_kbps = if elapsed_ms > 0.0 {
            (inbound_audio_bytes_in_window as f64 * 8.0) / elapsed_ms
        } else {
            0.0
        };
        let inbound_video_packets_in_window = stats
            .inbound_video_packet_count_total
            .saturating_sub(self.diagnostics_window_start_inbound_video_packets);
        let video_loss_finalized_packets_in_window = stats
            .video_loss_finalized_count_total
            .saturating_sub(self.diagnostics_window_start_video_loss_finalized_count);
        let video_loss_recovered_packets_in_window = stats
            .video_loss_recovered_count_total
            .saturating_sub(self.diagnostics_window_start_video_loss_recovered_count);
        let video_loss_late_recovered_packets_in_window = stats
            .video_loss_late_recovered_count_total
            .saturating_sub(self.diagnostics_window_start_video_loss_late_recovered_count);
        let (video_width, video_height) = stats
            .latest_video_frame
            .as_ref()
            .map(|frame| (Some(frame.width), Some(frame.height)))
            .or_else(|| {
                match (
                    stats.latest_video_stream_width,
                    stats.latest_video_stream_height,
                ) {
                    (Some(width), Some(height)) if width > 0 && height > 0 => {
                        Some((Some(width), Some(height)))
                    }
                    _ => None,
                }
            })
            .unwrap_or((None, None));

        self.event_sink
            .emit(XbxEngineRuntimeEventDto::DiagnosticsPulse {
                window_ms: elapsed_ms,
                frames_in_window,
                fps,
                render_idle_ms,
                inbound_kbps,
                inbound_video_kbps,
                inbound_primary_video_kbps,
                inbound_audio_kbps,
                inbound_video_packets_in_window,
                inbound_video_loss_ratio_1s: stats.inbound_video_loss_ratio_1s,
                inbound_video_loss_ratio_5s: stats.inbound_video_loss_ratio_5s,
                video_rtt_ms: stats.video_rtt_ms,
                video_rtt_source: stats.video_rtt_source.clone(),
                video_nack_recovery_rtt_ms: stats.video_nack_recovery_rtt_ms,
                video_remb_bps: stats.video_remb_bps,
                inbound_video_jitter_ms: stats.inbound_video_jitter_ms,
                video_loss_finalized_packets_in_window,
                video_loss_recovered_packets_in_window,
                video_loss_late_recovered_packets_in_window,
                video_width,
                video_height,
                transport_state: stats.transport_state.clone(),
            });

        self.diagnostics_window_started_at_ms = now_ms;
        self.diagnostics_window_start_frame_seq = self.health.last_frame_seq;
        self.diagnostics_window_start_inbound_bytes = stats.inbound_bytes_total;
        self.diagnostics_window_start_inbound_video_bytes = stats.inbound_video_bytes_total;
        self.diagnostics_window_start_inbound_primary_video_bytes =
            stats.inbound_primary_video_bytes_total;
        self.diagnostics_window_start_inbound_audio_bytes = stats.inbound_audio_bytes_total;
        self.diagnostics_window_start_inbound_video_packets =
            stats.inbound_video_packet_count_total;
        self.diagnostics_window_start_video_loss_finalized_count =
            stats.video_loss_finalized_count_total;
        self.diagnostics_window_start_video_loss_recovered_count =
            stats.video_loss_recovered_count_total;
        self.diagnostics_window_start_video_loss_late_recovered_count =
            stats.video_loss_late_recovered_count_total;
    }

    fn reset_diagnostics_window(&mut self) {
        self.diagnostics_window_started_at_ms = 0.0;
        self.diagnostics_window_start_frame_seq = self.health.last_frame_seq;
        self.diagnostics_window_start_inbound_bytes = 0;
        self.diagnostics_window_start_inbound_video_bytes = 0;
        self.diagnostics_window_start_inbound_primary_video_bytes = 0;
        self.diagnostics_window_start_inbound_audio_bytes = 0;
        self.diagnostics_window_start_inbound_video_packets = 0;
        self.diagnostics_window_start_video_loss_finalized_count = 0;
        self.diagnostics_window_start_video_loss_recovered_count = 0;
        self.diagnostics_window_start_video_loss_late_recovered_count = 0;
    }

    fn maybe_recover_media_stall(&mut self, stats: &XbxEngineMediaRuntimeStats) {
        let now = now_ms_f64();
        let next_action = self.health.next_recovery_action(
            now,
            self.state == XbxEngineRuntimeState::Running,
            &stats.transport_state,
        );
        match next_action {
            Some(XbxEngineRecoveryAction::RequestVideoKeyframe) => {
                if let Err(error) = self.media_backend.request_video_keyframe() {
                    self.emit_error("requestVideoKeyframeFailed", error.to_string());
                } else {
                    self.health.mark_keyframe_requested(now);
                }
            }
            Some(XbxEngineRecoveryAction::RequestReconnect(reason)) => {
                if let Err(error) = self.request_reconnect(reason) {
                    self.emit_error("recoverMediaStallFailed", error.to_string());
                }
            }
            None => {}
        }
    }
}

fn now_ms_f64() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::{Arc, Mutex};
    use std::time::{SystemTime, UNIX_EPOCH};

    use xbxengine_protocol::{
        XbxEngineControlCommandDto, XbxEngineDisplayOptionsDto, XbxEngineDisplayStateDto,
        XbxEngineHostRequestDto, XbxEngineHostResponseDto, XbxEngineIceCandidateDto,
        XbxEngineInputEventDto, XbxEngineReconnectReasonDto, XbxEngineRuntimeEventDto,
        XbxEngineRuntimePhaseDto, XbxEngineSessionDto, XbxEngineTargetTypeDto,
        XbxEngineTransportStateDto, XbxEngineViewportDto,
    };

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
    }

    impl TestHostBridge {
        fn new(requests: Rc<RefCell<Vec<XbxEngineHostRequestDto>>>) -> Self {
            Self {
                requests,
                fail_request_kind: Rc::new(RefCell::new(None)),
            }
        }

        fn with_failures(
            requests: Rc<RefCell<Vec<XbxEngineHostRequestDto>>>,
            fail_request_kind: Rc<RefCell<Option<&'static str>>>,
        ) -> Self {
            Self {
                requests,
                fail_request_kind,
            }
        }
    }

    impl XbxEngineHostBridge for TestHostBridge {
        fn request(
            &mut self,
            request: XbxEngineHostRequestDto,
        ) -> Result<XbxEngineHostResponseDto, XbxEngineRuntimeError> {
            self.requests.borrow_mut().push(request.clone());
            let request_kind = match &request {
                XbxEngineHostRequestDto::ExchangeOffer { .. } => "ExchangeOffer",
                XbxEngineHostRequestDto::ExchangeIce { .. } => "ExchangeIce",
                XbxEngineHostRequestDto::KeepAliveRemoteSession { .. } => "KeepAliveRemoteSession",
                XbxEngineHostRequestDto::CloseRemoteSession { .. } => "CloseRemoteSession",
            };
            if self
                .fail_request_kind
                .borrow()
                .is_some_and(|kind| kind == request_kind)
            {
                return Err(XbxEngineRuntimeError::new(format!(
                    "hostBridgeFailure:{request_kind}"
                )));
            }
            Ok(match request {
                XbxEngineHostRequestDto::ExchangeOffer { .. } => {
                    XbxEngineHostResponseDto::OfferExchanged {
                        answer_sdp: "answer".to_string(),
                    }
                }
                XbxEngineHostRequestDto::ExchangeIce { .. } => {
                    XbxEngineHostResponseDto::IceExchanged {
                        candidates: Vec::new(),
                    }
                }
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

    #[derive(Clone)]
    struct ScriptedMediaBackend {
        negotiation: XbxEngineMediaNegotiation,
        runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
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
                stop_calls: Arc::new(Mutex::new(0)),
            }
        }
    }

    impl XbxEngineMediaBackend for ScriptedMediaBackend {
        fn negotiate(
            &mut self,
            _request: XbxEngineMediaNegotiationRequest,
        ) -> Result<XbxEngineMediaNegotiation, XbxEngineRuntimeError> {
            Ok(self.negotiation.clone())
        }

        fn apply_remote_description(
            &mut self,
            _answer_sdp: String,
            _remote_candidates: Vec<XbxEngineIceCandidateDto>,
        ) -> Result<(), XbxEngineRuntimeError> {
            Ok(())
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

        fn set_microphone_capturing(
            &mut self,
            _capturing: bool,
        ) -> Result<(), XbxEngineRuntimeError> {
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

        fn current_input_status(
            &self,
        ) -> Result<crate::XbxEngineInputStatus, XbxEngineRuntimeError> {
            Ok(self.negotiation.input_status.clone())
        }

        fn snapshot_runtime_stats(
            &self,
        ) -> Result<XbxEngineMediaRuntimeStats, XbxEngineRuntimeError> {
            Ok(self
                .runtime_stats
                .lock()
                .expect("lock runtime stats")
                .clone())
        }

        fn take_latest_render_frame(
            &mut self,
        ) -> Result<Option<crate::XbxEngineRenderFrame>, XbxEngineRuntimeError> {
            Ok(None)
        }

        fn request_video_keyframe(&mut self) -> Result<(), XbxEngineRuntimeError> {
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
            .start(session(), viewport(), 0.75)
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
                XbxEngineHostRequestDto::ExchangeIce {
                    session_id: "session-1".to_string(),
                    candidates: Vec::new(),
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
                XbxEngineRuntimePhaseDto::ExchangingIce,
                XbxEngineRuntimePhaseDto::Connecting,
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
    fn reconnect_keeps_remote_session_alive_before_restart_negotiation() {
        let requests = Rc::new(RefCell::new(Vec::new()));
        let events = Rc::new(RefCell::new(Vec::new()));
        let mut runtime = create_runtime(requests.clone(), events.clone());

        runtime
            .start(session(), viewport(), 1.0)
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
                XbxEngineHostRequestDto::ExchangeIce {
                    session_id: "session-1".to_string(),
                    candidates: Vec::new(),
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
            .start(session(), viewport(), 0.3)
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
        let backend = PlaceholderXbxEngineMediaBackend::with_input_backend(Box::new(
            TestInputBackend::default(),
        ));
        let requests = Rc::new(RefCell::new(Vec::new()));
        let events = Rc::new(RefCell::new(Vec::new()));
        let mut runtime = XbxEngineRuntime::with_media_backend(
            XbxEngineRuntimeConfig::default(),
            TestHostBridge::new(requests),
            TestEventSink::new(events),
            backend,
        );

        runtime
            .start(session(), viewport(), 1.0)
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
            .start(session(), viewport(), 0.75)
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
            .start(session(), viewport(), 1.0)
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
            .start(session(), viewport(), 1.0)
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
        *runtime_stats.lock().expect("lock runtime stats") = XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            latest_video_frame: Some(crate::XbxEngineVideoFrameStats {
                width: 1920,
                height: 1080,
                frame_seq: 1,
                fps: 60.0,
                rendered_at_ms: frame_time_ms,
            }),
            ..Default::default()
        };

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
}
