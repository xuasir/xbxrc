use serde::{Deserialize, Serialize};
use xbox_streaming::{
    QueueDetails as MonitorQueueDetails, QueueSnapshot as MonitorQueueSnapshot,
    RenderDisplayOptionsProjection as DomainRenderDisplayOptionsProjection,
    RenderPlanProjection as DomainRenderPlanProjection,
    RuntimeCodecProjection as DomainRuntimeCodecProjection,
    RuntimePlanProjection as DomainRuntimePlanProjection,
    SessionErrorDetails as MonitorErrorDetails, SessionFlowSnapshot,
    SessionPhase as DomainSessionPhase, SessionProgressSnapshot as DomainSessionProgressSnapshot,
    SessionRuntimeBinding, SessionRuntimeSnapshot,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum StreamingTargetType {
    Home,
    Cloud,
}

impl StreamingTargetType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Home => "home",
            Self::Cloud => "cloud",
        }
    }

    pub fn from_value(value: &str) -> Self {
        if value == "home" {
            return Self::Home;
        }
        Self::Cloud
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamingErrorDetails {
    pub code: Option<serde_json::Value>,
    pub message: Option<String>,
}

impl StreamingErrorDetails {
    /// 转换为 monitor 域错误详情，供状态机输入与回写使用。
    pub fn to_monitor_error_details(&self) -> MonitorErrorDetails {
        MonitorErrorDetails {
            code: self.code.clone(),
            message: self.message.clone(),
        }
    }
}

impl From<MonitorErrorDetails> for StreamingErrorDetails {
    fn from(details: MonitorErrorDetails) -> Self {
        Self {
            code: details.code,
            message: details.message,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct StreamingQueueDetails {
    pub estimated_total_wait_time_in_seconds: Option<u64>,
    pub estimated_allocation_time_in_seconds: Option<u64>,
    pub estimated_provisioning_time_in_seconds: Option<u64>,
}

impl StreamingQueueDetails {
    /// 转换为 monitor 域排队详情，避免 service 层重复字段搬运。
    pub fn to_monitor_queue_details(&self) -> MonitorQueueDetails {
        MonitorQueueDetails {
            estimated_total_wait_time_in_seconds: self.estimated_total_wait_time_in_seconds,
            estimated_allocation_time_in_seconds: self.estimated_allocation_time_in_seconds,
            estimated_provisioning_time_in_seconds: self.estimated_provisioning_time_in_seconds,
        }
    }
}

impl From<MonitorQueueDetails> for StreamingQueueDetails {
    fn from(details: MonitorQueueDetails) -> Self {
        Self {
            estimated_total_wait_time_in_seconds: details.estimated_total_wait_time_in_seconds,
            estimated_allocation_time_in_seconds: details.estimated_allocation_time_in_seconds,
            estimated_provisioning_time_in_seconds: details.estimated_provisioning_time_in_seconds,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamingQueueSnapshot {
    pub details: StreamingQueueDetails,
}

impl StreamingQueueSnapshot {
    /// 转换为 monitor 域排队快照。
    pub fn to_monitor_queue_snapshot(&self) -> MonitorQueueSnapshot {
        MonitorQueueSnapshot {
            details: self.details.to_monitor_queue_details(),
        }
    }
}

impl From<MonitorQueueSnapshot> for StreamingQueueSnapshot {
    fn from(snapshot: MonitorQueueSnapshot) -> Self {
        Self {
            details: snapshot.details.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamingAnswerPayload {
    pub sdp: String,
    pub message_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamingIceCandidate {
    pub candidate: String,
    pub sdp_m_line_index: Option<u32>,
    pub sdp_mid: Option<String>,
    pub username_fragment: Option<String>,
    pub message_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StreamingTurnServerConfig {
    pub url: String,
    pub username: String,
    pub credential: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum StreamingRuntimeMode {
    WebRtcDirect,
    RustOwned,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum StreamingRuntimeOwner {
    Browser,
    Sidecar,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StreamingRuntimeCodecPreference {
    pub mime_type: String,
    pub profiles: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StreamingDisplayOptionsValue {
    pub sharpness: i16,
    pub saturation: i16,
    pub contrast: i16,
    pub brightness: i16,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StreamingRuntimeProjection {
    pub mode: StreamingRuntimeMode,
    pub transport: StreamingRuntimeOwner,
    pub decode: StreamingRuntimeOwner,
    pub render: StreamingRuntimeOwner,
    pub input: StreamingRuntimeOwner,
    pub microphone: StreamingRuntimeOwner,
    pub turn_server: Option<StreamingTurnServerConfig>,
    pub codec: Option<StreamingRuntimeCodecPreference>,
    pub max_video_bitrate_kbps: Option<u32>,
    pub max_audio_bitrate_kbps: Option<u32>,
    pub force_mono_audio: bool,
    pub polling_rate_hz: u16,
    pub vibration: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StreamingRenderProjection {
    pub enable_audio_control: bool,
    pub video_format: Option<String>,
    pub display_options: StreamingDisplayOptionsValue,
}

impl From<DomainRuntimePlanProjection> for StreamingRuntimeProjection {
    fn from(projection: DomainRuntimePlanProjection) -> Self {
        Self {
            mode: match projection.mode {
                xbox_streaming::RuntimeMode::WebRtcDirect => StreamingRuntimeMode::WebRtcDirect,
                xbox_streaming::RuntimeMode::RustOwned => StreamingRuntimeMode::RustOwned,
            },
            transport: map_runtime_owner(projection.transport),
            decode: map_runtime_owner(projection.decode),
            render: map_runtime_owner(projection.render),
            input: map_runtime_owner(projection.input),
            microphone: map_runtime_owner(projection.microphone),
            turn_server: projection.turn_server.map(Into::into),
            codec: projection.codec.map(Into::into),
            max_video_bitrate_kbps: projection.max_video_bitrate_kbps,
            max_audio_bitrate_kbps: projection.max_audio_bitrate_kbps,
            force_mono_audio: projection.force_mono_audio,
            polling_rate_hz: projection.polling_rate_hz,
            vibration: projection.vibration,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamingSessionExecutionSnapshot {
    pub session: StreamingSessionSnapshot,
    pub runtime: StreamingRuntimeProjection,
    pub render: StreamingRenderProjection,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum StreamingSessionPhase {
    Creating,
    WaitingSessionReady,
    RuntimeStarting,
    SessionReady,
    Recovering,
    Closing,
    Closed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamingSessionProgressSnapshot {
    pub session_id: String,
    pub phase: StreamingSessionPhase,
    pub status_text_key: String,
    pub retry_count: u8,
    pub queue_seconds: Option<u64>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamingStartSessionResult {
    pub execution: StreamingSessionExecutionSnapshot,
    pub progress: StreamingSessionProgressSnapshot,
}

impl StreamingSessionProgressSnapshot {
    /// M1 兼容兜底：progress 未命中时，按会话快照推导一个最小进度对象。
    pub fn from_session_snapshot(session: &StreamingSessionSnapshot) -> Self {
        let phase = match session.player_state.as_str() {
            "started" => StreamingSessionPhase::SessionReady,
            "failed" => StreamingSessionPhase::Failed,
            "queued" => StreamingSessionPhase::WaitingSessionReady,
            "pending" => StreamingSessionPhase::WaitingSessionReady,
            _ => StreamingSessionPhase::Creating,
        };

        let queue_seconds = session
            .queue
            .as_ref()
            .and_then(|queue| queue.details.estimated_total_wait_time_in_seconds);

        Self {
            session_id: session.id.clone(),
            phase: phase.clone(),
            status_text_key: match phase {
                StreamingSessionPhase::Creating => "streamPage.status.creatingSession".to_string(),
                StreamingSessionPhase::WaitingSessionReady => {
                    "streamPage.status.waitingSession".to_string()
                }
                StreamingSessionPhase::RuntimeStarting => {
                    "streamPage.status.startingPlayer".to_string()
                }
                StreamingSessionPhase::SessionReady => {
                    "streamPage.status.startingPlayer".to_string()
                }
                StreamingSessionPhase::Recovering => "streamPage.status.reconnecting".to_string(),
                StreamingSessionPhase::Closing => "streamPage.status.disconnecting".to_string(),
                StreamingSessionPhase::Closed => "streamPage.status.disconnected".to_string(),
                StreamingSessionPhase::Failed => "streamPage.errors.startFailed".to_string(),
            },
            retry_count: 0,
            queue_seconds,
            error_code: session.error_details.as_ref().and_then(|details| {
                details.code.as_ref().map(|code| match code {
                    serde_json::Value::String(raw) => raw.clone(),
                    _ => code.to_string(),
                })
            }),
            error_message: session
                .error_details
                .as_ref()
                .and_then(|details| details.message.clone()),
        }
    }
}

/// 对外 RPC DTO：只保留 UI 需要的字段，严禁持有敏感凭证或内部策略。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamingSessionSnapshot {
    pub id: String,
    pub target_id: String,
    pub path: String,
    pub target_type: String,
    pub stream_state: Option<String>,
    pub player_state: String,
    pub queue: Option<StreamingQueueSnapshot>,
    pub error_details: Option<StreamingErrorDetails>,
}

impl SessionRuntimeBinding for StreamingSessionSnapshot {
    fn runtime_snapshot(&self) -> SessionRuntimeSnapshot {
        SessionRuntimeSnapshot {
            stream_state: self.stream_state.clone(),
            player_state: self.player_state.clone(),
            queue: self
                .queue
                .as_ref()
                .map(StreamingQueueSnapshot::to_monitor_queue_snapshot),
            error_details: self
                .error_details
                .as_ref()
                .map(StreamingErrorDetails::to_monitor_error_details),
        }
    }

    fn replace_runtime_snapshot(&mut self, runtime: SessionRuntimeSnapshot) {
        self.stream_state = runtime.stream_state;
        self.player_state = runtime.player_state;
        self.queue = runtime.queue.map(Into::into);
        self.error_details = runtime.error_details.map(Into::into);
    }
}

impl SessionFlowSnapshot for StreamingSessionSnapshot {
    fn new_pending(
        session_id: String,
        session_path: String,
        target_id: String,
        target_type: String,
    ) -> Self {
        Self {
            id: session_id,
            target_id,
            path: session_path,
            target_type,
            stream_state: None,
            player_state: "pending".to_string(),
            queue: None,
            error_details: None,
        }
    }

    fn session_id(&self) -> &str {
        &self.id
    }

    fn session_path(&self) -> &str {
        &self.path
    }

    fn target_id(&self) -> &str {
        &self.target_id
    }

    fn target_type(&self) -> &str {
        &self.target_type
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamingStartSessionParams {
    pub target_type: String,
    pub target_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamingGetSessionProgressParams {
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamingCloseSessionParams {
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamingExchangeOfferParams {
    pub session_id: String,
    pub sdp: String,
    pub channel: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamingExchangeOfferResult {
    pub answer: StreamingAnswerPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamingExchangeIceParams {
    pub session_id: String,
    pub candidate: Vec<StreamingIceCandidate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamingExchangeIceResult {
    pub candidates: Vec<StreamingIceCandidate>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct StreamingListActiveSessionsParams {
    pub target_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamingListActiveSessionsResult {
    pub sessions: Vec<StreamingSessionSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamingDecideRecoveryParams {
    pub session_id: String,
    pub fact: StreamingRuntimeFact,
    pub is_closing: bool,
}

/// runtime 上报到 session 的运行事实，供恢复策略统一判定。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum StreamingRuntimeFact {
    TransportConnectionState {
        #[serde(rename = "connectionState")]
        connection_state: String,
    },
    MediaHealth {
        #[serde(rename = "connectionState")]
        connection_state: String,
        #[serde(rename = "connectedElapsedMs")]
        connected_elapsed_ms: u64,
        #[serde(rename = "inactivityElapsedMs")]
        inactivity_elapsed_ms: u64,
    },
    MediaStalled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamingDecideRecoveryResult {
    pub should_reconnect: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamingSessionPathPayload {
    pub session_path: String,
}

#[derive(Debug, Clone)]
pub struct StreamingConfigSnapshot {
    pub resolution: i64,
    pub preferred_game_language: String,
    pub ipv6: bool,
    pub force_region_ip: String,
    pub xhome_bitrate_mode: String,
    pub xhome_bitrate: i64,
    pub xcloud_bitrate_mode: String,
    pub xcloud_bitrate: i64,
    pub audio_bitrate_mode: String,
    pub audio_bitrate: i64,
    pub codec: String,
    pub polling_rate: i64,
    pub vibration: bool,
    pub stream_runtime_mode: String,
    pub power_on: bool,
    pub server_url: String,
    pub server_username: String,
    pub server_credential: String,
    pub xhome_turn_fallback: bool,
    pub enable_audio_control: bool,
    pub video_format: String,
    pub display_options: StreamingDisplayOptionsValue,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamingCloseSessionResult {
    pub closed: bool,
}

fn map_runtime_owner(owner: xbox_streaming::Owner) -> StreamingRuntimeOwner {
    match owner {
        xbox_streaming::Owner::Browser => StreamingRuntimeOwner::Browser,
        xbox_streaming::Owner::Sidecar => StreamingRuntimeOwner::Sidecar,
    }
}

impl From<xbox_streaming::TurnServer> for StreamingTurnServerConfig {
    fn from(turn_server: xbox_streaming::TurnServer) -> Self {
        Self {
            url: turn_server.url,
            username: turn_server.username,
            credential: turn_server.credential,
        }
    }
}

impl From<DomainRuntimeCodecProjection> for StreamingRuntimeCodecPreference {
    fn from(codec: DomainRuntimeCodecProjection) -> Self {
        Self {
            mime_type: codec.mime_type,
            profiles: codec.profiles,
        }
    }
}

impl From<DomainRenderPlanProjection> for StreamingRenderProjection {
    fn from(projection: DomainRenderPlanProjection) -> Self {
        Self {
            enable_audio_control: projection.enable_audio_control,
            video_format: projection.video_format,
            display_options: projection.display_options.into(),
        }
    }
}

impl From<DomainRenderDisplayOptionsProjection> for StreamingDisplayOptionsValue {
    fn from(options: DomainRenderDisplayOptionsProjection) -> Self {
        Self {
            sharpness: options.sharpness,
            saturation: options.saturation,
            contrast: options.contrast,
            brightness: options.brightness,
        }
    }
}

impl From<DomainSessionProgressSnapshot> for StreamingSessionProgressSnapshot {
    fn from(progress: DomainSessionProgressSnapshot) -> Self {
        Self {
            session_id: progress.session_id,
            phase: match progress.phase {
                DomainSessionPhase::Creating => StreamingSessionPhase::Creating,
                DomainSessionPhase::WaitingSessionReady => {
                    StreamingSessionPhase::WaitingSessionReady
                }
                DomainSessionPhase::RuntimeStarting => StreamingSessionPhase::RuntimeStarting,
                DomainSessionPhase::SessionReady => StreamingSessionPhase::SessionReady,
                DomainSessionPhase::Recovering => StreamingSessionPhase::Recovering,
                DomainSessionPhase::Closing => StreamingSessionPhase::Closing,
                DomainSessionPhase::Closed => StreamingSessionPhase::Closed,
                DomainSessionPhase::Failed => StreamingSessionPhase::Failed,
            },
            status_text_key: progress.status_text_key,
            retry_count: progress.retry_count,
            queue_seconds: progress.queue_seconds,
            error_code: progress.error_code,
            error_message: progress.error_message,
        }
    }
}
