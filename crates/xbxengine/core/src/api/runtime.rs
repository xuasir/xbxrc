use std::error::Error;
use std::fmt::{Display, Formatter};

use xbxengine_protocol::{
    XbxEngineControlCommandDto, XbxEngineDisplayStateDto, XbxEngineHostRequestDto,
    XbxEngineHostResponseDto, XbxEngineIceCandidateDto, XbxEngineInputEventDto,
    XbxEngineReconnectReasonDto, XbxEngineRenderProjectionDto, XbxEngineRuntimeEventDto,
    XbxEngineRuntimePhaseDto, XbxEngineRuntimeProjectionDto, XbxEngineSessionDto,
    XbxEngineStatsDto, XbxEngineTransportStateDto, XbxEngineViewportDto,
};

use crate::{
    build_xbxengine_stats, PlaceholderXbxEngineMediaBackend, XbxEngineDecodeRenderSignal,
    XbxEngineMediaBackend, XbxEngineMediaNegotiation, XbxEngineMediaNegotiationRequest,
    XbxEngineMediaRuntimeStats, XbxEngineMediaSignal, XbxEngineRecoveryAction,
    XbxEngineRecoveryRuntimeConfig, XbxEngineRecoverySignals, XbxEngineRenderFrame,
    XbxEngineRuntimeHealth, XbxEngineTransportSignal, STALL_SIGNAL_STABILITY_MS,
};

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
    pub bwe_mode: String,
    pub forced_remb_kbps: Option<u32>,
    pub adaptive_remb_enabled: bool,
    pub remb_floor_kbps: u32,
    pub remb_ceiling_kbps: u32,
    pub remb_ramp_up_step_kbps: u32,
    pub remb_ramp_down_factor: u16,
    pub negotiation: XbxEngineNegotiationRuntimeConfig,
    pub video_pipeline: XbxEngineVideoPipelineRuntimeConfig,
    pub rtt_diagnostics: XbxEngineRttDiagnosticsRuntimeConfig,
    pub recovery: XbxEngineRecoveryRuntimeConfig,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XbxEngineVideoPipelineRuntimeConfig {
    pub feedback_interval_ms: u64,
    pub nack_window_ms: u64,
    pub nack_burst_count: u16,
    pub nack_max_age_ms: u64,
    pub nack_retry_interval_ms: u64,
    pub nack_max_retry_count: u8,
    pub jitter_buffer_min_delay_ms: u64,
    pub jitter_buffer_max_delay_ms: u64,
    pub jitter_buffer_max_packets: u16,
    pub idle_timeout_ms: u64,
    pub late_frame_drop_threshold_ms: u64,
    pub backlog_drop_threshold_packets: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XbxEngineNegotiationRuntimeConfig {
    pub target_resolution_width: u32,
    pub target_resolution_height: u32,
    pub video_bitrate_kbps: u32,
    pub audio_bitrate_kbps: u32,
    pub force_mono_audio: bool,
    pub offer_profile: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XbxEngineRttDiagnosticsRuntimeConfig {
    pub enabled: bool,
    pub log_interval_ms: u64,
}

impl XbxEngineWebRtcRuntimeConfig {
    pub fn base() -> Self {
        Self {
            // 这些值只作为 plan 未显式覆盖时的保守缺省。
            bwe_mode: "twcc-gcc".to_string(),
            forced_remb_kbps: Some(50_000),
            adaptive_remb_enabled: false,
            remb_floor_kbps: 8_000,
            remb_ceiling_kbps: 50_000,
            remb_ramp_up_step_kbps: 2_000,
            remb_ramp_down_factor: 850,
            negotiation: XbxEngineNegotiationRuntimeConfig::default(),
            video_pipeline: XbxEngineVideoPipelineRuntimeConfig {
                feedback_interval_ms: 1_000,
                nack_window_ms: 400,
                nack_burst_count: 12,
                nack_max_age_ms: 200,
                nack_retry_interval_ms: 60,
                nack_max_retry_count: 5,
                jitter_buffer_min_delay_ms: 20,
                jitter_buffer_max_delay_ms: 30,
                jitter_buffer_max_packets: 1024,
                idle_timeout_ms: 150,
                late_frame_drop_threshold_ms: 500,
                backlog_drop_threshold_packets: 10,
            },
            rtt_diagnostics: XbxEngineRttDiagnosticsRuntimeConfig::default(),
            recovery: XbxEngineRecoveryRuntimeConfig::default(),
        }
    }
}

impl Default for XbxEngineWebRtcRuntimeConfig {
    fn default() -> Self {
        Self::base()
    }
}

impl Default for XbxEngineRttDiagnosticsRuntimeConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            log_interval_ms: 10_000,
        }
    }
}

impl Default for XbxEngineNegotiationRuntimeConfig {
    fn default() -> Self {
        Self {
            target_resolution_width: 1920,
            target_resolution_height: 1080,
            video_bitrate_kbps: 15_000,
            audio_bitrate_kbps: 128,
            force_mono_audio: false,
            offer_profile: "macos".to_string(), // maybe overwritten by front-end
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
    pub recovery_keyframe_request_count: u64,
    pub recovery_decoder_reset_count: u64,
    pub recovery_reconnect_count: u64,
    pub last_recovery_action: Option<String>,
    pub last_recovery_action_at_ms: Option<f64>,
    pub last_recovery_reason: Option<String>,
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

    pub fn is_cancelled(&self) -> bool {
        self.message == "xbxEngineRuntimeCancelled"
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

    fn attach_viewport(
        &mut self,
        _viewport: &XbxEngineViewportDto,
        _surface_id: Option<&str>,
    ) -> Result<(), XbxEngineRuntimeError> {
        Ok(())
    }

    fn detach_viewport(&mut self, _viewport_id: Option<&str>) -> Result<(), XbxEngineRuntimeError> {
        Ok(())
    }

    fn present_frame(
        &mut self,
        _viewport: &XbxEngineViewportDto,
        _surface_id: Option<&str>,
        _frame: &XbxEngineRenderFrame,
    ) -> Result<(), XbxEngineRuntimeError> {
        Ok(())
    }

    fn current_cancellation_epoch(&self) -> u64 {
        0
    }
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
    last_transport_reconnect_candidate_observation_id: Option<u64>,
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
            last_transport_reconnect_candidate_observation_id: None,
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
        runtime: Option<XbxEngineRuntimeProjectionDto>,
        render: Option<XbxEngineRenderProjectionDto>,
    ) -> Result<(), XbxEngineRuntimeError> {
        let previous_config = self.config.clone();
        let previous_state = self.state.clone();
        let previous_session = self.session.clone();
        let previous_snapshot = self.snapshot.clone();
        let previous_health = self.health.clone();
        let previous_transport_reconnect_candidate_observation_id =
            self.last_transport_reconnect_candidate_observation_id;

        self.state = XbxEngineRuntimeState::Starting;
        self.session = Some(session);
        self.snapshot.viewport = Some(viewport);
        self.snapshot.audio_volume = audio_volume;
        let operation_epoch = self.host_bridge.current_cancellation_epoch();

        let start_result = (|| {
            self.apply_execution_spec(runtime.as_ref(), render.as_ref())?;
            self.media_backend.sync_runtime_config(&self.config)?;
            self.media_backend.set_audio_volume(audio_volume)?;
            self.emit_phase(XbxEngineRuntimePhaseDto::Binding);
            self.ensure_operation_active(operation_epoch)?;
            self.negotiate_remote(false, operation_epoch)?;
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
                self.config = previous_config;
                self.state = previous_state;
                self.session = previous_session;
                self.snapshot = previous_snapshot;
                self.health = previous_health;
                self.last_transport_reconnect_candidate_observation_id =
                    previous_transport_reconnect_candidate_observation_id;
                Err(error)
            }
        }
    }

    pub fn request_reconnect(
        &mut self,
        reason: XbxEngineReconnectReasonDto,
    ) -> Result<(), XbxEngineRuntimeError> {
        let previous_state = self.state.clone();
        let previous_session = self.session.clone();
        let previous_snapshot = self.snapshot.clone();
        let previous_health = self.health.clone();
        let previous_transport_reconnect_candidate_observation_id =
            self.last_transport_reconnect_candidate_observation_id;
        let session_id = self.require_session_id()?;
        let reconnect_started_at_ms = now_ms_f64();
        let operation_epoch = self.host_bridge.current_cancellation_epoch();
        self.state = XbxEngineRuntimeState::Reconnecting;
        self.health.mark_reconnect_started(reconnect_started_at_ms);
        self.snapshot.recovery_reconnect_count =
            self.snapshot.recovery_reconnect_count.saturating_add(1);
        self.snapshot.last_recovery_action = Some("reconnect".to_string());
        self.snapshot.last_recovery_action_at_ms = Some(reconnect_started_at_ms);
        self.snapshot.last_recovery_reason = Some(format!("{reason:?}"));
        self.emit_phase(XbxEngineRuntimePhaseDto::Reconnecting);

        let reconnect_result = (|| {
            self.ensure_operation_active(operation_epoch)?;
            let _ = self
                .host_bridge
                .request(XbxEngineHostRequestDto::KeepAliveRemoteSession { session_id })?;
            self.ensure_operation_active(operation_epoch)?;
            self.negotiate_remote(true, operation_epoch)?;
            if self.snapshot.microphone_capturing {
                self.ensure_operation_active(operation_epoch)?;
                // 重连后 PeerConnection 已重建，需要先恢复本地 mic track，再走 chat 重协商。
                self.media_backend.set_microphone_capturing(true)?;
                self.renegotiate_chat_channel()?;
            }
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
                self.last_transport_reconnect_candidate_observation_id =
                    previous_transport_reconnect_candidate_observation_id;
                self.health
                    .restore_reconnect_marker(reconnect_started_at_ms);
                Err(error)
            }
        }
    }

    pub fn stop(&mut self) {
        let viewport_id = self
            .snapshot
            .viewport
            .as_ref()
            .map(|viewport| viewport.viewport_id.as_str());
        if let Err(error) = self.host_bridge.detach_viewport(viewport_id) {
            self.emit_error("detachViewportFailed", error.to_string());
        }
        self.session = None;
        if let Err(error) = self.media_backend.stop() {
            self.emit_error("stopMediaBackendFailed", error.to_string());
        }
        self.snapshot.viewport = None;
        self.snapshot.surface_id = None;
        self.snapshot.video_size = None;
        self.health = XbxEngineRuntimeHealth::default();
        self.last_transport_reconnect_candidate_observation_id = None;
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

        self.present_latest_render_frame();
        self.sync_transport_state(&runtime_stats);
        self.sync_video_packet_stats(&runtime_stats);
        self.sync_video_frame_stats(&runtime_stats);
        if self.maybe_recover_transport_reconnect_candidate(&runtime_stats) {
            return;
        }
        self.maybe_recover_media_stall(&runtime_stats);
    }

    fn maybe_recover_transport_reconnect_candidate(
        &mut self,
        stats: &XbxEngineMediaRuntimeStats,
    ) -> bool {
        let Some(observation) = stats.latest_video_escalation_observation.as_ref() else {
            return false;
        };
        if observation.action != "requestReconnectCandidate" {
            return false;
        }
        if self.last_transport_reconnect_candidate_observation_id
            == Some(observation.observation_id)
        {
            return false;
        }
        self.last_transport_reconnect_candidate_observation_id = Some(observation.observation_id);
        if let Err(error) = self.request_reconnect(XbxEngineReconnectReasonDto::MediaStalled) {
            if !error.is_cancelled() {
                self.emit_error(
                    "recoverTransportReconnectCandidateFailed",
                    error.to_string(),
                );
            }
        } else {
            self.snapshot.last_recovery_reason = Some(format!(
                "transportReconnectCandidate:{}",
                observation.reason
            ));
        }
        true
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
                runtime,
                render,
            } => self.start(session, viewport, audio_volume, runtime, render),
            XbxEngineControlCommandDto::StopRuntime => {
                self.stop();
                Ok(())
            }
            XbxEngineControlCommandDto::RequestReconnect { reason } => {
                self.request_reconnect(reason)
            }
            XbxEngineControlCommandDto::AttachViewport { viewport } => {
                self.host_bridge
                    .attach_viewport(&viewport, self.snapshot.surface_id.as_deref())?;
                self.snapshot.viewport = Some(viewport);
                Ok(())
            }
            XbxEngineControlCommandDto::DetachViewport => {
                let viewport_id = self
                    .snapshot
                    .viewport
                    .as_ref()
                    .map(|viewport| viewport.viewport_id.as_str());
                self.host_bridge.detach_viewport(viewport_id)?;
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
                if let Err(error) = self.renegotiate_chat_channel() {
                    // chat 重协商失败时回滚麦克风状态，避免宿主与远端状态分叉。
                    let _ = self.media_backend.set_microphone_capturing(false);
                    return Err(error);
                }
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
                if let Err(error) = self.renegotiate_chat_channel() {
                    // 停麦重协商失败时尽量恢复本地状态，保持行为和已连接会话一致。
                    let _ = self.media_backend.set_microphone_capturing(true);
                    return Err(error);
                }
                self.snapshot.microphone_capturing = false;
                self.snapshot.microphone_paused = true;
                self.event_sink
                    .emit(XbxEngineRuntimeEventDto::ChatStateChanged {
                        capturing: false,
                        paused: true,
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

    // execution spec 只在启动前应用一次：协商参数进 config，显示参数进 media backend/snapshot。
    fn apply_execution_spec(
        &mut self,
        runtime: Option<&XbxEngineRuntimeProjectionDto>,
        render: Option<&XbxEngineRenderProjectionDto>,
    ) -> Result<(), XbxEngineRuntimeError> {
        if let Some(runtime) = runtime {
            if let Some(video_bitrate_kbps) = runtime.max_video_bitrate_kbps {
                self.config.webrtc.negotiation.video_bitrate_kbps = video_bitrate_kbps;
            }
            if let Some(audio_bitrate_kbps) = runtime.max_audio_bitrate_kbps {
                self.config.webrtc.negotiation.audio_bitrate_kbps = audio_bitrate_kbps;
            }
            self.config.webrtc.negotiation.force_mono_audio = runtime.force_mono_audio;
            self.config.webrtc.negotiation.target_resolution_width = runtime.target_video_width;
            self.config.webrtc.negotiation.target_resolution_height = runtime.target_video_height;
            self.config.webrtc.forced_remb_kbps = runtime.forced_remb_kbps;
            self.config.webrtc.adaptive_remb_enabled = runtime.adaptive_remb_enabled;
            self.config.webrtc.bwe_mode = runtime.bwe_mode.clone();
            self.config.webrtc.remb_floor_kbps = runtime.remb_floor_kbps;
            self.config.webrtc.remb_ceiling_kbps = runtime.remb_ceiling_kbps;
            self.config.webrtc.remb_ramp_up_step_kbps = runtime.remb_ramp_up_step_kbps;
            self.config.webrtc.remb_ramp_down_factor = runtime.remb_ramp_down_factor;
            self.config.webrtc.video_pipeline = XbxEngineVideoPipelineRuntimeConfig {
                feedback_interval_ms: runtime.video_pipeline.feedback_interval_ms,
                nack_window_ms: runtime.video_pipeline.nack_window_ms,
                nack_burst_count: runtime.video_pipeline.nack_burst_count,
                nack_max_age_ms: runtime.video_pipeline.nack_max_age_ms,
                nack_retry_interval_ms: runtime.video_pipeline.nack_retry_interval_ms,
                nack_max_retry_count: runtime.video_pipeline.nack_max_retry_count,
                jitter_buffer_min_delay_ms: runtime.video_pipeline.jitter_buffer_min_delay_ms,
                jitter_buffer_max_delay_ms: runtime.video_pipeline.jitter_buffer_max_delay_ms,
                jitter_buffer_max_packets: runtime.video_pipeline.jitter_buffer_max_packets,
                idle_timeout_ms: runtime.video_pipeline.idle_timeout_ms,
                late_frame_drop_threshold_ms: runtime.video_pipeline.late_frame_drop_threshold_ms,
                backlog_drop_threshold_packets: runtime
                    .video_pipeline
                    .backlog_drop_threshold_packets,
            };
            self.config.webrtc.recovery = XbxEngineRecoveryRuntimeConfig {
                first_frame_grace_ms: runtime.recovery.first_frame_grace_ms,
                keyframe_request_stall_ms: runtime.recovery.keyframe_request_stall_ms,
                keyframe_loss_burst_threshold: runtime.recovery.keyframe_loss_burst_threshold,
                decoder_reset_after_keyframe_wait_ms: runtime
                    .recovery
                    .decoder_reset_after_keyframe_wait_ms,
                decoder_reset_request_cooldown_ms: runtime
                    .recovery
                    .decoder_reset_request_cooldown_ms,
                reconnect_stall_ms: runtime.recovery.reconnect_stall_ms,
                stall_recovery_cooldown_ms: runtime.recovery.stall_recovery_cooldown_ms,
            };
            if let Some(codec) = runtime.codec.as_ref() {
                if let Some(profile) = codec.profiles.first() {
                    self.config.webrtc.negotiation.offer_profile = profile.clone();
                }
            }
        }

        if let Some(render) = render {
            let display_state = XbxEngineDisplayStateDto {
                display_options: render.display_options.clone(),
            };
            self.media_backend
                .apply_display_state(display_state.clone())?;
            self.snapshot.display_state = Some(display_state);
        }

        Ok(())
    }

    fn renegotiate_chat_channel(&mut self) -> Result<(), XbxEngineRuntimeError> {
        let local_offer_sdp = self.media_backend.create_offer()?;
        let answer_sdp = Self::extract_offer_response(self.host_bridge.request(
            XbxEngineHostRequestDto::ExchangeOffer {
                session_id: self.require_session_id()?,
                channel: "chat".to_string(),
                sdp: local_offer_sdp.clone(),
                restart: false,
            },
        )?)?;
        self.media_backend
            .apply_remote_description(answer_sdp.clone(), Vec::new())?;
        self.snapshot.last_offer_sdp = Some(local_offer_sdp);
        self.snapshot.last_answer_sdp = Some(answer_sdp);
        Ok(())
    }

    fn negotiate_remote(
        &mut self,
        restart: bool,
        operation_epoch: u64,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.ensure_operation_active(operation_epoch)?;
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
        self.ensure_operation_active(operation_epoch)?;
        let answer_sdp = Self::extract_offer_response(self.host_bridge.request(
            XbxEngineHostRequestDto::ExchangeOffer {
                session_id: self.require_session_id()?,
                channel: "media".to_string(),
                sdp: negotiation.local_offer_sdp.clone(),
                restart,
            },
        )?)?;

        self.emit_phase(XbxEngineRuntimePhaseDto::GatheringIce);
        self.ensure_operation_active(operation_epoch)?;
        self.media_backend
            .apply_remote_description(answer_sdp.clone(), Vec::new())?;
        // 远端 answer 一旦生效，底层 PeerConnection 就可能很快开始连通甚至直接出帧。
        // 这里提前进入 connecting，避免启动主链被后续 ICE polling 卡住时，上层还停留在 exchanging。
        self.emit_phase(XbxEngineRuntimePhaseDto::Connecting);
        self.health.observed_transport_state = XbxEngineTransportStateDto::Connecting;
        self.emit_transport_state(XbxEngineTransportStateDto::Connecting);
        self.sync_runtime_activity_snapshot();
        let remote_candidates = self.exchange_remote_ice_incrementally(
            negotiation.local_candidates.clone(),
            restart,
            operation_epoch,
        )?;
        self.snapshot.last_answer_sdp = Some(answer_sdp);
        self.snapshot.last_remote_candidates = remote_candidates;
        self.record_media_ready(&negotiation);
        self.record_input_status(&negotiation.input_status);
        self.sync_runtime_activity_snapshot();
        Ok(())
    }

    fn exchange_remote_ice_incrementally(
        &mut self,
        initial_local_candidates: Vec<XbxEngineIceCandidateDto>,
        restart: bool,
        operation_epoch: u64,
    ) -> Result<Vec<XbxEngineIceCandidateDto>, XbxEngineRuntimeError> {
        use std::collections::HashSet;
        use std::time::Duration;

        let mut sent_local_candidates = HashSet::<String>::new();
        let mut applied_remote_candidates = HashSet::<String>::new();
        let mut aggregated_remote_candidates = Vec::new();
        let mut final_poll_sent = false;
        let mut remote_end_of_candidates_seen = false;

        loop {
            self.ensure_operation_active(operation_epoch)?;
            let local_candidates = self.collect_unsent_local_candidates(
                &initial_local_candidates,
                &mut sent_local_candidates,
            )?;
            let local_gathering_complete = self.media_backend.local_ice_gathering_complete()?;

            if local_candidates.is_empty() && !local_gathering_complete {
                std::thread::sleep(Duration::from_millis(60));
                continue;
            }

            if local_candidates.is_empty() && local_gathering_complete && final_poll_sent {
                break;
            }

            let request_candidates = if local_candidates.is_empty() {
                final_poll_sent = true;
                Vec::new()
            } else {
                local_candidates
            };

            self.emit_phase(XbxEngineRuntimePhaseDto::ExchangingIce);
            self.ensure_operation_active(operation_epoch)?;
            Self::extract_submit_ice_response(self.host_bridge.request(
                XbxEngineHostRequestDto::SubmitIce {
                    session_id: self.require_session_id()?,
                    candidates: request_candidates,
                    restart,
                },
            )?)?;
            self.ensure_operation_active(operation_epoch)?;
            let remote_candidates = Self::extract_poll_ice_response(self.host_bridge.request(
                XbxEngineHostRequestDto::PollIce {
                    session_id: self.require_session_id()?,
                    restart,
                },
            )?)?;
            self.ensure_operation_active(operation_epoch)?;
            remote_end_of_candidates_seen |= remote_candidates
                .iter()
                .any(|candidate| is_end_of_candidates_marker(&candidate.candidate));

            let next_remote_candidates =
                dedupe_remote_ice_candidates(remote_candidates, &mut applied_remote_candidates);
            if !next_remote_candidates.is_empty() {
                self.media_backend
                    .add_remote_ice_candidates(next_remote_candidates.clone())?;
                aggregated_remote_candidates.extend(next_remote_candidates);
            }

            self.sync_runtime_activity_snapshot();

            // 一旦已经拿到 end-of-candidates，或者底层 transport 已经连上，就不再让启动主链继续被 polling 阻塞。
            if local_gathering_complete
                && (remote_end_of_candidates_seen || self.health.connected_at_ms.is_some())
            {
                break;
            }
        }

        Ok(aggregated_remote_candidates)
    }

    fn ensure_operation_active(&self, operation_epoch: u64) -> Result<(), XbxEngineRuntimeError> {
        if self.host_bridge.current_cancellation_epoch() != operation_epoch {
            return Err(XbxEngineRuntimeError::new("xbxEngineRuntimeCancelled"));
        }
        Ok(())
    }

    fn collect_unsent_local_candidates(
        &self,
        initial_local_candidates: &[XbxEngineIceCandidateDto],
        sent_local_candidates: &mut std::collections::HashSet<String>,
    ) -> Result<Vec<XbxEngineIceCandidateDto>, XbxEngineRuntimeError> {
        let mut pending = Vec::new();
        for candidate in initial_local_candidates
            .iter()
            .cloned()
            .chain(self.media_backend.local_candidates_snapshot()?.into_iter())
        {
            let key = ice_candidate_dedupe_key(&candidate);
            if sent_local_candidates.insert(key) {
                pending.push(candidate);
            }
        }
        Ok(pending)
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
        if let Some(viewport) = self.snapshot.viewport.as_ref() {
            if let Err(error) = self
                .host_bridge
                .attach_viewport(viewport, Some(&negotiation.surface_id))
            {
                self.emit_error("attachViewportFailed", error.to_string());
            }
        }
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

    fn present_latest_render_frame(&mut self) {
        let Some(viewport) = self.snapshot.viewport.clone() else {
            return;
        };
        let frame = match self.media_backend.take_latest_render_frame() {
            Ok(frame) => frame,
            Err(error) => {
                self.emit_error("takeLatestRenderFrameFailed", error.to_string());
                return;
            }
        };
        let Some(frame) = frame else {
            return;
        };
        if let Err(error) =
            self.host_bridge
                .present_frame(&viewport, self.snapshot.surface_id.as_deref(), &frame)
        {
            self.emit_error("presentFrameFailed", error.to_string());
            return;
        }
        self.snapshot.video_size = Some((frame.width, frame.height));
        self.snapshot.frame_rendered_time_ms = Some(frame.rendered_at_ms);
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

    fn extract_submit_ice_response(
        response: XbxEngineHostResponseDto,
    ) -> Result<(), XbxEngineRuntimeError> {
        match response {
            XbxEngineHostResponseDto::IceSubmitted => Ok(()),
            _ => Err(XbxEngineRuntimeError::new(
                "xbxEngineHostBridgeInvalidSubmitIceResponse",
            )),
        }
    }

    fn extract_poll_ice_response(
        response: XbxEngineHostResponseDto,
    ) -> Result<Vec<XbxEngineIceCandidateDto>, XbxEngineRuntimeError> {
        match response {
            XbxEngineHostResponseDto::IcePolled { candidates } => Ok(candidates),
            _ => Err(XbxEngineRuntimeError::new(
                "xbxEngineHostBridgeInvalidPollIceResponse",
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

    fn sync_runtime_activity_snapshot(&mut self) {
        let Ok(runtime_stats) = self.media_backend.snapshot_runtime_stats() else {
            return;
        };
        self.sync_transport_state(&runtime_stats);
        self.sync_video_packet_stats(&runtime_stats);
        self.sync_video_frame_stats(&runtime_stats);
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

    fn maybe_recover_media_stall(&mut self, stats: &XbxEngineMediaRuntimeStats) {
        let now = now_ms_f64();
        let recovery_signals = self.build_recovery_signals(now, stats);
        let keyframe_request_stall_ms =
            self.config.webrtc.recovery.keyframe_request_stall_ms as f64;
        let packet_age_ms = recovery_signals
            .transport
            .latest_video_packet_arrival_at_ms
            .map(|at_ms| (now - at_ms).max(0.0));
        let decode_age_ms = recovery_signals
            .media
            .latest_frame_decoded_at_ms
            .map(|at_ms| (now - at_ms).max(0.0));
        let present_age_ms = recovery_signals
            .media
            .latest_frame_rendered_at_ms
            .map(|at_ms| (now - at_ms).max(0.0));
        let stall_candidate = recovery_signals.decode_render.decoder_stalled == Some(true)
            || recovery_signals.decode_render.render_stalled == Some(true)
            || decode_age_ms.is_some_and(|age| age >= keyframe_request_stall_ms)
            || present_age_ms.is_some_and(|age| age >= keyframe_request_stall_ms)
            || packet_age_ms.is_some_and(|age| age >= keyframe_request_stall_ms);
        if !self
            .health
            .update_stall_candidate(now, stall_candidate, STALL_SIGNAL_STABILITY_MS)
        {
            return;
        }
        let next_action = self.health.next_recovery_action_with_signals_and_config(
            now,
            self.state == XbxEngineRuntimeState::Running,
            recovery_signals,
            &self.config.webrtc.recovery,
        );
        match next_action {
            Some(XbxEngineRecoveryAction::RequestVideoKeyframe) => {
                if let Err(error) = self.media_backend.request_video_keyframe() {
                    self.emit_error("requestVideoKeyframeFailed", error.to_string());
                } else {
                    self.health.mark_keyframe_requested(now);
                    self.snapshot.recovery_keyframe_request_count = self
                        .snapshot
                        .recovery_keyframe_request_count
                        .saturating_add(1);
                    self.snapshot.last_recovery_action = Some("keyframe".to_string());
                    self.snapshot.last_recovery_action_at_ms = Some(now);
                    self.snapshot.last_recovery_reason = Some("mediaStall".to_string());
                }
            }
            Some(XbxEngineRecoveryAction::RequestDecoderReset) => {
                if let Err(error) = self.media_backend.request_decoder_reset() {
                    self.emit_error("requestDecoderResetFailed", error.to_string());
                } else {
                    self.health.mark_decoder_reset_requested(now);
                    self.snapshot.recovery_decoder_reset_count =
                        self.snapshot.recovery_decoder_reset_count.saturating_add(1);
                    self.snapshot.last_recovery_action = Some("decoderReset".to_string());
                    self.snapshot.last_recovery_action_at_ms = Some(now);
                    self.snapshot.last_recovery_reason = Some("mediaStall".to_string());
                }
            }
            Some(XbxEngineRecoveryAction::RequestReconnect(reason)) => {
                if let Err(error) = self.request_reconnect(reason) {
                    if !error.is_cancelled() {
                        self.emit_error("recoverMediaStallFailed", error.to_string());
                    }
                }
            }
            None => {}
        }
    }

    fn build_recovery_signals(
        &self,
        now_ms: f64,
        stats: &XbxEngineMediaRuntimeStats,
    ) -> XbxEngineRecoverySignals {
        let latest_video_packet_arrival_at_ms = stats
            .latest_video_packet_arrival_time_ms
            .or(self.health.last_video_packet_arrival_at_ms);
        let latest_frame_rendered_at_ms = stats
            .latest_video_present_time_ms
            .or(stats
                .latest_video_frame
                .as_ref()
                .map(|frame| frame.rendered_at_ms))
            .or(self.health.last_frame_rendered_at_ms);
        let latest_decode_ok_at_ms = stats.latest_video_decode_ok_time_ms.or(stats
            .latest_video_frame
            .as_ref()
            .map(|frame| frame.rendered_at_ms));
        let packet_age_ms =
            latest_video_packet_arrival_at_ms.map(|at_ms| (now_ms - at_ms).max(0.0));
        let frame_age_ms = latest_frame_rendered_at_ms.map(|at_ms| (now_ms - at_ms).max(0.0));
        let decode_age_ms = latest_decode_ok_at_ms.map(|at_ms| (now_ms - at_ms).max(0.0));
        let keyframe_request_stall_ms =
            self.config.webrtc.recovery.keyframe_request_stall_ms as f64;
        let inferred_decoder_stalled = match (frame_age_ms, packet_age_ms) {
            (Some(frame_age_ms), Some(packet_age_ms)) => {
                packet_age_ms <= keyframe_request_stall_ms
                    && frame_age_ms >= keyframe_request_stall_ms
            }
            _ => false,
        };
        let decoder_stalled =
            stats
                .video_decoder_stalled
                .unwrap_or_else(|| match (decode_age_ms, packet_age_ms) {
                    (Some(decode_age_ms), Some(packet_age_ms)) => {
                        packet_age_ms <= keyframe_request_stall_ms
                            && decode_age_ms >= keyframe_request_stall_ms
                    }
                    _ => inferred_decoder_stalled,
                });
        let renderer_stalled = stats.video_renderer_stalled.unwrap_or(false);

        XbxEngineRecoverySignals {
            transport: XbxEngineTransportSignal {
                transport_connected: stats.transport_state == XbxEngineTransportStateDto::Connected,
                connected_at_ms: self.health.connected_at_ms,
                latest_video_packet_arrival_at_ms,
                latest_twcc_feedback_at_ms: stats
                    .latest_video_twcc_observation
                    .as_ref()
                    .map(|observation| observation.observed_at_ms),
                audio_stream_alive: stats
                    .inbound_audio_bitrate_kbps
                    .is_some_and(|bitrate_kbps| bitrate_kbps >= 16.0),
            },
            media: XbxEngineMediaSignal {
                latest_frame_decoded_at_ms: latest_decode_ok_at_ms,
                latest_frame_rendered_at_ms,
            },
            decode_render: XbxEngineDecodeRenderSignal {
                decoder_stalled: Some(decoder_stalled),
                render_stalled: Some(renderer_stalled),
                allow_decoder_reset: latest_frame_rendered_at_ms.is_some(),
            },
        }
    }
}

fn dedupe_remote_ice_candidates(
    candidates: Vec<XbxEngineIceCandidateDto>,
    applied_remote_candidates: &mut std::collections::HashSet<String>,
) -> Vec<XbxEngineIceCandidateDto> {
    candidates
        .into_iter()
        .filter(|candidate| applied_remote_candidates.insert(ice_candidate_dedupe_key(candidate)))
        .collect()
}

fn ice_candidate_dedupe_key(candidate: &XbxEngineIceCandidateDto) -> String {
    format!(
        "{}|{}|{}",
        candidate.candidate,
        candidate.sdp_mid.as_deref().unwrap_or_default(),
        candidate.sdp_m_line_index.unwrap_or_default()
    )
}

fn is_end_of_candidates_marker(candidate: &str) -> bool {
    matches!(
        candidate.trim(),
        "a=end-of-candidates" | "end-of-candidates"
    )
}

fn now_ms_f64() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
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
        XbxEngineRuntimeProjectionDto, XbxEngineRuntimeRecoveryDto,
        XbxEngineRuntimeVideoPipelineDto, XbxEngineSessionDto, XbxEngineTargetTypeDto,
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
        cancellation_epoch: Rc<Cell<u64>>,
        cancel_after_request_kind: Rc<RefCell<Option<&'static str>>>,
    }

    impl TestHostBridge {
        fn new(requests: Rc<RefCell<Vec<XbxEngineHostRequestDto>>>) -> Self {
            Self {
                requests,
                fail_request_kind: Rc::new(RefCell::new(None)),
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
                cancellation_epoch: Rc::new(Cell::new(0)),
                cancel_after_request_kind: Rc::new(RefCell::new(None)),
            }
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

    #[derive(Clone)]
    struct ScriptedMediaBackend {
        negotiation: XbxEngineMediaNegotiation,
        runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
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

        fn set_microphone_capturing(
            &mut self,
            capturing: bool,
        ) -> Result<(), XbxEngineRuntimeError> {
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
            .start(session(), viewport(), 0.75, None, None, None)
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
            .start(session(), viewport(), 0.3, None, None, None)
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
            .start(session(), viewport(), 0.75, None, None, None)
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

        *runtime_stats.lock().expect("lock runtime stats") = XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            latest_video_packet_arrival_time_ms: Some(now_ms - 20.0),
            inbound_video_packet_count_total: 200,
            ..Default::default()
        };

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

        *runtime_stats.lock().expect("lock runtime stats") = XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            latest_video_packet_arrival_time_ms: Some(now_ms - 20.0),
            latest_video_present_time_ms: Some(now_ms - 100.0),
            latest_video_decode_ok_time_ms: Some(now_ms - 100.0),
            video_decoder_stalled: Some(true),
            video_renderer_stalled: Some(false),
            inbound_video_packet_count_total: 300,
            ..Default::default()
        };

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
        *runtime_stats.lock().expect("lock runtime stats") = XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            latest_video_packet_arrival_time_ms: Some(now_ms - 20.0),
            latest_video_present_time_ms: Some(now_ms - 5_000.0),
            latest_video_decode_ok_time_ms: Some(now_ms - 5_000.0),
            video_decoder_stalled: Some(true),
            video_renderer_stalled: Some(false),
            inbound_video_packet_count_total: 500,
            ..Default::default()
        };
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
    fn runtime_consumes_transport_reconnect_candidate_once() {
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
                latest_video_escalation_observation: Some(
                    crate::XbxEngineVideoEscalationObservation {
                        observation_id: 42,
                        reason: "transportExpiredDeadline".to_string(),
                        action: "requestReconnectCandidate".to_string(),
                        observed_at_ms: now_ms,
                    },
                ),
                ..Default::default()
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
