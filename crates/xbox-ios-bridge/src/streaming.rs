use crate::cloud_access::{
    find_home_host_facts, load_scoped_stream_access, load_stream_access, replace_home_host_facts,
    HomeHostFacts, StreamingAccessContext,
};
use crate::data::extract_home_host_facts;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;
use tokio::sync::Mutex;
use xbox_auth_flow::{AuthFlow, TransferTokenInput};
use xbox_streaming::runtime::{RuntimeCapabilities, TurnContext};
use xbox_streaming::{
    compile, parse_resolution_preference, BitratePreference, CodecPreference, CompilerInput,
    Config, Context, IceCandidate, Plan, RemotePlayContext, RuntimeLaunchState, SessionFlowError,
    SessionFlowProvider, SessionFlowService, SessionFlowSnapshot, SessionPhase as FlowSessionPhase,
    SessionProgressSnapshot, SessionRuntimeBinding, SessionRuntimeSnapshot, SessionStartupScope,
    Target, TurnServer, TurnSource,
};
use xbox_streaming_protocol::{
    build_xbox_stream_control_authorization_payload,
    build_xbox_stream_control_gamepad_changed_payload,
    build_xbox_stream_control_video_keyframe_requested_payload,
    build_xbox_stream_input_metadata_bootstrap_packet, build_xbox_stream_message_handshake_payload,
    build_xbox_stream_post_handshake_payloads, is_xbox_stream_message_handshake_ack,
    XBOX_STREAM_DATA_CHANNEL_PROFILES,
};
use xbox_webapi::{ConsoleApi, SmartglassApi};

mod credentials;
mod frame_feedback;
mod home_context;
mod touch_controls;
use credentials::SessionCredentials;
pub use credentials::XboxStreamingAuthUpdate;
mod startup;
pub use frame_feedback::{stream_input_frame_feedback_payload, XboxStreamFrameFeedback};
use startup::IosStartupObserver;
pub use startup::{
    create_stream_startup, XboxStartupFailure, XboxStartupFailureKind, XboxStartupProgress,
    XboxStreamStartup,
};

const OFFICIAL_XBOX_STUN_URL: &str = "stun:relay.communication.microsoft.com:3478";

#[derive(Debug, Clone, thiserror::Error, uniffi::Error)]
pub enum XboxStreamingError {
    #[error("Streaming startup failed: {failure:?}")]
    Startup { failure: XboxStartupFailure },
    #[error("Invalid streaming argument: {0}")]
    InvalidArgument(String),
    #[error("Streaming session state is invalid: {0}")]
    InvalidState(String),
    #[error("Streaming request was cancelled (generation {0})")]
    Cancelled(u64),
    #[error("Streaming authentication failed: {0}")]
    Authentication(String),
    #[error("Streaming network request failed: {0}")]
    Network(String),
    #[error("Streaming HTTP request failed ({0}): {1}")]
    Http(u16, String),
    #[error("Streaming response could not be parsed: {0}")]
    Parse(String),
    #[error("Remote streaming session failed: {0}")]
    Remote(String),
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct XboxIceCandidate {
    pub source: Option<String>,
    pub candidate: String,
    pub sdp_m_line_index: Option<u32>,
    pub sdp_mid: Option<String>,
    pub username_fragment: Option<String>,
    pub message_type: Option<String>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct XboxRemoteIceBatch {
    pub candidates: Vec<XboxIceCandidate>,
    pub end_of_candidates: bool,
    pub configuration_pending: bool,
}

/// 可直接映射到 libwebrtc `RTCIceServer` 的稳定配置。
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct XboxIceServer {
    pub urls: Vec<String>,
    pub username: Option<String>,
    pub credential: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct XboxHostAddress {
    pub ip: String,
    pub port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct XboxStreamDataChannelProfile {
    pub label: String,
    pub ordered: bool,
    pub protocol_name: String,
}

#[uniffi::export]
pub fn stream_data_channel_profiles() -> Vec<XboxStreamDataChannelProfile> {
    XBOX_STREAM_DATA_CHANNEL_PROFILES
        .iter()
        .map(|profile| XboxStreamDataChannelProfile {
            label: profile.label.to_string(),
            ordered: profile.ordered,
            protocol_name: profile.protocol_name.to_string(),
        })
        .collect()
}

#[uniffi::export]
pub fn stream_message_handshake_payload() -> String {
    build_xbox_stream_message_handshake_payload()
}

#[uniffi::export]
pub fn stream_post_handshake_payloads(width: u32, height: u32) -> Vec<String> {
    build_xbox_stream_post_handshake_payloads(width, height)
}

#[uniffi::export]
pub fn stream_control_bootstrap_payloads() -> Vec<String> {
    vec![
        build_xbox_stream_control_authorization_payload(),
        build_xbox_stream_control_gamepad_changed_payload(false),
        build_xbox_stream_control_video_keyframe_requested_payload(),
    ]
}

#[uniffi::export]
pub fn stream_control_gamepad_added_payload() -> String {
    build_xbox_stream_control_gamepad_changed_payload(true)
}

#[uniffi::export]
pub fn stream_control_gamepad_changed_payload(added: bool) -> String {
    build_xbox_stream_control_gamepad_changed_payload(added)
}

#[uniffi::export]
pub fn stream_input_metadata_bootstrap_payload() -> Vec<u8> {
    build_xbox_stream_input_metadata_bootstrap_packet(now_ms_f64(), 64)
}

fn now_ms_f64() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64() * 1_000.0)
        .unwrap_or_default()
}

#[uniffi::export]
pub fn is_stream_message_handshake_ack(payload: String) -> bool {
    is_xbox_stream_message_handshake_ack(&payload)
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct XboxPreparedSignaling {
    pub generation: u64,
    pub ice_servers: Vec<XboxIceServer>,
    pub fallback_ice_servers: Option<Vec<XboxIceServer>>,
    pub web_rtc_plan: XboxWebRtcPlan,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct XboxWebRtcPlan {
    pub audio_direction: String,
    pub video_direction: String,
    pub video_codec_mime_type: String,
    pub target_video_width: u32,
    pub target_video_height: u32,
    pub audio_bitrate_kbps: Option<u32>,
    pub h264_profiles: Vec<String>,
    pub h264_packetization_mode: u8,
    pub h264_level_asymmetry_allowed: bool,
    pub max_frame_size: u32,
    pub max_frame_rate: u32,
    pub min_video_bitrate_kbps: Option<u32>,
    pub start_video_bitrate_kbps: Option<u32>,
    pub max_video_bitrate_kbps: Option<u32>,
    pub stereo_audio: bool,
    pub required_video_rtcp_feedback: Vec<String>,
    pub allowed_candidate_types: Vec<String>,
    pub ice_transport_policy: String,
    pub prefer_ipv6: bool,
    pub normalize_end_of_candidates: bool,
    pub console_addresses: Vec<XboxHostAddress>,
    pub prevent_resolution_drops: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct XboxCustomTurn {
    pub server_url: String,
    pub username: String,
    pub credential: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct XboxStreamSettings {
    pub cloud_server_region: String,
    pub preferred_game_locale: String,
    pub cloud_resolution: i64,
    pub home_resolution: i64,
    pub prefer_ipv6: bool,
    pub video_codec: String,
    pub home_bitrate_mode: String,
    pub home_bitrate_mbps: i64,
    pub cloud_bitrate_mode: String,
    pub cloud_bitrate_mbps: i64,
    pub audio_bitrate_mode: String,
    pub audio_bitrate_kbps: i64,
    pub home_turn_fallback: bool,
    pub custom_turn: Option<XboxCustomTurn>,
    pub prevent_resolution_drops: bool,
    pub power_on: bool,
}

impl Default for XboxStreamSettings {
    fn default() -> Self {
        Self {
            cloud_server_region: "auto".to_string(),
            preferred_game_locale: "en-US".to_string(),
            cloud_resolution: 720,
            home_resolution: 1080,
            prefer_ipv6: false,
            video_codec: String::new(),
            home_bitrate_mode: "Auto".to_string(),
            home_bitrate_mbps: 20,
            cloud_bitrate_mode: "Auto".to_string(),
            cloud_bitrate_mbps: 20,
            audio_bitrate_mode: "Auto".to_string(),
            audio_bitrate_kbps: 20,
            home_turn_fallback: false,
            custom_turn: None,
            prevent_resolution_drops: false,
            power_on: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionPhase {
    Idle,
    Creating,
    Ready,
    Negotiating,
    Connected,
    Closing,
    Closed,
    Failed,
}

impl SessionPhase {
    fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Creating => "creating",
            Self::Ready => "ready",
            Self::Negotiating => "negotiating",
            Self::Connected => "connected",
            Self::Closing => "closing",
            Self::Closed => "closed",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone)]
struct IosSessionSnapshot {
    session_id: String,
    session_path: String,
    target_id: String,
    target_type: String,
    runtime: SessionRuntimeSnapshot,
}

impl SessionRuntimeBinding for IosSessionSnapshot {
    fn runtime_snapshot(&self) -> SessionRuntimeSnapshot {
        self.runtime.clone()
    }

    fn replace_runtime_snapshot(&mut self, runtime: SessionRuntimeSnapshot) {
        self.runtime = runtime;
    }
}

impl SessionFlowSnapshot for IosSessionSnapshot {
    fn new_pending(
        session_id: String,
        session_path: String,
        target_id: String,
        target_type: String,
    ) -> Self {
        Self {
            session_id,
            session_path,
            target_id,
            target_type,
            runtime: SessionRuntimeSnapshot {
                stream_state: None,
                player_state: "pending".to_string(),
                queue: None,
                error_details: None,
            },
        }
    }

    fn session_id(&self) -> &str {
        &self.session_id
    }

    fn session_path(&self) -> &str {
        &self.session_path
    }

    fn target_id(&self) -> &str {
        &self.target_id
    }

    fn target_type(&self) -> &str {
        &self.target_type
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RemoteSessionTerminal {
    Failed,
    Closed,
    TimedOut,
}

impl RemoteSessionTerminal {
    fn error_code(self) -> &'static str {
        match self {
            Self::Failed => "remoteSessionFailed",
            Self::Closed => "remoteSessionClosed",
            Self::TimedOut => "remoteSessionTimedOut",
        }
    }
}

#[derive(Debug, Clone, Default)]
struct SessionFlowProjection {
    generation: u64,
    session_id: Option<String>,
    remote_terminal: Option<RemoteSessionTerminal>,
    remote_observation_revision: u64,
    startup_progress: Option<XboxStartupProgress>,
    startup_phase: Option<xbox_streaming::SessionStartupPhase>,
}

#[derive(Clone)]
struct IosSessionFlowProvider {
    credentials: Arc<SessionCredentials>,
    target: Target,
    projection: Arc<StdMutex<SessionFlowProjection>>,
    console_ready_cache: Arc<
        Mutex<
            Option<(
                std::time::Instant,
                Vec<xbox_streaming::RemoteConsoleSnapshot>,
            )>,
        >,
    >,
}

#[async_trait]
impl SessionFlowProvider for IosSessionFlowProvider {
    fn on_session_lifecycle_event(&self, event: &str, _session_id: Option<&str>, _payload: Value) {
        if event == "cleanupStarted" {
            self.credentials.begin_cleanup();
        }
    }
    async fn get_streaming_token(&self, target_type: &str) -> Result<Value, SessionFlowError> {
        let access = self.credentials.access(target_type, None, None).await?;
        Ok(json!({ "gsToken": access.bearer_token, "_authGeneration": access.owner_generation }))
    }

    async fn recover_streaming_token(
        &self,
        target_type: &str,
        rejected_token: &str,
        credential_generation: Option<u64>,
    ) -> Result<Value, xbox_webapi::WebApiError> {
        let access = self
            .credentials
            .access(target_type, Some(rejected_token), credential_generation)
            .await?;
        Ok(json!({ "gsToken": access.bearer_token, "_authGeneration": access.owner_generation }))
    }

    async fn transfer_token(&self) -> Result<String, SessionFlowError> {
        let access = self
            .credentials
            .access(self.target.as_str(), None, None)
            .await?;
        AuthFlow::new()
            .transfer_token(TransferTokenInput {
                refresh_token: access.refresh_token,
            })
            .await
            .map(|output| output.transfer_token)
            .map_err(|error| SessionFlowError::message(error.to_string()))
    }

    async fn power_on_console(&self, console_id: &str) -> Result<bool, SessionFlowError> {
        if self.target != Target::Home {
            return Ok(false);
        }
        let access = self
            .credentials
            .access(self.target.as_str(), None, None)
            .await?;
        let facts = find_home_host_facts(&access.account_id, console_id)
            .map_err(|error| SessionFlowError::message(error.to_string()))?
            .ok_or_else(|| SessionFlowError::message("homeTargetContextMissing"))?;
        let command_id = facts
            .command_id()
            .ok_or_else(|| SessionFlowError::message("homeCommandIdentityMissing"))?;
        let command_id = command_id.to_owned();
        self.credentials
            .run_web_query(|access| {
                let command_id = command_id.clone();
                async move {
                    ConsoleApi::new(access.web_uhs, access.web_token)
                        .power_on(&command_id)
                        .await
                        .map(|response| response.accepted)
                }
            })
            .await
            .map_err(SessionFlowError::from)
    }

    async fn get_remote_consoles(
        &self,
    ) -> Result<Vec<xbox_streaming::RemoteConsoleSnapshot>, SessionFlowError> {
        if self.target != Target::Home {
            return Ok(Vec::new());
        }
        let mut cache = self.console_ready_cache.lock().await;
        if let Some((recorded_at, consoles)) = cache.as_ref() {
            if recorded_at.elapsed()
                < xbox_streaming::session::operation::CONSOLE_READY_SMARTGLASS_CACHE_TTL
            {
                return Ok(consoles.clone());
            }
        }
        let result = self
            .credentials
            .run_web_query(|access| async move {
                let response = SmartglassApi::new(access.web_uhs, access.web_token)
                    .get_consoles_list()
                    .await?;
                let facts = extract_home_host_facts(&response)?;
                replace_home_host_facts(&access.account_id, facts.clone())
                    .map_err(|_| xbox_webapi::WebApiError::parse("home facts unavailable"))?;
                Ok(home_context::smartglass_ready_candidates(facts))
            })
            .await;
        // PC 以本轮 Smartglass 事实判断就绪，查询失败交给公共就绪等待继续处理。
        let consoles = result.unwrap_or_default();
        *cache = Some((std::time::Instant::now(), consoles.clone()));
        Ok(consoles)
    }

    fn on_session_created(
        &self,
        session_id: &str,
        _session_path: &str,
        _target_type: &str,
        _target_id: &str,
        _recreate_from_session_id: Option<&str>,
    ) {
        if let Ok(mut projection) = self.projection.lock() {
            projection.session_id = Some(session_id.to_string());
            projection.remote_terminal = None;
        }
    }

    fn on_session_monitor_tick(
        &self,
        session_id: &str,
        _target_type: &str,
        _target_id: &str,
        progress: &SessionProgressSnapshot,
        stream_state: Option<&str>,
        _player_state: &str,
        _should_continue: bool,
        _should_send_connect_token: bool,
    ) {
        let terminal = remote_session_terminal(
            progress.phase,
            progress.runtime_launch_state,
            stream_state,
            progress.error_code.as_deref(),
        );
        if let Ok(mut projection) = self.projection.lock() {
            if projection.session_id.as_deref() == Some(session_id) {
                projection.remote_observation_revision =
                    projection.remote_observation_revision.saturating_add(1);
                if let Some(startup) = &mut projection.startup_progress {
                    startup.apply_session(progress, stream_state);
                }
                if let Some(terminal) = terminal {
                    projection.remote_terminal.get_or_insert(terminal);
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
struct SessionState {
    generation: u64,
    phase: SessionPhase,
    completed_offer_exchanges: u64,
    signaling_epoch: u64,
    active_signaling_restart: bool,
    remote_ice: Vec<IceCandidate>,
    remote_ice_next_poll_at: Option<tokio::time::Instant>,
    remote_ice_observed: bool,
    remote_ice_end_seen: bool,
    remote_ice_complete: bool,
}

impl SessionState {
    fn new() -> Self {
        Self {
            generation: 0,
            phase: SessionPhase::Idle,
            completed_offer_exchanges: 0,
            signaling_epoch: 0,
            active_signaling_restart: false,
            remote_ice: Vec::new(),
            remote_ice_next_poll_at: None,
            remote_ice_observed: false,
            remote_ice_end_seen: false,
            remote_ice_complete: false,
        }
    }

    fn begin_offer_exchange(&mut self) -> bool {
        let restart = self.completed_offer_exchanges > 0;
        self.signaling_epoch = self.signaling_epoch.saturating_add(1);
        self.active_signaling_restart = restart;
        self.phase = SessionPhase::Negotiating;
        if restart {
            self.reset_remote_ice();
        }
        restart
    }

    fn complete_offer_exchange(&mut self) {
        self.completed_offer_exchanges = self.completed_offer_exchanges.saturating_add(1);
    }

    fn reset_signaling(&mut self) {
        self.completed_offer_exchanges = 0;
        self.signaling_epoch = 0;
        self.active_signaling_restart = false;
        self.reset_remote_ice();
    }

    fn reset_remote_ice(&mut self) {
        self.remote_ice.clear();
        self.remote_ice_next_poll_at = None;
        self.remote_ice_observed = false;
        self.remote_ice_end_seen = false;
        self.remote_ice_complete = false;
    }
}

type IosSessionFlow = SessionFlowService<IosSessionSnapshot, IosSessionFlowProvider>;

/// Swift 只转交 libwebrtc 产生的 offer/local ICE，并应用这里返回的 remote ICE。
/// 会话编排、轮询、connect token、keepalive、signaling 与结束判定全部由 Rust 持有。
#[derive(uniffi::Object)]
pub struct XboxStreamSession {
    plan: StdMutex<Option<Plan>>,
    target_id: String,
    settings: XboxStreamSettings,
    flow: IosSessionFlow,
    access: StreamingAccessContext,
    credentials: Arc<SessionCredentials>,
    projection: Arc<StdMutex<SessionFlowProjection>>,
    state: Mutex<SessionState>,
    generation: AtomicU64,
    startup_scope: StdMutex<SessionStartupScope>,
    startup_context: Option<Arc<XboxStreamStartup>>,
}

#[uniffi::export]
pub fn create_stream_session(
    access_handle: String,
    target_id: String,
) -> Result<Arc<XboxStreamSession>, XboxStreamingError> {
    let target_id = normalize_target_id(&target_id)?;
    let access = load_stream_access(access_handle.trim())
        .map_err(|error| XboxStreamingError::Authentication(error.to_string()))?;
    build_stream_session(
        access_handle,
        access,
        target_id,
        XboxStreamSettings::default(),
        None,
    )
}

/// 账户、代际和 target 都由 Swift 启动请求显式回传，bridge 在创建远端会话前校验。
#[uniffi::export]
pub fn create_scoped_stream_session(
    access_handle: String,
    target_type: String,
    target_id: String,
    account_id: String,
    owner_generation: u64,
    settings: XboxStreamSettings,
    startup: Option<Arc<XboxStreamStartup>>,
) -> Result<Arc<XboxStreamSession>, XboxStreamingError> {
    let target = parse_target_type(&target_type)?;
    let target_id = normalize_target_id(&target_id)?;
    let account_id = account_id.trim();
    if account_id.is_empty() || account_id.len() > 256 || account_id.chars().any(char::is_control) {
        return Err(XboxStreamingError::InvalidArgument(
            "stream owner account is invalid".into(),
        ));
    }
    let access = load_scoped_stream_access(
        access_handle.trim(),
        target,
        Some(account_id),
        Some(owner_generation),
    )
    .map_err(|error| XboxStreamingError::Authentication(error.to_string()))?;
    if startup
        .as_ref()
        .is_some_and(|startup| startup.target != target)
    {
        return Err(XboxStreamingError::InvalidArgument(
            "startupTargetMismatch".into(),
        ));
    }
    if startup
        .as_ref()
        .is_some_and(|startup| startup.scope.is_cancelled())
    {
        return Err(XboxStreamingError::Cancelled(owner_generation));
    }
    build_stream_session(access_handle, access, target_id, settings, startup)
}

fn build_stream_session(
    access_handle: String,
    access: StreamingAccessContext,
    target_id: String,
    settings: XboxStreamSettings,
    startup: Option<Arc<XboxStreamStartup>>,
) -> Result<Arc<XboxStreamSession>, XboxStreamingError> {
    let projection = Arc::new(StdMutex::new(SessionFlowProjection::default()));
    let credentials = Arc::new(SessionCredentials::new(
        access_handle.trim().to_owned(),
        access.clone(),
    ));
    let flow = SessionFlowService::new(IosSessionFlowProvider {
        console_ready_cache: Arc::new(Mutex::new(None)),
        credentials: Arc::clone(&credentials),
        target: access.target,
        projection: Arc::clone(&projection),
    });

    Ok(Arc::new(XboxStreamSession {
        plan: StdMutex::new(None),
        target_id,
        settings,
        flow,
        access,
        credentials,
        projection,
        state: Mutex::new(SessionState::new()),
        generation: AtomicU64::new(0),
        startup_scope: StdMutex::new(
            startup
                .as_ref()
                .map(|startup| startup.scope.clone())
                .unwrap_or_default(),
        ),
        startup_context: startup,
    }))
}

#[uniffi::export(async_runtime = "tokio")]
impl XboxStreamSession {
    pub async fn pending_auth_update(&self) -> Option<XboxStreamingAuthUpdate> {
        self.credentials.pending_update().await
    }

    pub async fn acknowledge_auth_update(&self, revision: u64) {
        self.credentials.acknowledge(revision).await;
    }

    pub async fn synchronize_auth_session(
        &self,
        revision: u64,
        auth_session: crate::AuthSession,
    ) -> Result<(), XboxStreamingError> {
        self.credentials
            .synchronize(revision, auth_session)
            .await
            .map_err(|_| {
                XboxStreamingError::Authentication(
                    "host credential synchronization rejected".into(),
                )
            })
    }
    pub fn startup_progress(&self) -> Option<XboxStartupProgress> {
        self.projection_snapshot()
            .and_then(|projection| projection.startup_progress)
    }

    pub async fn start(&self) -> Result<XboxPreparedSignaling, XboxStreamingError> {
        let generation = self.begin_generation().await?;
        let scope = self
            .startup_scope
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone();
        let observer = IosStartupObserver::new(Arc::clone(&self.projection), generation);
        let context_timeout = match self.startup_context.as_ref() {
            Some(startup) => startup.context_timeout(),
            None => Ok(xbox_streaming::session::RESOLVING_CONTEXT_TIMEOUT),
        };
        let context_timeout = match context_timeout {
            Ok(timeout) => timeout,
            Err(error) => {
                self.mark_failed(generation).await?;
                return Err(startup::map_startup_error(
                    error,
                    xbox_streaming::SessionStartupPhase::ResolvingContext,
                ));
            }
        };
        // UniFFI 在 Swift 小栈线程上按值包装 future；重型阶段分别放到堆上。
        let plan = match Box::pin(scope.run(
            "resolvingContext",
            Some(context_timeout),
            self.prepare_start_plan(&scope),
        ))
        .await
        {
            Ok(plan) => plan,
            Err(error) => {
                self.mark_failed(generation).await?;
                return Err(startup::map_startup_error(
                    error,
                    xbox_streaming::SessionStartupPhase::ResolvingContext,
                ));
            }
        };
        ensure_generation(&self.generation, generation)?;
        *self
            .plan
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = Some(plan.clone());
        let execution = match Box::pin(self.flow.start_session_execution_with_scope(
            plan.clone(),
            |_| (),
            |_| (),
            Some(&observer),
            &scope,
        ))
        .await
        {
            Ok(execution) => execution,
            Err(error) => {
                let phase = self
                    .projection_snapshot()
                    .and_then(|projection| projection.startup_phase)
                    .unwrap_or(xbox_streaming::SessionStartupPhase::CreatingSession);
                self.mark_failed(generation).await?;
                return Err(startup::map_startup_error(error, phase));
            }
        };
        ensure_generation(&self.generation, generation)?;

        if let Ok(mut projection) = self.projection.lock() {
            projection.session_id = Some(execution.session.session_id.clone());
            if let Some(progress) = &mut projection.startup_progress {
                progress.phase = "ready".into();
                progress.status = "succeeded".into();
            }
        }
        let mut state = self.state.lock().await;
        ensure_generation(&self.generation, generation)?;
        state.phase = SessionPhase::Ready;
        let (ice_servers, fallback_ice_servers) = prepare_runtime_ice_servers(&plan);
        Ok(XboxPreparedSignaling {
            generation,
            ice_servers,
            fallback_ice_servers,
            web_rtc_plan: project_web_rtc_plan(&plan),
        })
    }

    pub async fn exchange_offer(
        &self,
        generation: u64,
        sdp: String,
    ) -> Result<String, XboxStreamingError> {
        let session_id = self.session_id_for(generation).await?;
        if sdp.trim().is_empty() {
            return Err(XboxStreamingError::InvalidArgument(
                "SDP offer is empty".into(),
            ));
        }
        let restart = {
            let mut state = self.state.lock().await;
            ensure_generation(&self.generation, generation)?;
            state.begin_offer_exchange()
        };
        let timeout_ms = self
            .plan_snapshot()?
            .session
            .schedule
            .ready_timeout_ms
            .max(1_000);
        let answer = tokio::time::timeout(
            Duration::from_millis(timeout_ms),
            Box::pin(self.flow.exchange_offer(&session_id, None, &sdp, restart)),
        )
        .await
        .map_err(|_| {
            XboxStreamingError::Remote(format!(
                "Timed out waiting for SDP answer after {timeout_ms}ms"
            ))
        })?
        .map_err(map_session_flow_error)?;
        ensure_generation(&self.generation, generation)?;
        let mut state = self.state.lock().await;
        ensure_generation(&self.generation, generation)?;
        state.complete_offer_exchange();
        Ok(answer.sdp)
    }

    /// Chat/quality 使用同一个 Peer 的媒体协商，保留主 ICE 代次。
    pub async fn exchange_media_offer(
        &self,
        generation: u64,
        sdp: String,
        kind: String,
    ) -> Result<String, XboxStreamingError> {
        if !matches!(kind.as_str(), "chat" | "quality") || sdp.trim().is_empty() {
            return Err(XboxStreamingError::InvalidArgument(
                "media offer kind or SDP invalid".into(),
            ));
        }
        let session_id = self.session_id_for(generation).await?;
        self.ensure_remote_session_active(&session_id)?;
        let answer = tokio::time::timeout(
            Duration::from_secs(10),
            Box::pin(
                self.flow
                    .exchange_offer(&session_id, Some(&kind), &sdp, false),
            ),
        )
        .await
        .map_err(|_| XboxStreamingError::Remote("mediaNegotiationTimeout".into()))?
        .map_err(map_session_flow_error)?;
        ensure_generation(&self.generation, generation)?;
        self.ensure_remote_session_active(&session_id)?;
        Ok(answer.sdp)
    }

    pub async fn supervise_runtime(
        &self,
        generation: u64,
        sample_json: String,
    ) -> Result<String, XboxStreamingError> {
        if sample_json.len() > 16_384 {
            return Err(XboxStreamingError::InvalidArgument(
                "runtime sample budget exceeded".into(),
            ));
        }
        let sample = serde_json::from_str(&sample_json)
            .map_err(|_| XboxStreamingError::Parse("runtime sample invalid".into()))?;
        let session_id = self.session_id_for(generation).await?;
        self.ensure_remote_session_active(&session_id)?;
        let decision = self
            .flow
            .supervise_runtime(&session_id, sample)
            .await
            .map_err(map_session_flow_error)?;
        ensure_generation(&self.generation, generation)?;
        serde_json::to_string(&decision)
            .map_err(|_| XboxStreamingError::Parse("runtime decision invalid".into()))
    }

    pub async fn validate_resume(&self, generation: u64) -> Result<(), XboxStreamingError> {
        let session_id = self.session_id_for(generation).await?;
        self.ensure_remote_session_active(&session_id)?;
        let target = self.plan_snapshot()?.session.target;
        self.credentials
            .access(target.as_str(), None, None)
            .await
            .map_err(|_| {
                XboxStreamingError::Authentication("resumeCredentialsUnavailable".into())
            })?;
        let before = self
            .projection_snapshot()
            .map(|p| p.remote_observation_revision)
            .unwrap_or(0);
        tokio::time::timeout(
            Duration::from_secs(10),
            Box::pin(self.flow.refresh_session_observation(&session_id)),
        )
        .await
        .map_err(|_| XboxStreamingError::Network("resumeObservationTimeout".into()))?;
        ensure_generation(&self.generation, generation)?;
        self.ensure_remote_session_active(&session_id)?;
        let after = self
            .projection_snapshot()
            .map(|p| p.remote_observation_revision)
            .unwrap_or(0);
        if before == after {
            return Err(XboxStreamingError::Network(
                "resumeObservationUnavailable".into(),
            ));
        }
        if !self.flow.session_allows_recovery(&session_id).await {
            return Err(XboxStreamingError::Remote("remoteSessionTerminal".into()));
        }
        self.ensure_remote_session_active(&session_id)
    }

    pub async fn submit_ice(
        &self,
        generation: u64,
        candidates: Vec<XboxIceCandidate>,
    ) -> Result<(), XboxStreamingError> {
        let session_id = self.session_id_for(generation).await?;
        let candidates = candidates
            .into_iter()
            .map(|candidate| IceCandidate {
                source: candidate.source,
                candidate: candidate.candidate,
                sdp_m_line_index: candidate.sdp_m_line_index,
                sdp_mid: candidate.sdp_mid,
                username_fragment: candidate.username_fragment,
                message_type: candidate.message_type,
            })
            .collect::<Vec<_>>();
        let restart = {
            let state = self.state.lock().await;
            ensure_generation(&self.generation, generation)?;
            state.active_signaling_restart
        };
        Box::pin(self.flow.submit_ice(&session_id, &candidates, restart))
            .await
            .map_err(map_session_flow_error)?;
        ensure_generation(&self.generation, generation)
    }

    pub async fn next_remote_ice_batch(
        &self,
        generation: u64,
    ) -> Result<XboxRemoteIceBatch, XboxStreamingError> {
        let session_id = self.session_id_for(generation).await?;
        self.ensure_remote_session_active(&session_id)?;
        let (signaling_epoch, restart, next_poll_at) = {
            let state = self.state.lock().await;
            ensure_generation(&self.generation, generation)?;
            if state.remote_ice_complete {
                return Ok(XboxRemoteIceBatch {
                    candidates: Vec::new(),
                    end_of_candidates: true,
                    configuration_pending: false,
                });
            }
            (
                state.signaling_epoch,
                state.active_signaling_restart,
                state.remote_ice_next_poll_at,
            )
        };
        if let Some(next_poll_at) = next_poll_at {
            tokio::time::sleep_until(next_poll_at).await;
        }
        ensure_generation(&self.generation, generation)?;
        let candidates = Box::pin(self.flow.poll_ice(&session_id, restart))
            .await
            .map_err(map_session_flow_error)?;
        let configuration_pending = self.flow.home_configuration_pending(&session_id).await;
        ensure_generation(&self.generation, generation)?;
        self.ensure_remote_session_active(&session_id)?;
        let mut state = self.state.lock().await;
        ensure_generation(&self.generation, generation)?;
        if state.signaling_epoch != signaling_epoch {
            return Err(XboxStreamingError::InvalidState(
                "signalingSuperseded".into(),
            ));
        }
        state.remote_ice_next_poll_at = Some(
            tokio::time::Instant::now()
                + Duration::from_millis(
                    self.plan_snapshot()?
                        .session
                        .schedule
                        .ice_poll_interval_ms
                        .max(100),
                ),
        );
        let batch = project_remote_ice_batch(&mut state, candidates, configuration_pending);
        if batch.end_of_candidates && !state.remote_ice_observed {
            return Err(XboxStreamingError::Remote(
                "remoteIceCandidatesMissing".into(),
            ));
        }
        Ok(batch)
    }

    pub async fn cancel(&self) -> u64 {
        let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        if let Ok(mut projection) = self.projection.lock() {
            projection.generation = generation;
            projection.startup_progress = None;
        }
        self.startup_scope
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .cancel();
        let mut state = self.state.lock().await;
        state.generation = generation;
        state.phase = SessionPhase::Failed;
        generation
    }

    pub async fn close(&self) -> Result<(), XboxStreamingError> {
        let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        if let Ok(mut projection) = self.projection.lock() {
            projection.generation = generation;
            projection.startup_progress = None;
        }
        self.startup_scope
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .cancel();
        {
            let mut state = self.state.lock().await;
            state.generation = generation;
            if state.phase == SessionPhase::Closed {
                return Ok(());
            }
            state.phase = SessionPhase::Closing;
        }
        let session_id = self
            .projection_snapshot()
            .and_then(|projection| projection.session_id);
        if let Some(session_id) = session_id {
            let result = self
                .flow
                .close_session(&session_id)
                .await
                .map_err(map_session_flow_error);
            self.credentials.revoke();
            result?;
        }
        self.credentials.revoke();
        if let Ok(mut projection) = self.projection.lock() {
            *projection = SessionFlowProjection::default();
        }
        let mut state = self.state.lock().await;
        ensure_generation(&self.generation, generation)?;
        state.phase = SessionPhase::Closed;
        state.reset_signaling();
        Ok(())
    }

    pub async fn mark_connected(&self, generation: u64) -> Result<(), XboxStreamingError> {
        let mut state = self.state.lock().await;
        ensure_generation(&self.generation, generation)?;
        match state.phase {
            SessionPhase::Negotiating | SessionPhase::Ready => {
                state.phase = SessionPhase::Connected;
            }
            SessionPhase::Connected => {}
            _ => {
                return Err(XboxStreamingError::InvalidState(
                    "session is not negotiating".into(),
                ));
            }
        }
        Ok(())
    }

    pub async fn send_text(
        &self,
        generation: u64,
        text: String,
    ) -> Result<bool, XboxStreamingError> {
        self.ensure_active_home_generation(generation).await?;
        let text = normalize_console_text(&text)?;
        let command_id = self.home_command_id()?;
        let access = self
            .credentials
            .access(Target::Home.as_str(), None, None)
            .await
            .map_err(|error| XboxStreamingError::Authentication(error.to_string()))?;
        ensure_generation(&self.generation, generation)?;
        ConsoleApi::new(access.web_uhs, access.web_token)
            .send_text(&command_id, &text)
            .await
            .map(|response| response.accepted)
            .map_err(|error| XboxStreamingError::Network(error.to_string()))
    }

    pub async fn power_off_console(&self, generation: u64) -> Result<bool, XboxStreamingError> {
        self.ensure_active_home_generation(generation).await?;
        let command_id = self.home_command_id()?;
        let access = self
            .credentials
            .access(Target::Home.as_str(), None, None)
            .await
            .map_err(|error| XboxStreamingError::Authentication(error.to_string()))?;
        ensure_generation(&self.generation, generation)?;
        ConsoleApi::new(access.web_uhs, access.web_token)
            .power_off(&command_id)
            .await
            .map(|response| response.accepted)
            .map_err(|error| XboxStreamingError::Network(error.to_string()))
    }
}

impl XboxStreamSession {
    async fn prepare_start_plan(
        &self,
        scope: &SessionStartupScope,
    ) -> Result<Plan, SessionFlowError> {
        let access = self
            .credentials
            .access(self.access.target.as_str(), None, None)
            .await?;
        // 发现 future 的构造和轮询也保留小栈余量，避免压力随包装向下传递。
        let mut access = Box::pin(home_context::resolve_home_context(
            access,
            &self.credentials,
            &self.target_id,
            &self.settings,
            scope,
        ))
        .await?;
        if access.target.is_home() {
            access.fallback_turn = home_context::resolve_fallback_turn().await;
        }
        control_plan(&access, &self.target_id, &self.settings)
            .map_err(|error| SessionFlowError::message(error.to_string()))
    }

    fn plan_snapshot(&self) -> Result<Plan, XboxStreamingError> {
        self.plan
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
            .ok_or_else(|| {
                XboxStreamingError::InvalidState("streaming plan is not prepared".into())
            })
    }
    async fn ensure_active_home_generation(
        &self,
        generation: u64,
    ) -> Result<(), XboxStreamingError> {
        ensure_generation(&self.generation, generation)?;
        if self.access.target != Target::Home {
            return Err(XboxStreamingError::InvalidState(
                "console commands require a home session".into(),
            ));
        }
        let state = self.state.lock().await;
        ensure_generation(&self.generation, generation)?;
        if !matches!(
            state.phase,
            SessionPhase::Ready | SessionPhase::Negotiating | SessionPhase::Connected
        ) {
            return Err(XboxStreamingError::InvalidState(
                "stream session cannot accept console commands".into(),
            ));
        }
        Ok(())
    }

    fn home_command_id(&self) -> Result<String, XboxStreamingError> {
        find_home_host_facts(&self.access.account_id, &self.target_id)
            .map_err(|error| XboxStreamingError::InvalidState(error.to_string()))?
            .and_then(|facts| facts.command_id().map(str::to_owned))
            .ok_or_else(|| XboxStreamingError::InvalidState("homeCommandIdentityMissing".into()))
    }

    async fn begin_generation(&self) -> Result<u64, XboxStreamingError> {
        let mut state = self.state.lock().await;
        let current_generation = self.generation.load(Ordering::Acquire);
        if !matches!(
            state.phase,
            SessionPhase::Idle | SessionPhase::Closed | SessionPhase::Failed
        ) && state.generation == current_generation
        {
            return Err(XboxStreamingError::InvalidState(
                "session is already active".into(),
            ));
        }
        let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        state.generation = generation;
        let first_start = state.phase == SessionPhase::Idle;
        state.phase = SessionPhase::Creating;
        state.reset_signaling();
        {
            let mut scope = self
                .startup_scope
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            if !first_start {
                scope.cancel();
                *scope = SessionStartupScope::default();
            }
        }
        if let Ok(mut projection) = self.projection.lock() {
            *projection = SessionFlowProjection {
                generation,
                startup_progress: Some(XboxStartupProgress::initial(self.access.target.is_home())),
                ..SessionFlowProjection::default()
            };
        }
        Ok(generation)
    }

    async fn mark_failed(&self, generation: u64) -> Result<(), XboxStreamingError> {
        let mut state = self.state.lock().await;
        ensure_generation(&self.generation, generation)?;
        state.phase = SessionPhase::Failed;
        Ok(())
    }

    async fn session_id_for(&self, generation: u64) -> Result<String, XboxStreamingError> {
        ensure_generation(&self.generation, generation)?;
        let phase = self.state.lock().await.phase;
        ensure_generation(&self.generation, generation)?;
        if !matches!(
            phase,
            SessionPhase::Ready | SessionPhase::Negotiating | SessionPhase::Connected
        ) {
            return Err(XboxStreamingError::InvalidState(format!(
                "session is {}",
                phase.as_str()
            )));
        }
        self.projection_snapshot()
            .and_then(|projection| projection.session_id)
            .ok_or_else(|| XboxStreamingError::InvalidState("session id is missing".into()))
    }

    fn projection_snapshot(&self) -> Option<SessionFlowProjection> {
        self.projection
            .lock()
            .ok()
            .map(|projection| projection.clone())
    }

    fn ensure_remote_session_active(&self, session_id: &str) -> Result<(), XboxStreamingError> {
        let terminal = self.projection_snapshot().and_then(|projection| {
            (projection.session_id.as_deref() == Some(session_id))
                .then_some(projection.remote_terminal)
                .flatten()
        });
        if let Some(terminal) = terminal {
            return Err(XboxStreamingError::Remote(
                terminal.error_code().to_string(),
            ));
        }
        Ok(())
    }
}

fn remote_session_terminal(
    phase: FlowSessionPhase,
    launch_state: RuntimeLaunchState,
    stream_state: Option<&str>,
    error_code: Option<&str>,
) -> Option<RemoteSessionTerminal> {
    if error_code.is_some_and(|code| code.eq_ignore_ascii_case("SessionStateTimeout")) {
        return Some(RemoteSessionTerminal::TimedOut);
    }
    match (phase, launch_state, stream_state) {
        (FlowSessionPhase::Failed, _, _) => Some(RemoteSessionTerminal::Failed),
        (FlowSessionPhase::Closed, RuntimeLaunchState::Closed, _) => {
            Some(RemoteSessionTerminal::Closed)
        }
        _ => None,
    }
}

fn control_plan(
    access: &StreamingAccessContext,
    target_id: &str,
    settings: &XboxStreamSettings,
) -> Result<Plan, XboxStreamingError> {
    let mut config = match access.target {
        Target::Cloud => Config::default(),
        Target::Home => Config::new_home_config(
            normalize_game_locale(&settings.preferred_game_locale),
            access.force_region_ip.clone(),
            settings.home_resolution,
        ),
    };
    if let Some(turn) = &settings.custom_turn {
        let uri = turn.server_url.trim();
        let (scheme, authority) = uri
            .split_once(':')
            .ok_or_else(|| XboxStreamingError::InvalidArgument("customTurnInvalid".to_string()))?;
        let endpoint = url::Url::parse(&format!("https://{authority}"))
            .map_err(|_| XboxStreamingError::InvalidArgument("customTurnInvalid".to_string()))?;
        if !["turn", "turns"].contains(&scheme.to_ascii_lowercase().as_str())
            || authority.starts_with("//")
            || uri.len() > 2048
            || uri.chars().any(char::is_whitespace)
            || endpoint.host_str().is_none()
            || endpoint.path() != "/"
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.fragment().is_some()
            || endpoint.port() == Some(0)
            || endpoint
                .query_pairs()
                .any(|(key, value)| key != "transport" || !["udp", "tcp"].contains(&value.as_ref()))
            || turn.username.trim().is_empty()
            || turn.username.len() > 512
            || turn.credential.trim().is_empty()
            || turn.credential.len() > 4096
        {
            return Err(XboxStreamingError::InvalidArgument(
                "customTurnInvalid".to_string(),
            ));
        }
        config.runtime.custom_turn = Some(TurnServer {
            url: format!("{}:{authority}", scheme.to_ascii_lowercase()),
            username: turn.username.trim().to_string(),
            credential: turn.credential.trim().to_string(),
        });
    }
    config.session.preferred_game_locale = normalize_game_locale(&settings.preferred_game_locale);
    config.session.power_on = settings.power_on;
    config.input.prevent_resolution_drops = settings.prevent_resolution_drops;
    if access.target == Target::Cloud {
        let region = settings.cloud_server_region.trim();
        if !region.is_empty() && !region.eq_ignore_ascii_case("auto") {
            config.session.preferred_region_name = Some(region.to_string());
        }
    }
    config.session.cloud_resolution = parse_resolution_preference(settings.cloud_resolution);
    config.session.home_resolution = parse_resolution_preference(settings.home_resolution);
    config.negotiation.cloud_prefer_ipv6 = settings.prefer_ipv6;
    config.negotiation.home_prefer_ipv6 = settings.prefer_ipv6;
    config.negotiation.video_codec = parse_codec_preference(&settings.video_codec);
    config.negotiation.home_video_bitrate =
        parse_video_bitrate_preference(&settings.home_bitrate_mode, settings.home_bitrate_mbps);
    config.negotiation.cloud_video_bitrate =
        parse_video_bitrate_preference(&settings.cloud_bitrate_mode, settings.cloud_bitrate_mbps);
    config.negotiation.audio_bitrate =
        parse_audio_bitrate_preference(&settings.audio_bitrate_mode, settings.audio_bitrate_kbps);
    if !access.force_region_ip.trim().is_empty() {
        config.session.force_region_ip = Some(access.force_region_ip.clone());
    }
    if access.target == Target::Home
        && settings.home_turn_fallback
        && access.fallback_turn.is_some()
    {
        config.runtime.home_fallback_turn = true;
    }
    let (target_id, remote_play) = match access.target {
        Target::Cloud => (target_id.to_string(), RemotePlayContext::default()),
        Target::Home => {
            let facts = find_home_host_facts(&access.account_id, target_id)
                .map_err(|error| XboxStreamingError::InvalidArgument(error.to_string()))?;
            if let Some(facts) = facts {
                validate_home_host_capability(&facts)?;
                let canonical_target_id = facts
                    .canonical_target_id()
                    .ok_or_else(|| {
                        XboxStreamingError::InvalidArgument("homeTargetIdentityMissing".to_string())
                    })?
                    .to_string();
                (
                    canonical_target_id,
                    RemotePlayContext {
                        configuration_resolved: false,
                        remote_management_enabled: facts.remote_console.remote_management_enabled,
                        console_streaming_enabled: facts.remote_console.console_streaming_enabled,
                        console_addrs: facts.console_addrs,
                    },
                )
            } else {
                (target_id.to_owned(), RemotePlayContext::default())
            }
        }
    };
    let context = Context {
        target: access.target,
        target_id,
        session: access.session_access.clone(),
        runtime: RuntimeCapabilities {
            browser_webrtc: true,
            rust_owned: false,
            native_mkb: false,
            touch_surface: true,
            prefer_browser: true,
        },
        remote_play,
        turn: TurnContext {
            fallback: access.fallback_turn.clone(),
        },
        ..Default::default()
    };
    compile(CompilerInput { config, context })
        .map(|output| output.plan)
        .map_err(|error| XboxStreamingError::InvalidArgument(error.to_string()))
}

fn normalize_game_locale(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        "en-US".to_string()
    } else {
        trimmed.to_string()
    }
}

fn parse_codec_preference(value: &str) -> CodecPreference {
    match value.trim() {
        "" => CodecPreference::Auto,
        "video/H264-420" => CodecPreference::H264Low,
        "video/H264-42e" => CodecPreference::H264Normal,
        "video/H264-4d" => CodecPreference::H264Main,
        "video/H264-64" => CodecPreference::H264High,
        mime_type => CodecPreference::MimeType {
            mime_type: mime_type.to_string(),
        },
    }
}

fn parse_video_bitrate_preference(mode: &str, bitrate_mbps: i64) -> BitratePreference {
    if mode != "Custom" || bitrate_mbps <= 0 {
        return BitratePreference::Auto;
    }

    BitratePreference::CustomKbps {
        kbps: bitrate_mbps.saturating_mul(1000).clamp(1, u32::MAX as i64) as u32,
    }
}

fn parse_audio_bitrate_preference(mode: &str, bitrate_kbps: i64) -> BitratePreference {
    if mode != "Custom" || bitrate_kbps <= 0 {
        return BitratePreference::Auto;
    }

    BitratePreference::CustomKbps {
        kbps: bitrate_kbps.clamp(1, u32::MAX as i64) as u32,
    }
}

fn validate_home_host_capability(facts: &HomeHostFacts) -> Result<(), XboxStreamingError> {
    let power_state = facts.remote_console.power_state.as_deref();
    let allowed = if power_state == Some("On") {
        facts.remote_console.console_streaming_enabled != Some(false)
    } else {
        facts.remote_console.remote_management_enabled != Some(false)
    };
    if allowed {
        Ok(())
    } else {
        Err(XboxStreamingError::InvalidArgument(
            "homeRemotePlayUnavailable".to_string(),
        ))
    }
}

fn resolve_ice_servers(plan: &Plan) -> Vec<XboxIceServer> {
    let mut servers = vec![XboxIceServer {
        urls: vec![OFFICIAL_XBOX_STUN_URL.to_string()],
        username: None,
        credential: None,
    }];
    if let Some(turn) = plan.runtime.turn.resolved.as_ref() {
        servers.push(XboxIceServer {
            urls: vec![turn.url.clone()],
            username: non_empty_string(&turn.username),
            credential: non_empty_string(&turn.credential),
        });
    }
    servers
}

// PC fallback 来源先尝试直连，自定义 TURN 保持首次连接直接装配。
fn prepare_runtime_ice_servers(plan: &Plan) -> (Vec<XboxIceServer>, Option<Vec<XboxIceServer>>) {
    let servers = resolve_ice_servers(plan);
    if plan.runtime.turn.source == TurnSource::Fallback && plan.runtime.turn.resolved.is_some() {
        (servers[..1].to_vec(), Some(servers))
    } else {
        (servers, None)
    }
}

fn project_web_rtc_plan(plan: &Plan) -> XboxWebRtcPlan {
    let (video_codec_mime_type, h264_profiles) = plan
        .negotiation
        .codec
        .as_ref()
        .map(|codec| (codec.mime_type.clone(), codec.profiles.clone()))
        .unwrap_or_else(|| {
            (
                "video/H264".to_string(),
                vec!["4d".to_string(), "42e".to_string(), "420".to_string()],
            )
        });
    let max_video_bitrate_kbps = plan.negotiation.video_bitrate_kbps;
    let macroblock_width = plan.session.device.max_width.saturating_add(15) / 16;
    let macroblock_height = plan.session.device.max_height.saturating_add(15) / 16;
    let max_frame_size = macroblock_width.saturating_mul(macroblock_height);
    let max_frame_rate = u32::from(plan.render.initial_target_fps.max(1));
    let (tier_min_bitrate_kbps, tier_start_bitrate_cap_kbps, tier_default_max_bitrate_kbps) =
        if plan.session.device.max_height <= 720 {
            (3_000, 10_000, 20_000)
        } else if plan.session.device.max_height > 1080 || plan.session.device.max_width > 1920 {
            (8_000, 35_000, 75_000)
        } else {
            (5_000, 20_000, 50_000)
        };
    let max_video_bitrate_kbps =
        Some(max_video_bitrate_kbps.unwrap_or(tier_default_max_bitrate_kbps));
    XboxWebRtcPlan {
        audio_direction: "recvonly".to_string(),
        video_direction: "recvonly".to_string(),
        video_codec_mime_type,
        target_video_width: plan.session.device.max_width.max(1),
        target_video_height: plan.session.device.max_height.max(1),
        audio_bitrate_kbps: plan.negotiation.audio_bitrate_kbps,
        h264_profiles,
        h264_packetization_mode: 1,
        h264_level_asymmetry_allowed: true,
        max_frame_size,
        max_frame_rate,
        min_video_bitrate_kbps: max_video_bitrate_kbps.map(|max| max.min(tier_min_bitrate_kbps)),
        start_video_bitrate_kbps: max_video_bitrate_kbps
            .map(|max| max.min(tier_start_bitrate_cap_kbps)),
        max_video_bitrate_kbps,
        stereo_audio: plan.negotiation.stereo_audio,
        required_video_rtcp_feedback: vec![
            "nack".to_string(),
            "nack pli".to_string(),
            "ccm fir".to_string(),
            "goog-remb".to_string(),
            "transport-cc".to_string(),
        ],
        allowed_candidate_types: vec!["host".to_string(), "srflx".to_string(), "relay".to_string()],
        ice_transport_policy: "all".to_string(),
        prefer_ipv6: plan.negotiation.prefer_ipv6,
        normalize_end_of_candidates: plan.negotiation.normalize_end_of_candidates,
        console_addresses: plan
            .negotiation
            .console_addrs
            .iter()
            .map(|address| XboxHostAddress {
                ip: address.ip.clone(),
                port: address.port,
            })
            .collect(),
        prevent_resolution_drops: plan.input.prevent_resolution_drops,
    }
}

fn non_empty_string(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn normalize_target_id(value: &str) -> Result<String, XboxStreamingError> {
    let target_id = value.trim();
    if target_id.is_empty() {
        return Err(XboxStreamingError::InvalidArgument(
            "stream target id is empty".into(),
        ));
    }
    if target_id.len() > 256 || target_id.chars().any(char::is_control) {
        return Err(XboxStreamingError::InvalidArgument(
            "stream target id must be a bounded printable identifier".into(),
        ));
    }
    Ok(target_id.to_string())
}

fn normalize_console_text(value: &str) -> Result<String, XboxStreamingError> {
    let text = value.trim();
    if text.is_empty() || text.chars().count() > 1_024 || text.chars().any(char::is_control) {
        return Err(XboxStreamingError::InvalidArgument(
            "text must be a bounded printable value".into(),
        ));
    }
    Ok(text.to_string())
}

fn parse_target_type(value: &str) -> Result<Target, XboxStreamingError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "cloud" => Ok(Target::Cloud),
        "home" => Ok(Target::Home),
        _ => Err(XboxStreamingError::InvalidArgument(
            "stream target type must be cloud or home".to_string(),
        )),
    }
}

fn map_session_flow_error(error: SessionFlowError) -> XboxStreamingError {
    if error.startup_hint.is_some() || error.message.starts_with("operationCancelled:") {
        return startup::map_startup_error(
            error,
            xbox_streaming::SessionStartupPhase::StartingRuntime,
        );
    }
    match error.status {
        Some(status) => XboxStreamingError::Http(status, error.message),
        None => XboxStreamingError::Remote(error.message),
    }
}

fn ensure_generation(generation: &AtomicU64, expected: u64) -> Result<(), XboxStreamingError> {
    let current = generation.load(Ordering::Acquire);
    if current != expected {
        return Err(XboxStreamingError::Cancelled(expected));
    }
    Ok(())
}

fn to_bridge_candidate(candidate: IceCandidate) -> XboxIceCandidate {
    XboxIceCandidate {
        source: candidate.source,
        candidate: candidate.candidate,
        sdp_m_line_index: candidate.sdp_m_line_index,
        sdp_mid: candidate.sdp_mid,
        username_fragment: candidate.username_fragment,
        message_type: candidate.message_type,
    }
}

fn is_end_of_candidates(value: &str) -> bool {
    let value = value.trim();
    value.is_empty()
        || value.eq_ignore_ascii_case("a=end-of-candidates")
        || value.eq_ignore_ascii_case("end-of-candidates")
}

fn project_remote_ice_batch(
    state: &mut SessionState,
    candidates: impl IntoIterator<Item = IceCandidate>,
    configuration_pending: bool,
) -> XboxRemoteIceBatch {
    let mut fresh = Vec::new();
    for candidate in candidates {
        let end = is_end_of_candidates(&candidate.candidate);
        state.remote_ice_end_seen |= end;
        if !state
            .remote_ice
            .iter()
            .any(|existing| existing == &candidate)
        {
            state.remote_ice_observed |= !end;
            state.remote_ice.push(candidate.clone());
            fresh.push(candidate);
        }
    }
    // 与 PC 共用结束事实：当前配置完成，且远端 EOC 或已收到候选并连接。
    state.remote_ice_complete = !configuration_pending
        && (state.remote_ice_end_seen
            || (state.remote_ice_observed && state.phase == SessionPhase::Connected));
    XboxRemoteIceBatch {
        candidates: fresh.into_iter().map(to_bridge_candidate).collect(),
        end_of_candidates: state.remote_ice_complete,
        configuration_pending,
    }
}

#[cfg(test)]
#[path = "streaming/stack_tests.rs"]
mod stack_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cloud_access::home_host_facts;
    use xbox_streaming::policy::session::SessionAccessContext;
    use xbox_streaming::{HostAddr, Region, RemoteConsoleSnapshot, TurnServer, TurnSource};

    fn test_access(target: Target) -> StreamingAccessContext {
        test_access_for_account(target, "xuid")
    }

    #[tokio::test]
    async fn provider_passes_shared_auth_generation_and_checks_recovery_scope() {
        let account = uuid::Uuid::new_v4().to_string();
        let (handle, access) = crate::cloud_access::register_test_stream_access(
            test_access_for_account(Target::Cloud, &account),
        );
        let generation = access.owner_generation;
        let provider = IosSessionFlowProvider {
            console_ready_cache: Arc::new(Mutex::new(None)),
            credentials: Arc::new(SessionCredentials::new(handle, access)),
            target: Target::Cloud,
            projection: Arc::new(StdMutex::new(SessionFlowProjection::default())),
        };
        let token = provider.get_streaming_token("cloud").await.unwrap();
        assert_eq!(token["_authGeneration"], generation);
        assert_eq!(token["gsToken"], "gs-token");
        assert_eq!(
            provider
                .recover_streaming_token("cloud", "gs-token", Some(generation + 1))
                .await
                .unwrap_err()
                .to_status_code(),
            Some(401)
        );
    }

    #[test]
    fn home_wake_is_an_explicit_setting_with_the_pc_default() {
        let account = uuid::Uuid::new_v4().to_string();
        let access = test_access_for_account(Target::Home, &account);
        let mut facts = home_host_facts(&account).unwrap();
        facts[0].remote_console.power_state = Some("Standby".into());
        replace_home_host_facts(&account, facts).unwrap();
        for enabled in [false, true] {
            let settings = XboxStreamSettings {
                power_on: enabled,
                ..Default::default()
            };
            let plan = control_plan(&access, "console-device-id", &settings).unwrap();
            assert_eq!(plan.session.schedule.wake_console, enabled);
            assert_eq!(plan.session.schedule.require_console_ready, enabled);
        }
        assert!(!XboxStreamSettings::default().power_on);
    }

    pub(super) fn test_access_for_account(
        target: Target,
        account_id: &str,
    ) -> StreamingAccessContext {
        let access = StreamingAccessContext {
            host: "cloud.example.com".to_string(),
            bearer_token: "gs-token".to_string(),
            account_id: account_id.to_string(),
            refresh_token: "refresh-token".to_string(),
            auth_seed: None,
            auth_session: None,
            auth_provider: None,
            auth_observer: None,
            target,
            session_access: SessionAccessContext {
                gs_token: Some("gs-token".to_string()),
                regions: vec![Region {
                    name: "TEST".to_string(),
                    base_uri: "https://cloud.example.com".to_string(),
                    is_default: true,
                    ..Default::default()
                }],
            },
            force_region_ip: String::new(),
            web_uhs: "uhs".to_string(),
            web_token: "web-token".to_string(),
            owner_generation: 1,
            created_at_ms: 1,
            expires_at_ms: u64::MAX,
            fallback_turn: None,
        };
        if target == Target::Home {
            cache_home_fixture(account_id, Some(true), Some(true));
        }
        access
    }

    fn cache_home_fixture(
        account_id: &str,
        remote_management_enabled: Option<bool>,
        console_streaming_enabled: Option<bool>,
    ) {
        replace_home_host_facts(
            account_id,
            vec![HomeHostFacts {
                remote_console: RemoteConsoleSnapshot {
                    id: Some("console-command-id".to_string()),
                    device_id: Some("console-device-id".to_string()),
                    server_id: Some("1234abcd".to_string()),
                    power_state: Some("On".to_string()),
                    remote_management_enabled,
                    console_streaming_enabled,
                    console_addrs_count: 1,
                    ready_source: Some("fixture".to_string()),
                },
                console_addrs: vec![HostAddr {
                    ip: "10.0.0.8".to_string(),
                    port: 9002,
                }],
            }],
        )
        .expect("home facts");
    }

    fn test_session_for_target(target: Target) -> Arc<XboxStreamSession> {
        let access = test_access(target);
        let plan = control_plan(&access, "1234abcd", &XboxStreamSettings::default()).unwrap();
        let session = build_stream_session(
            "test-handle".into(),
            access,
            "1234abcd".to_string(),
            XboxStreamSettings::default(),
            None,
        )
        .expect("session");
        *session.plan.lock().unwrap() = Some(plan);
        session
    }

    fn test_session() -> Arc<XboxStreamSession> {
        test_session_for_target(Target::Cloud)
    }

    fn candidate(value: &str) -> IceCandidate {
        IceCandidate {
            candidate: value.to_string(),
            sdp_m_line_index: Some(0),
            sdp_mid: Some("0".to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn remote_session_terminal_maps_timeout_and_terminal_states_without_server_text() {
        assert_eq!(
            remote_session_terminal(
                FlowSessionPhase::Failed,
                RuntimeLaunchState::Failed,
                Some("Provisioning"),
                Some("SessionStateTimeout")
            ),
            Some(RemoteSessionTerminal::TimedOut)
        );
        assert_eq!(
            remote_session_terminal(
                FlowSessionPhase::Failed,
                RuntimeLaunchState::Failed,
                Some("Failed"),
                None
            ),
            Some(RemoteSessionTerminal::Failed)
        );
        assert_eq!(
            remote_session_terminal(
                FlowSessionPhase::Closed,
                RuntimeLaunchState::Closed,
                Some("Closed"),
                None
            ),
            Some(RemoteSessionTerminal::Closed)
        );
        assert_eq!(
            RemoteSessionTerminal::Failed.error_code(),
            "remoteSessionFailed"
        );
    }

    #[test]
    fn remote_session_terminal_ignores_ready_and_recovering_states() {
        assert_eq!(
            remote_session_terminal(
                FlowSessionPhase::SessionReady,
                RuntimeLaunchState::Ready,
                Some("Provisioned"),
                None
            ),
            None
        );
        assert_eq!(
            remote_session_terminal(
                FlowSessionPhase::Recovering,
                RuntimeLaunchState::Blocked,
                Some("Recovering"),
                None
            ),
            None
        );
    }

    #[test]
    fn monitor_terminal_projection_rejects_stale_session_callbacks() {
        let projection = Arc::new(StdMutex::new(SessionFlowProjection::default()));
        let provider = IosSessionFlowProvider {
            console_ready_cache: Arc::new(Mutex::new(None)),
            credentials: Arc::new(SessionCredentials::new(
                "test-handle".into(),
                test_access(Target::Cloud),
            )),
            target: Target::Cloud,
            projection: Arc::clone(&projection),
        };
        provider.on_session_created(
            "current-session",
            "/v5/sessions/cloud/current-session",
            "cloud",
            "title-1",
            None,
        );
        let progress = SessionProgressSnapshot {
            session_id: "stale-session".to_string(),
            phase: FlowSessionPhase::Failed,
            runtime_launch_state: RuntimeLaunchState::Failed,
            status_text_key: "streamPage.errors.startFailed".to_string(),
            queue_seconds: None,
            queue: None,
            error_code: None,
            error_message: None,
            error_hint: None,
        };

        provider.on_session_monitor_tick(
            "stale-session",
            "cloud",
            "title-1",
            &progress,
            Some("Failed"),
            "failed",
            false,
            false,
        );
        assert_eq!(projection.lock().expect("projection").remote_terminal, None);

        provider.on_session_monitor_tick(
            "current-session",
            "cloud",
            "title-1",
            &progress,
            Some("Failed"),
            "failed",
            false,
            false,
        );
        assert_eq!(
            projection.lock().expect("projection").remote_terminal,
            Some(RemoteSessionTerminal::Failed)
        );
    }

    #[test]
    fn target_id_is_trimmed_and_validated() {
        assert_eq!(
            normalize_target_id("  ASDUSKFALLS ").unwrap(),
            "ASDUSKFALLS"
        );
        assert_eq!(normalize_target_id("stream-a").unwrap(), "stream-a");
        assert!(normalize_target_id("").is_err());
        assert!(normalize_target_id("title\ninvalid").is_err());
        assert!(normalize_target_id(&"x".repeat(257)).is_err());
    }

    #[test]
    fn console_text_is_trimmed_and_bounded_before_network_use() {
        assert_eq!(
            normalize_console_text("  hello xbox  ").unwrap(),
            "hello xbox"
        );
        assert_eq!(
            normalize_console_text(&"x".repeat(1_024)).unwrap().len(),
            1_024
        );
        assert!(normalize_console_text("   ").is_err());
        assert!(normalize_console_text("line\nbreak").is_err());
        assert!(normalize_console_text(&"x".repeat(1_025)).is_err());
    }

    #[test]
    fn control_plan_uses_domain_compiler_and_desktop_headers() {
        let plan = control_plan(
            &test_access(Target::Cloud),
            "title-1",
            &XboxStreamSettings::default(),
        )
        .expect("plan");
        assert_eq!(plan.session.target, Target::Cloud);
        assert_eq!(plan.session.target_id, "title-1");
        assert_eq!(plan.session.base_url, "https://cloud.example.com");
        assert!(plan.session.headers.contains_key("x-ms-device-info"));
        assert!(plan.session.headers.contains_key("User-Agent"));
        assert_eq!(plan.session.settings.sdk_type, "web");
    }

    #[test]
    fn ios_pc_ranges_reach_shared_plan_for_home_1440_and_zero_auto_bitrates() {
        let settings = XboxStreamSettings {
            preferred_game_locale: String::new(),
            home_resolution: 1440,
            home_bitrate_mode: "Custom".to_string(),
            home_bitrate_mbps: 0,
            cloud_bitrate_mode: "Custom".to_string(),
            cloud_bitrate_mbps: 0,
            audio_bitrate_mode: "Custom".to_string(),
            audio_bitrate_kbps: 0,
            ..XboxStreamSettings::default()
        };
        let access = test_access_for_account(Target::Home, "settings-pc-ranges");
        let plan = control_plan(&access, "console-command-id", &settings).expect("plan");
        let projection = project_web_rtc_plan(&plan);
        assert_eq!(projection.target_video_width, 2560);
        assert_eq!(projection.target_video_height, 1440);
        assert_eq!(plan.session.locale, "en-US");
        assert_eq!(projection.audio_bitrate_kbps, Some(128));
        assert!(projection.max_video_bitrate_kbps.is_some());
        assert_eq!(
            parse_video_bitrate_preference("Custom", 0),
            BitratePreference::Auto
        );
        assert_eq!(
            parse_audio_bitrate_preference("Custom", 0),
            BitratePreference::Auto
        );
    }

    #[test]
    fn clarity_preference_reaches_shared_input_and_ios_runtime_projection() {
        for target in [Target::Cloud, Target::Home] {
            for enabled in [false, true] {
                let settings = XboxStreamSettings {
                    prevent_resolution_drops: enabled,
                    ..XboxStreamSettings::default()
                };
                let access = test_access_for_account(target, "clarity-preference");
                let plan = control_plan(&access, "console-command-id", &settings).expect("plan");
                assert_eq!(plan.input.prevent_resolution_drops, enabled);
                assert_eq!(
                    project_web_rtc_plan(&plan).prevent_resolution_drops,
                    enabled
                );
            }
        }
    }

    #[test]
    fn web_rtc_plan_projects_receive_only_audio_codec_feedback_and_ice_policy() {
        let plan = control_plan(
            &test_access(Target::Cloud),
            "title-1",
            &XboxStreamSettings::default(),
        )
        .expect("plan");
        let projection = project_web_rtc_plan(&plan);
        assert_eq!(projection.audio_direction, "recvonly");
        assert_eq!(projection.video_direction, "recvonly");
        assert_eq!(projection.video_codec_mime_type, "video/H264");
        assert_eq!(projection.target_video_width, 1_280);
        assert_eq!(projection.target_video_height, 720);
        assert_eq!(projection.audio_bitrate_kbps, Some(128));
        assert_eq!(projection.h264_packetization_mode, 1);
        assert!(projection.h264_level_asymmetry_allowed);
        assert_eq!(projection.max_frame_size, 3_600);
        assert_eq!(projection.max_frame_rate, 60);
        assert_eq!(projection.min_video_bitrate_kbps, Some(3_000));
        assert_eq!(projection.start_video_bitrate_kbps, Some(10_000));
        assert_eq!(projection.max_video_bitrate_kbps, Some(10_000));
        assert_eq!(projection.required_video_rtcp_feedback.len(), 5);
        assert_eq!(
            projection.allowed_candidate_types,
            vec!["host", "srflx", "relay"]
        );
        assert_eq!(projection.ice_transport_policy, "all");
        assert!(projection.console_addresses.is_empty());
    }

    #[test]
    fn control_plan_builds_home_target_with_desktop_profile() {
        let plan = control_plan(
            &test_access(Target::Home),
            "console-command-id",
            &XboxStreamSettings::default(),
        )
        .expect("plan");
        assert_eq!(plan.session.target, Target::Home);
        assert_eq!(plan.session.target_id, "1234abcd");
        assert_eq!(plan.session.base_url, "https://cloud.example.com");
        assert!(plan.session.headers.contains_key("x-ms-device-info"));
        assert!(plan.session.headers.contains_key("User-Agent"));
        assert_eq!(plan.session.settings.sdk_type, "web");
        assert!(plan.negotiation.inject_console_addrs);
        assert_eq!(plan.negotiation.console_addrs[0].ip, "10.0.0.8");
        let web_rtc = project_web_rtc_plan(&plan);
        assert_eq!(web_rtc.target_video_width, 1_920);
        assert_eq!(web_rtc.target_video_height, 1_080);
        assert_eq!(web_rtc.audio_bitrate_kbps, Some(128));
        assert_eq!(web_rtc.max_frame_size, 8_160);
        assert_eq!(web_rtc.min_video_bitrate_kbps, Some(5_000));
        assert_eq!(web_rtc.start_video_bitrate_kbps, Some(20_000));
        assert_eq!(web_rtc.max_video_bitrate_kbps, Some(35_000));
        assert_eq!(
            web_rtc.console_addresses,
            vec![XboxHostAddress {
                ip: "10.0.0.8".to_string(),
                port: 9002,
            }]
        );
    }

    #[test]
    fn control_plan_projects_consumed_cloud_streaming_settings() {
        let settings = XboxStreamSettings {
            preferred_game_locale: "ja-JP".to_string(),
            cloud_resolution: 1440,
            prefer_ipv6: true,
            video_codec: "video/H264-64".to_string(),
            cloud_bitrate_mode: "Custom".to_string(),
            cloud_bitrate_mbps: 42,
            audio_bitrate_mode: "Custom".to_string(),
            audio_bitrate_kbps: 256,
            ..XboxStreamSettings::default()
        };

        let plan = control_plan(&test_access(Target::Cloud), "title-1", &settings).expect("plan");
        let projection = project_web_rtc_plan(&plan);

        assert_eq!(plan.session.locale, "ja-JP");
        assert_eq!(plan.session.settings.locale, "ja-JP");
        assert_eq!(projection.target_video_width, 2_560);
        assert_eq!(projection.target_video_height, 1_440);
        assert_eq!(projection.video_codec_mime_type, "video/H264");
        assert_eq!(projection.h264_profiles, vec!["64"]);
        assert_eq!(projection.audio_bitrate_kbps, Some(256));
        assert_eq!(projection.min_video_bitrate_kbps, Some(8_000));
        assert_eq!(projection.start_video_bitrate_kbps, Some(35_000));
        assert_eq!(projection.max_video_bitrate_kbps, Some(42_000));
        assert!(projection.prefer_ipv6);
    }

    #[test]
    fn control_plan_projects_consumed_home_streaming_settings() {
        let mut access = test_access_for_account(Target::Home, "settings-home");
        access.fallback_turn = Some(TurnServer {
            url: "turn:relay.example.com:3478".to_string(),
            username: "user".to_string(),
            credential: "credential".to_string(),
        });
        let settings = XboxStreamSettings {
            preferred_game_locale: "".to_string(),
            home_resolution: 720,
            prefer_ipv6: true,
            video_codec: "video/H264-420".to_string(),
            home_bitrate_mode: "Custom".to_string(),
            home_bitrate_mbps: 18,
            home_turn_fallback: true,
            ..XboxStreamSettings::default()
        };

        let plan = control_plan(&access, "console-command-id", &settings).expect("plan");
        let projection = project_web_rtc_plan(&plan);

        assert_eq!(plan.session.locale, "en-US");
        assert_eq!(plan.session.settings.locale, "en-US");
        assert_eq!(plan.runtime.turn.source, TurnSource::Fallback);
        assert_eq!(projection.target_video_width, 1_280);
        assert_eq!(projection.target_video_height, 720);
        assert_eq!(projection.video_codec_mime_type, "video/H264");
        assert_eq!(projection.h264_profiles, vec!["420"]);
        assert_eq!(projection.min_video_bitrate_kbps, Some(3_000));
        assert_eq!(projection.start_video_bitrate_kbps, Some(10_000));
        assert_eq!(projection.max_video_bitrate_kbps, Some(18_000));
        assert!(projection.prefer_ipv6);
    }

    #[test]
    fn custom_turn_is_validated_and_overrides_home_fallback() {
        let mut access = test_access_for_account(Target::Home, "custom-turn-settings");
        access.fallback_turn = Some(TurnServer {
            url: "turn:fallback.example.com:3478".into(),
            username: "fallback".into(),
            credential: "fallback".into(),
        });
        let mut settings = XboxStreamSettings {
            custom_turn: Some(XboxCustomTurn {
                server_url: "turns:custom.example.com:5349?transport=tcp".into(),
                username: "user".into(),
                credential: "secret".into(),
            }),
            ..Default::default()
        };
        let plan = control_plan(&access, "console-command-id", &settings).expect("custom plan");
        assert_eq!(plan.runtime.turn.source, TurnSource::Custom);
        assert_eq!(
            resolve_ice_servers(&plan)[1].username.as_deref(),
            Some("user")
        );
        for uri in [
            "https:relay.example.com",
            "turn:",
            "turn://relay.example.com",
            "turn:user@relay.example.com",
            "turn:relay.example.com:0",
            "turn:relay.example.com/path",
            "turn:relay.example.com?transport=bad",
        ] {
            settings.custom_turn.as_mut().unwrap().server_url = uri.into();
            assert!(
                control_plan(&access, "console-command-id", &settings).is_err(),
                "{uri}"
            );
        }
    }

    #[test]
    fn home_plan_rejects_explicitly_disabled_streaming_capability() {
        let access = test_access_for_account(Target::Home, "capability-disabled");
        cache_home_fixture("capability-disabled", Some(true), Some(false));
        let error = control_plan(&access, "1234abcd", &XboxStreamSettings::default())
            .expect_err("capability rejection");
        assert!(error.to_string().contains("homeRemotePlayUnavailable"));
    }

    #[test]
    fn home_plan_projects_fallback_turn_into_ice_servers() {
        let mut access = test_access_for_account(Target::Home, "turn-fixture");
        access.fallback_turn = Some(TurnServer {
            url: "turn:relay.example.com:3478".to_string(),
            username: "user".to_string(),
            credential: "credential".to_string(),
        });

        let default_plan =
            control_plan(&access, "console-device-id", &XboxStreamSettings::default())
                .expect("default home plan");
        assert_eq!(default_plan.runtime.turn.source, TurnSource::None);
        assert_eq!(resolve_ice_servers(&default_plan).len(), 1);
        let settings = XboxStreamSettings {
            home_turn_fallback: true,
            ..Default::default()
        };
        let plan = control_plan(&access, "console-device-id", &settings).expect("relay home plan");
        assert_eq!(plan.runtime.turn.source, TurnSource::Fallback);
        assert_eq!(plan.session.target_id, "1234abcd");
        let ice_servers = resolve_ice_servers(&plan);
        assert_eq!(ice_servers.len(), 2);
        assert_eq!(ice_servers[0].urls, vec![OFFICIAL_XBOX_STUN_URL]);
        assert_eq!(ice_servers[1].urls, vec!["turn:relay.example.com:3478"]);
        let (direct, fallback) = prepare_runtime_ice_servers(&plan);
        assert_eq!(direct.len(), 1);
        assert_eq!(fallback.unwrap().len(), 2);
        let (direct, fallback) = prepare_runtime_ice_servers(&default_plan);
        assert_eq!(direct.len(), 1);
        assert!(fallback.is_none());
    }

    #[test]
    fn cloud_and_home_share_the_same_rust_session_flow_adapter() {
        let cloud = test_session_for_target(Target::Cloud);
        let home = test_session_for_target(Target::Home);

        assert_eq!(cloud.plan_snapshot().unwrap().session.target, Target::Cloud);
        assert_eq!(home.plan_snapshot().unwrap().session.target, Target::Home);
        assert_eq!(
            std::any::type_name_of_val(&cloud.flow),
            std::any::type_name_of_val(&home.flow)
        );
    }

    #[test]
    fn remote_ice_completion_uses_connected_and_current_configuration_facts() {
        let mut state = SessionState::new();
        let first = project_remote_ice_batch(&mut state, vec![candidate("candidate:remote")], true);
        assert_eq!(first.candidates.len(), 1);
        assert!(first.configuration_pending);
        for _ in 0..50 {
            let batch =
                project_remote_ice_batch(&mut state, vec![candidate("candidate:remote")], true);
            assert!(batch.candidates.is_empty());
            assert!(!batch.end_of_candidates);
        }
        state.phase = SessionPhase::Connected;
        assert!(!project_remote_ice_batch(&mut state, Vec::new(), true).end_of_candidates);
        assert!(project_remote_ice_batch(&mut state, Vec::new(), false).end_of_candidates);
    }

    #[test]
    fn remote_ice_preserves_shared_candidate_binding_source_and_end_marker() {
        let mut state = SessionState::new();
        let mut remote =
            candidate("a=candidate:remote 1 UDP 2130706431 203.0.113.10 9000 typ host");
        remote.source = Some("session-configuration".into());
        remote.sdp_mid = Some("video".into());
        remote.sdp_m_line_index = Some(1);
        remote.username_fragment = Some("current-ufrag".into());
        let mut end = remote.clone();
        end.candidate = "end-of-candidates".into();
        let batch = project_remote_ice_batch(&mut state, vec![remote.clone(), end], true);
        assert_eq!(batch.candidates.len(), 2);
        assert_eq!(
            batch.candidates[0].source.as_deref(),
            Some("session-configuration")
        );
        assert_eq!(batch.candidates[0].sdp_mid.as_deref(), Some("video"));
        assert_eq!(batch.candidates[0].sdp_m_line_index, Some(1));
        assert_eq!(
            batch.candidates[0].username_fragment.as_deref(),
            Some("current-ufrag")
        );
        assert!(!batch.end_of_candidates);
        assert!(project_remote_ice_batch(&mut state, Vec::new(), false).end_of_candidates);
        state.reset_remote_ice();
        let replay = project_remote_ice_batch(&mut state, vec![remote], false);
        assert_eq!(replay.candidates.len(), 1);
    }

    #[test]
    fn second_offer_starts_restart_generation_and_resets_remote_ice() {
        let mut state = SessionState::new();
        assert!(!state.begin_offer_exchange());
        assert_eq!(state.signaling_epoch, 1);
        state.complete_offer_exchange();
        let first_ice = project_remote_ice_batch(
            &mut state,
            vec![
                candidate("candidate:first"),
                candidate("a=end-of-candidates"),
            ],
            false,
        );
        assert!(first_ice.end_of_candidates);
        assert_eq!(state.remote_ice.len(), 2);

        assert!(state.begin_offer_exchange());
        assert_eq!(state.signaling_epoch, 2);
        assert!(state.active_signaling_restart);
        assert!(state.remote_ice.is_empty());
        assert!(state.remote_ice_next_poll_at.is_none());
        assert!(!state.remote_ice_end_seen);
        assert!(!state.remote_ice_observed);
        assert!(!state.remote_ice_complete);
    }

    #[test]
    fn failed_restart_offer_keeps_rebuild_on_same_restart_generation() {
        let mut state = SessionState::new();
        assert!(!state.begin_offer_exchange());
        assert_eq!(state.signaling_epoch, 1);
        state.complete_offer_exchange();

        assert!(state.begin_offer_exchange());
        assert_eq!(state.signaling_epoch, 2);
        let failed_restart_count = state.completed_offer_exchanges;
        state.remote_ice.push(candidate("candidate:stale-restart"));
        state.remote_ice_complete = true;

        assert!(state.begin_offer_exchange());
        assert_eq!(state.signaling_epoch, 3);
        assert_eq!(state.completed_offer_exchanges, failed_restart_count);
        assert!(state.active_signaling_restart);
        assert!(state.remote_ice.is_empty());
        assert!(!state.remote_ice_complete);
    }

    #[test]
    fn explicit_remote_ice_end_is_projected_with_fresh_candidates() {
        let mut state = SessionState::new();
        let batch = project_remote_ice_batch(
            &mut state,
            vec![
                candidate("candidate:remote"),
                candidate("a=end-of-candidates"),
            ],
            false,
        );

        assert_eq!(batch.candidates.len(), 2);
        assert!(batch.end_of_candidates);
        assert!(state.remote_ice_complete);
    }

    #[test]
    fn ios_snapshot_implements_domain_runtime_binding() {
        let mut snapshot = IosSessionSnapshot::new_pending(
            "session-1".to_string(),
            "/v5/sessions/cloud/session-1".to_string(),
            "title-1".to_string(),
            "cloud".to_string(),
        );
        let mut runtime = snapshot.runtime_snapshot();
        runtime.stream_state = Some("Provisioned".to_string());
        runtime.player_state = "started".to_string();
        snapshot.replace_runtime_snapshot(runtime);
        assert_eq!(
            snapshot.runtime.stream_state.as_deref(),
            Some("Provisioned")
        );
        assert_eq!(snapshot.runtime.player_state, "started");
    }

    #[test]
    fn official_stun_is_platform_default() {
        let plan = control_plan(
            &test_access(Target::Cloud),
            "title-1",
            &XboxStreamSettings::default(),
        )
        .expect("plan");
        let servers = resolve_ice_servers(&plan);
        assert_eq!(servers[0].urls, vec![OFFICIAL_XBOX_STUN_URL]);
    }

    #[tokio::test]
    async fn cancel_invalidates_generation() {
        let session = test_session();
        let generation = session.begin_generation().await.expect("generation");
        session.cancel().await;
        assert!(matches!(
            ensure_generation(&session.generation, generation),
            Err(XboxStreamingError::Cancelled(_))
        ));
        assert_eq!(session.state.lock().await.phase, SessionPhase::Failed);
    }

    #[tokio::test]
    async fn mark_connected_is_idempotent() {
        let session = test_session();
        session.generation.store(1, Ordering::Release);
        {
            let mut state = session.state.lock().await;
            state.generation = 1;
            state.phase = SessionPhase::Negotiating;
        }
        if let Ok(mut projection) = session.projection.lock() {
            projection.session_id = Some("session-1".to_string());
        }

        session.mark_connected(1).await.expect("first callback");
        session.mark_connected(1).await.expect("duplicate callback");
        assert_eq!(session.state.lock().await.phase, SessionPhase::Connected);
    }
}
