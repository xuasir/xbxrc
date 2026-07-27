use crate::cloud_access::{
    find_home_host_facts, home_host_facts, load_scoped_stream_access, load_stream_access,
    replace_home_host_facts, HomeHostFacts, StreamingAccessContext,
};
use crate::data::extract_home_host_facts;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::net::IpAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use xbox_auth_flow::{AuthFlow, TransferTokenInput};
use xbox_streaming::runtime::{RuntimeCapabilities, TurnContext};
use xbox_streaming::{
    compile, parse_resolution_preference, BitratePreference, CodecPreference, CompilerInput,
    Config, Context, IceCandidate, Plan, RemotePlayContext, RuntimeLaunchState, SessionFlowError,
    SessionFlowProvider, SessionFlowService, SessionFlowSnapshot, SessionPhase as FlowSessionPhase,
    SessionProgressSnapshot, SessionRuntimeBinding, SessionRuntimeSnapshot, Target,
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

const OFFICIAL_XBOX_STUN_URL: &str = "stun:relay.communication.microsoft.com:3478";
const REMOTE_ICE_APPLICATION_WINDOW: Duration = Duration::from_secs(30);
const REMOTE_ICE_EMPTY_POLLS_AFTER_FIRST_CANDIDATE: u32 = 10;

#[derive(Debug, Clone, thiserror::Error, uniffi::Error)]
pub enum XboxStreamingError {
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
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct XboxStreamSettings {
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
}

impl Default for XboxStreamSettings {
    fn default() -> Self {
        Self {
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
            home_turn_fallback: true,
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
    session_id: Option<String>,
    remote_terminal: Option<RemoteSessionTerminal>,
}

#[derive(Clone)]
struct IosSessionFlowProvider {
    access: StreamingAccessContext,
    projection: Arc<StdMutex<SessionFlowProjection>>,
}

#[async_trait]
impl SessionFlowProvider for IosSessionFlowProvider {
    async fn get_streaming_token(&self, target_type: &str) -> Result<Value, SessionFlowError> {
        if target_type != self.access.target.as_str() {
            return Err(SessionFlowError::message(format!(
                "streamingTargetMismatch:expected={};actual={target_type}",
                self.access.target.as_str()
            )));
        }
        Ok(json!({ "gsToken": self.access.bearer_token }))
    }

    async fn transfer_token(&self) -> Result<String, SessionFlowError> {
        AuthFlow::new()
            .transfer_token(TransferTokenInput {
                refresh_token: self.access.refresh_token.clone(),
            })
            .await
            .map(|output| output.transfer_token)
            .map_err(|error| SessionFlowError::message(error.to_string()))
    }

    async fn power_on_console(&self, console_id: &str) -> Result<bool, SessionFlowError> {
        if self.access.target != Target::Home {
            return Ok(false);
        }
        let facts = find_home_host_facts(&self.access.account_id, console_id)
            .map_err(|error| SessionFlowError::message(error.to_string()))?
            .ok_or_else(|| SessionFlowError::message("homeTargetContextMissing"))?;
        let command_id = facts
            .command_id()
            .ok_or_else(|| SessionFlowError::message("homeCommandIdentityMissing"))?;
        ConsoleApi::new(self.access.web_uhs.clone(), self.access.web_token.clone())
            .power_on(command_id)
            .await
            .map(|response| response.accepted)
            .map_err(|error| SessionFlowError::message(error.to_string()))
    }

    async fn get_remote_consoles(
        &self,
    ) -> Result<Vec<xbox_streaming::RemoteConsoleSnapshot>, SessionFlowError> {
        if self.access.target != Target::Home {
            return Ok(Vec::new());
        }
        let smartglass =
            SmartglassApi::new(self.access.web_uhs.clone(), self.access.web_token.clone());
        let refreshed =
            tokio::time::timeout(Duration::from_secs(8), smartglass.get_consoles_list()).await;
        let facts = match refreshed {
            Ok(Ok(response)) => {
                let facts = extract_home_host_facts(&response);
                replace_home_host_facts(&self.access.account_id, facts.clone())
                    .map_err(|error| SessionFlowError::message(error.to_string()))?;
                facts
            }
            Ok(Err(error)) => {
                let cached = home_host_facts(&self.access.account_id)
                    .map_err(|cache_error| SessionFlowError::message(cache_error.to_string()))?;
                if cached.is_empty() {
                    return Err(SessionFlowError::message(error.to_string()));
                }
                cached
            }
            Err(_) => {
                let cached = home_host_facts(&self.access.account_id)
                    .map_err(|error| SessionFlowError::message(error.to_string()))?;
                if cached.is_empty() {
                    return Err(SessionFlowError::message("homeConsoleLookupTimeout"));
                }
                cached
            }
        };
        Ok(facts
            .into_iter()
            .map(|facts| facts.remote_console)
            .collect())
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
        let Some(terminal) = terminal else { return };
        if let Ok(mut projection) = self.projection.lock() {
            if projection.session_id.as_deref() == Some(session_id) {
                projection.remote_terminal.get_or_insert(terminal);
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
    remote_ice_started_at: Option<Instant>,
    remote_ice_empty_polls: u32,
    remote_ice_observed: bool,
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
            remote_ice_started_at: None,
            remote_ice_empty_polls: 0,
            remote_ice_observed: false,
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
        self.remote_ice_started_at = None;
        self.remote_ice_empty_polls = 0;
        self.remote_ice_observed = false;
        self.remote_ice_complete = false;
    }
}

type IosSessionFlow = SessionFlowService<IosSessionSnapshot, IosSessionFlowProvider>;

/// Swift 只转交 libwebrtc 产生的 offer/local ICE，并应用这里返回的 remote ICE。
/// 会话编排、轮询、connect token、keepalive、signaling 与结束判定全部由 Rust 持有。
#[derive(uniffi::Object)]
pub struct XboxStreamSession {
    plan: Plan,
    flow: IosSessionFlow,
    projection: Arc<StdMutex<SessionFlowProjection>>,
    state: Mutex<SessionState>,
    generation: AtomicU64,
}

#[uniffi::export]
pub fn create_stream_session(
    access_handle: String,
    target_id: String,
) -> Result<Arc<XboxStreamSession>, XboxStreamingError> {
    let target_id = normalize_target_id(&target_id)?;
    let access = load_stream_access(access_handle.trim())
        .map_err(|error| XboxStreamingError::Authentication(error.to_string()))?;
    build_stream_session(access, target_id, XboxStreamSettings::default())
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
    build_stream_session(access, target_id, settings)
}

fn build_stream_session(
    access: StreamingAccessContext,
    target_id: String,
    settings: XboxStreamSettings,
) -> Result<Arc<XboxStreamSession>, XboxStreamingError> {
    let plan = control_plan(&access, &target_id, &settings)?;
    let projection = Arc::new(StdMutex::new(SessionFlowProjection::default()));
    let flow = SessionFlowService::new(IosSessionFlowProvider {
        access,
        projection: Arc::clone(&projection),
    });

    Ok(Arc::new(XboxStreamSession {
        plan,
        flow,
        projection,
        state: Mutex::new(SessionState::new()),
        generation: AtomicU64::new(0),
    }))
}

#[uniffi::export(async_runtime = "tokio")]
impl XboxStreamSession {
    pub async fn start(&self) -> Result<XboxPreparedSignaling, XboxStreamingError> {
        let generation = self.begin_generation().await?;
        let execution = match self
            .flow
            .start_session_execution(self.plan.clone(), |_| (), |_| ())
            .await
        {
            Ok(execution) => execution,
            Err(error) => {
                self.mark_failed(generation).await?;
                return Err(map_session_flow_error(error));
            }
        };
        ensure_generation(&self.generation, generation)?;

        if let Ok(mut projection) = self.projection.lock() {
            projection.session_id = Some(execution.session.session_id.clone());
        }
        let mut state = self.state.lock().await;
        ensure_generation(&self.generation, generation)?;
        state.phase = SessionPhase::Ready;
        Ok(XboxPreparedSignaling {
            generation,
            ice_servers: resolve_ice_servers(&self.plan),
            web_rtc_plan: project_web_rtc_plan(&self.plan),
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
        let timeout_ms = self.plan.session.schedule.ready_timeout_ms.max(1_000);
        let answer = tokio::time::timeout(
            Duration::from_millis(timeout_ms),
            self.flow.exchange_offer(&session_id, None, &sdp, restart),
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

    pub async fn submit_ice(
        &self,
        generation: u64,
        candidates: Vec<XboxIceCandidate>,
    ) -> Result<(), XboxStreamingError> {
        let session_id = self.session_id_for(generation).await?;
        let candidates = candidates
            .into_iter()
            .map(|candidate| IceCandidate {
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
        self.flow
            .submit_ice(&session_id, &candidates, restart)
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
        let poll_interval =
            Duration::from_millis(self.plan.session.schedule.ice_poll_interval_ms.max(100));
        let (signaling_epoch, restart) = {
            let state = self.state.lock().await;
            ensure_generation(&self.generation, generation)?;
            self.ensure_remote_session_active(&session_id)?;
            (state.signaling_epoch, state.active_signaling_restart)
        };

        loop {
            ensure_generation(&self.generation, generation)?;
            {
                let mut state = self.state.lock().await;
                ensure_generation(&self.generation, generation)?;
                if state.signaling_epoch != signaling_epoch {
                    return Ok(XboxRemoteIceBatch {
                        candidates: Vec::new(),
                        end_of_candidates: true,
                    });
                }
                if state.remote_ice_complete {
                    return Ok(XboxRemoteIceBatch {
                        candidates: Vec::new(),
                        end_of_candidates: true,
                    });
                }
                let started_at = state.remote_ice_started_at.get_or_insert_with(Instant::now);
                if started_at.elapsed() >= REMOTE_ICE_APPLICATION_WINDOW {
                    state.remote_ice_complete = true;
                    return Ok(XboxRemoteIceBatch {
                        candidates: Vec::new(),
                        end_of_candidates: true,
                    });
                }
            }

            let candidates = self
                .flow
                .poll_ice(&session_id, restart)
                .await
                .map_err(map_session_flow_error)?;
            ensure_generation(&self.generation, generation)?;
            self.ensure_remote_session_active(&session_id)?;

            let mut state = self.state.lock().await;
            ensure_generation(&self.generation, generation)?;
            if state.signaling_epoch != signaling_epoch {
                return Ok(XboxRemoteIceBatch {
                    candidates: Vec::new(),
                    end_of_candidates: true,
                });
            }
            let batch = project_remote_ice_batch_for_plan(&mut state, &self.plan, candidates);
            drop(state);

            if !batch.candidates.is_empty() || batch.end_of_candidates {
                return Ok(batch);
            }

            tokio::time::sleep(poll_interval).await;
        }
    }

    pub async fn cancel(&self) -> u64 {
        let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        let mut state = self.state.lock().await;
        state.generation = generation;
        state.phase = SessionPhase::Failed;
        generation
    }

    pub async fn close(&self) -> Result<(), XboxStreamingError> {
        let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
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
            self.flow
                .close_session(&session_id)
                .await
                .map_err(map_session_flow_error)?;
        }
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
}

impl XboxStreamSession {
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
        state.phase = SessionPhase::Creating;
        state.reset_signaling();
        if let Ok(mut projection) = self.projection.lock() {
            *projection = SessionFlowProjection::default();
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
    config.session.preferred_game_locale = normalize_game_locale(&settings.preferred_game_locale);
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
                .map_err(|error| XboxStreamingError::InvalidArgument(error.to_string()))?
                .ok_or_else(|| {
                    XboxStreamingError::InvalidArgument("homeTargetContextMissing".to_string())
                })?;
            validate_home_host_capability(&facts)?;
            config.session.power_on = facts.remote_console.power_state.as_deref() != Some("On")
                && facts.remote_console.remote_management_enabled == Some(true);
            let canonical_target_id = facts
                .canonical_target_id()
                .ok_or_else(|| {
                    XboxStreamingError::InvalidArgument("homeTargetIdentityMissing".to_string())
                })?
                .to_string();
            (
                canonical_target_id,
                RemotePlayContext {
                    configuration_resolved: true,
                    remote_management_enabled: facts.remote_console.remote_management_enabled,
                    console_streaming_enabled: facts.remote_console.console_streaming_enabled,
                    console_addrs: facts.console_addrs,
                },
            )
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
        facts.remote_console.remote_management_enabled == Some(true)
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
        audio_direction: "sendrecv".to_string(),
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

fn console_address_ice_candidates(plan: &Plan) -> impl Iterator<Item = IceCandidate> + '_ {
    plan.negotiation
        .console_addrs
        .iter()
        .enumerate()
        .filter_map(|(index, address)| {
            address.ip.parse::<IpAddr>().ok()?;
            (address.port > 0).then(|| IceCandidate {
                candidate: format!(
                    "a=candidate:{} 1 UDP 1 {} {} typ host",
                    index.saturating_add(1),
                    address.ip,
                    address.port
                ),
                sdp_m_line_index: Some(0),
                sdp_mid: Some("0".to_string()),
                username_fragment: None,
                message_type: Some("iceCandidate".to_string()),
            })
        })
}

fn project_remote_ice_batch_for_plan(
    state: &mut SessionState,
    plan: &Plan,
    candidates: impl IntoIterator<Item = IceCandidate>,
) -> XboxRemoteIceBatch {
    project_remote_ice_batch(
        state,
        candidates
            .into_iter()
            .chain(console_address_ice_candidates(plan)),
    )
}

fn project_remote_ice_batch(
    state: &mut SessionState,
    candidates: impl IntoIterator<Item = IceCandidate>,
) -> XboxRemoteIceBatch {
    let mut fresh = Vec::new();
    let mut explicit_end = false;
    for candidate in candidates {
        if is_end_of_candidates(&candidate.candidate) {
            explicit_end = true;
            continue;
        }
        if !state
            .remote_ice
            .iter()
            .any(|existing| existing == &candidate)
        {
            state.remote_ice.push(candidate.clone());
            fresh.push(candidate);
        }
    }
    if fresh.is_empty() {
        state.remote_ice_empty_polls = state.remote_ice_empty_polls.saturating_add(1);
    } else {
        state.remote_ice_observed = true;
        state.remote_ice_empty_polls = 0;
    }
    let inferred_end = state.remote_ice_observed
        && state.remote_ice_empty_polls >= REMOTE_ICE_EMPTY_POLLS_AFTER_FIRST_CANDIDATE;
    let end_of_candidates = explicit_end || inferred_end;
    if end_of_candidates {
        state.remote_ice_complete = true;
    }
    XboxRemoteIceBatch {
        candidates: fresh.into_iter().map(to_bridge_candidate).collect(),
        end_of_candidates,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xbox_streaming::policy::session::SessionAccessContext;
    use xbox_streaming::{HostAddr, Region, RemoteConsoleSnapshot, TurnServer, TurnSource};

    fn test_access(target: Target) -> StreamingAccessContext {
        test_access_for_account(target, "xuid")
    }

    fn test_access_for_account(target: Target, account_id: &str) -> StreamingAccessContext {
        let access = StreamingAccessContext {
            host: "cloud.example.com".to_string(),
            bearer_token: "gs-token".to_string(),
            account_id: account_id.to_string(),
            refresh_token: "refresh-token".to_string(),
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
        build_stream_session(
            test_access(target),
            "1234abcd".to_string(),
            XboxStreamSettings::default(),
        )
        .expect("session")
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
            access: test_access(Target::Cloud),
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
    fn web_rtc_plan_projects_desktop_direction_codec_feedback_and_ice_policy() {
        let plan = control_plan(
            &test_access(Target::Cloud),
            "title-1",
            &XboxStreamSettings::default(),
        )
        .expect("plan");
        let projection = project_web_rtc_plan(&plan);
        assert_eq!(projection.audio_direction, "sendrecv");
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

        let plan = control_plan(&access, "console-device-id", &XboxStreamSettings::default())
            .expect("home plan");
        assert_eq!(plan.runtime.turn.source, TurnSource::Fallback);
        assert_eq!(plan.session.target_id, "1234abcd");
        let ice_servers = resolve_ice_servers(&plan);
        assert_eq!(ice_servers.len(), 2);
        assert_eq!(ice_servers[0].urls, vec![OFFICIAL_XBOX_STUN_URL]);
        assert_eq!(ice_servers[1].urls, vec!["turn:relay.example.com:3478"]);
    }

    #[test]
    fn cloud_and_home_share_the_same_rust_session_flow_adapter() {
        let cloud = test_session_for_target(Target::Cloud);
        let home = test_session_for_target(Target::Home);

        assert_eq!(cloud.plan.session.target, Target::Cloud);
        assert_eq!(home.plan.session.target, Target::Home);
        assert_eq!(
            std::any::type_name_of_val(&cloud.flow),
            std::any::type_name_of_val(&home.flow)
        );
    }

    #[test]
    fn rust_owns_remote_ice_deduplication_and_completion() {
        let mut state = SessionState::new();
        let first = project_remote_ice_batch(&mut state, vec![candidate("candidate:remote")]);
        assert_eq!(first.candidates.len(), 1);
        assert!(!first.end_of_candidates);

        let duplicate = project_remote_ice_batch(&mut state, vec![candidate("candidate:remote")]);
        assert!(duplicate.candidates.is_empty());
        assert!(!duplicate.end_of_candidates);

        let mut final_batch = duplicate;
        for _ in 1..REMOTE_ICE_EMPTY_POLLS_AFTER_FIRST_CANDIDATE {
            final_batch = project_remote_ice_batch(&mut state, Vec::new());
        }
        assert!(final_batch.candidates.is_empty());
        assert!(final_batch.end_of_candidates);
        assert!(state.remote_ice_complete);
    }

    #[test]
    fn home_console_addresses_are_injected_once_per_signaling_epoch() {
        let plan = control_plan(
            &test_access(Target::Home),
            "console-command-id",
            &XboxStreamSettings::default(),
        )
        .expect("plan");
        let mut state = SessionState::new();
        assert!(!state.begin_offer_exchange());

        let first = project_remote_ice_batch_for_plan(
            &mut state,
            &plan,
            std::iter::once(candidate(
                "a=candidate:remote 1 UDP 2130706431 203.0.113.10 9000 typ host",
            )),
        );
        assert_eq!(first.candidates.len(), 2);
        assert_eq!(
            first.candidates[1].candidate,
            "a=candidate:1 1 UDP 1 10.0.0.8 9002 typ host"
        );
        assert_eq!(first.candidates[1].sdp_m_line_index, Some(0));
        assert_eq!(first.candidates[1].sdp_mid.as_deref(), Some("0"));
        assert_eq!(
            first.candidates[1].message_type.as_deref(),
            Some("iceCandidate")
        );

        let duplicate = project_remote_ice_batch_for_plan(&mut state, &plan, Vec::new());
        assert!(duplicate.candidates.is_empty());

        state.complete_offer_exchange();
        assert!(state.begin_offer_exchange());
        let replay = project_remote_ice_batch_for_plan(&mut state, &plan, Vec::new());
        assert_eq!(replay.candidates.len(), 1);
        assert_eq!(
            replay.candidates[0].candidate,
            "a=candidate:1 1 UDP 1 10.0.0.8 9002 typ host"
        );
    }

    #[test]
    fn cloud_plan_does_not_inject_console_address_candidates() {
        let plan = control_plan(
            &test_access(Target::Cloud),
            "title-1",
            &XboxStreamSettings::default(),
        )
        .expect("plan");
        assert_eq!(console_address_ice_candidates(&plan).count(), 0);
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
        );
        assert!(first_ice.end_of_candidates);
        assert_eq!(state.remote_ice.len(), 1);

        assert!(state.begin_offer_exchange());
        assert_eq!(state.signaling_epoch, 2);
        assert!(state.active_signaling_restart);
        assert!(state.remote_ice.is_empty());
        assert!(state.remote_ice_started_at.is_none());
        assert_eq!(state.remote_ice_empty_polls, 0);
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
        );

        assert_eq!(batch.candidates.len(), 1);
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
