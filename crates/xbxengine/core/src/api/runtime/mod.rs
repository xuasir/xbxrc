use std::error::Error;
use std::fmt::{Display, Formatter};

use ohmygamepad_protocol::{OhMyGamepadRumbleRequestDto, OhMyGamepadRumbleTargetDto};
use xbxengine_protocol::{
    XbxEngineDisplayStateDto, XbxEngineHostRequestDto, XbxEngineHostResponseDto,
    XbxEngineIceCandidateDto, XbxEngineInputEventDto, XbxEngineRuntimeEventDto,
    XbxEngineSessionDto, XbxEngineStatsDto, XbxEngineVideoTrackStatusDto, XbxEngineViewportDto,
};

use crate::{
    build_xbxengine_stats, PlaceholderXbxEngineMediaBackend, XbxEngineHostVideoPresentMetrics,
    XbxEngineMediaBackend, XbxEngineRecoveryRuntimeConfig, XbxEngineRenderFrame,
    XbxEngineRuntimeHealth,
};

mod lifecycle;
mod sync;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum XbxEngineRuntimeState {
    Idle,
    Starting,
    Running,
    Reconnecting,
    Stopped,
}

/// 重连带 `restart=true` 的触发来源：验收“仅 policy 自动重连 vs 显式/兜底”去双轨。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XbxEngineReconnectTriggerSource {
    /// transport session policy 产出并消费的 `RequestReconnectCandidate`
    Policy,
    /// 控制面 `RequestReconnect` 等显式调用
    Runtime,
    /// 非 rust-owned 模式下的 health 超时重连等历史路径
    Other,
}

impl XbxEngineReconnectTriggerSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Policy => "policy",
            Self::Runtime => "runtime",
            Self::Other => "other",
        }
    }
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
    pub prefer_ipv6: bool,
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
            prefer_ipv6: false,
            offer_profile: "macos".to_string(),
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
    pub latest_video_track_status: Option<XbxEngineVideoTrackStatusDto>,
    pub recovery_keyframe_request_count: u64,
    pub recovery_decoder_reset_count: u64,
    pub recovery_reconnect_count: u64,
    pub last_recovery_action: Option<String>,
    pub last_recovery_action_at_ms: Option<f64>,
    pub last_recovery_reason: Option<String>,
    pub reconnect_trigger_source: Option<String>,
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

    fn play_gamepad_rumble(
        &mut self,
        _request: OhMyGamepadRumbleRequestDto,
    ) -> Result<(), XbxEngineRuntimeError> {
        Ok(())
    }

    fn stop_gamepad_rumble(
        &mut self,
        _target: OhMyGamepadRumbleTargetDto,
    ) -> Result<(), XbxEngineRuntimeError> {
        Ok(())
    }

    fn submit_gamepad_rumble_request(
        &mut self,
        request: OhMyGamepadRumbleRequestDto,
    ) -> Result<(), XbxEngineRuntimeError> {
        if crate::api::runtime::lifecycle::is_stop_gamepad_rumble_request(&request.effect) {
            self.stop_gamepad_rumble(request.target)
        } else {
            self.play_gamepad_rumble(request)
        }
    }

    fn clear_pending_gamepad_rumble_requests(&mut self) -> Result<(), XbxEngineRuntimeError> {
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
 * runtime 继续作为 engine 顶层编排入口，
 * 但把生命周期、同步和 watchdog 逻辑拆到粗粒度子模块，避免继续长成单文件 God object。
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

    pub fn update_host_video_timing(
        &mut self,
        host_display_interval_ms: Option<f64>,
        host_frame_age_budget_ms: Option<f64>,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.media_backend
            .update_host_video_timing(host_display_interval_ms, host_frame_age_budget_ms)
    }

    pub fn update_host_video_present_metrics(
        &mut self,
        metrics: XbxEngineHostVideoPresentMetrics,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.media_backend
            .update_host_video_present_metrics(metrics)
    }

    pub fn record_host_video_frame_drop(
        &mut self,
        event: crate::XbxEngineHostVideoFrameDropEvent,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.media_backend.record_host_video_frame_drop(event)
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

    pub fn record_video_frame_drop(
        &mut self,
        observation: crate::XbxEngineVideoFrameDropObservation,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.media_backend.record_video_frame_drop(observation)
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
#[path = "mod.test.rs"]
mod tests;
