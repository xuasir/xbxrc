use crate::mods::streaming::api_provider::StreamingApiProvider;
use crate::mods::streaming::auth_bridge::AuthServiceBridge;
use crate::mods::streaming::config_bridge::ConfigServiceBridge;
use crate::mods::streaming::fallback_turn_server_provider::FallbackTurnServerProvider;
use crate::mods::streaming::http_client::StreamingHttpError;
use crate::mods::streaming::types::*;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::AppHandle;

const SESSION_MONITOR_INTERVAL_MS: u64 = 1000;
const SESSION_STALL_TIMEOUT_MS: u64 = 45_000;

#[derive(Clone)]
pub struct StreamingService {
    inner: Arc<StreamingServiceInner>,
}

struct StreamingServiceInner {
    auth_bridge: AuthServiceBridge,
    api_provider: StreamingApiProvider,
    fallback_turn_server_provider: tokio::sync::Mutex<FallbackTurnServerProvider>,
    sessions: tokio::sync::RwLock<HashMap<String, StreamingSessionRecord>>,
}

#[derive(Clone)]
struct StreamingSessionRecord {
    snapshot: StreamingSessionSnapshot,
    created_at_ms: u64,
    last_observed_state: Option<String>,
    state_observed_at_ms: Option<u64>,
    repeated_state_count: u32,
    monitor_attempt_count: u32,
    cancelled: Arc<AtomicBool>,
}

impl StreamingService {
    pub fn new(app_handle: AppHandle) -> Self {
        Self {
            inner: Arc::new(StreamingServiceInner {
                auth_bridge: AuthServiceBridge::new(app_handle.clone()),
                api_provider: StreamingApiProvider::new(ConfigServiceBridge::new(app_handle)),
                fallback_turn_server_provider: tokio::sync::Mutex::new(
                    FallbackTurnServerProvider::new(),
                ),
                sessions: tokio::sync::RwLock::new(HashMap::new()),
            }),
        }
    }

    pub async fn get_fallback_turn_server(
        &self,
        target_type: &str,
    ) -> Result<Option<StreamingTurnServerConfig>, String> {
        let mut provider = self.inner.fallback_turn_server_provider.lock().await;
        provider.get_by_target_type(target_type).await
    }

    pub async fn create_session(
        &self,
        params: StreamingCreateSessionParams,
    ) -> Result<StreamingSessionSnapshot, String> {
        let target_type = StreamingTargetType::from_value(&params.target_type);
        let session_api = self.create_session_api(target_type.as_str()).await?;
        let session_path = session_api
            .start_stream(&params.target_id)
            .await
            .map_err(to_err)?;

        let session_id = session_path
            .split('/')
            .nth(3)
            .map(|value| value.to_string())
            .filter(|value| !value.is_empty())
            .ok_or("Streaming session id is missing")?;

        let snapshot = StreamingSessionSnapshot {
            id: session_id.clone(),
            target_id: params.target_id,
            path: session_path,
            target_type: target_type.as_str().to_string(),
            stream_state: None,
            player_state: "pending".to_string(),
            queue: None,
            error_details: None,
        };

        let cancelled = Arc::new(AtomicBool::new(false));
        let record = StreamingSessionRecord {
            snapshot: snapshot.clone(),
            created_at_ms: now_ms(),
            last_observed_state: None,
            state_observed_at_ms: None,
            repeated_state_count: 0,
            monitor_attempt_count: 0,
            cancelled: cancelled.clone(),
        };

        self.inner
            .sessions
            .write()
            .await
            .insert(session_id.clone(), record);

        let service = self.clone();
        tauri::async_runtime::spawn(async move {
            service.monitor_session_loop(session_id, cancelled).await;
        });

        Ok(snapshot)
    }

    pub async fn get_session(
        &self,
        params: StreamingGetSessionParams,
    ) -> Result<Option<StreamingSessionSnapshot>, String> {
        let sessions = self.inner.sessions.read().await;
        Ok(sessions
            .get(&params.session_id)
            .map(|record| record.snapshot.clone()))
    }

    pub async fn close_session(
        &self,
        params: StreamingCloseSessionParams,
    ) -> Result<serde_json::Value, String> {
        let session = {
            let sessions = self.inner.sessions.read().await;
            sessions.get(&params.session_id).cloned()
        };

        let Some(session) = session else {
            return Ok(serde_json::json!({ "closed": false }));
        };

        let session_api = self
            .create_session_api(&session.snapshot.target_type)
            .await?;
        let result = session_api.stop_stream(&params.session_id).await;

        self.clear_session(&params.session_id).await;

        match result {
            Ok(_) => Ok(serde_json::json!({ "closed": true })),
            Err(error) if error.status == Some(404) => Ok(serde_json::json!({ "closed": false })),
            Err(error) => Err(to_err(error)),
        }
    }

    pub async fn exchange_offer(
        &self,
        params: StreamingExchangeOfferParams,
    ) -> Result<StreamingExchangeOfferResult, String> {
        let session = self
            .get_session_record(&params.session_id)
            .await
            .ok_or_else(|| format!("Session not found: {}", params.session_id))?;
        let signaling_api = self
            .create_signaling_api(&session.snapshot.target_type)
            .await?;

        if params.channel.as_deref() == Some("chat") {
            signaling_api
                .send_chat_sdp(&params.session_id, &params.sdp)
                .await
                .map_err(to_err)?;
        } else {
            signaling_api
                .send_sdp(&params.session_id, &params.sdp)
                .await
                .map_err(to_err)?;
        }

        loop {
            let answer = signaling_api
                .get_sdp_exchange_response(&params.session_id)
                .await
                .map_err(to_err)?;
            if let Some(answer) = answer {
                return Ok(StreamingExchangeOfferResult { answer });
            }

            if self.get_session_record(&params.session_id).await.is_none() {
                return Err(format!("Session not found: {}", params.session_id));
            }

            tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
        }
    }

    pub async fn exchange_offer_sdp(
        &self,
        session_id: String,
        channel: Option<String>,
        sdp: String,
    ) -> Result<String, String> {
        let result = self
            .exchange_offer(StreamingExchangeOfferParams {
                session_id,
                channel,
                sdp,
            })
            .await?;
        Ok(result.answer.sdp)
    }

    pub async fn exchange_ice(
        &self,
        params: StreamingExchangeIceParams,
    ) -> Result<StreamingExchangeIceResult, String> {
        let session = self
            .get_session_record(&params.session_id)
            .await
            .ok_or_else(|| format!("Session not found: {}", params.session_id))?;
        let signaling_api = self
            .create_signaling_api(&session.snapshot.target_type)
            .await?;

        signaling_api
            .send_ice(&params.session_id, &params.candidate)
            .await
            .map_err(to_err)?;

        loop {
            let candidates = signaling_api
                .get_ice_exchange_response(&params.session_id)
                .await
                .map_err(to_err)?;

            if let Some(candidates) = candidates {
                if has_usable_ice_candidates(&candidates) {
                    return Ok(StreamingExchangeIceResult { candidates });
                }
            }

            if self.get_session_record(&params.session_id).await.is_none() {
                return Err(format!("Session not found: {}", params.session_id));
            }

            tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
        }
    }

    pub async fn exchange_ice_candidates(
        &self,
        session_id: String,
        candidates: Vec<StreamingIceCandidate>,
    ) -> Result<Vec<StreamingIceCandidate>, String> {
        let result = self
            .exchange_ice(StreamingExchangeIceParams {
                session_id,
                candidate: candidates,
            })
            .await?;
        Ok(result.candidates)
    }

    pub async fn send_keepalive(
        &self,
        params: StreamingKeepAliveParams,
    ) -> Result<StreamingKeepAliveResult, String> {
        let session = self.get_session_record(&params.session_id).await;
        let Some(session) = session else {
            return Ok(StreamingKeepAliveResult { accepted: false });
        };

        let session_api = self
            .create_session_api(&session.snapshot.target_type)
            .await?;
        match session_api.send_keepalive(&params.session_id).await {
            Ok(_) => Ok(StreamingKeepAliveResult { accepted: true }),
            Err(error) if should_ignore_keepalive_error(&error) => {
                Ok(StreamingKeepAliveResult { accepted: false })
            }
            Err(error) => Err(to_err(error)),
        }
    }

    pub async fn keep_alive_remote_session(&self, session_id: String) -> Result<bool, String> {
        let result = self
            .send_keepalive(StreamingKeepAliveParams { session_id })
            .await?;
        Ok(result.accepted)
    }

    pub async fn close_remote_session(&self, session_id: String) -> Result<bool, String> {
        let result = self
            .close_session(StreamingCloseSessionParams { session_id })
            .await?;

        Ok(result
            .get("closed")
            .and_then(|value| value.as_bool())
            .unwrap_or(false))
    }

    // 进程退出前 best-effort 关闭会话，避免服务端残留活跃会话。
    pub async fn shutdown(&self) {
        let session_ids = {
            let sessions = self.inner.sessions.read().await;
            sessions.keys().cloned().collect::<Vec<_>>()
        };

        for session_id in session_ids {
            let _ = self
                .close_session(StreamingCloseSessionParams {
                    session_id: session_id.clone(),
                })
                .await;
            self.clear_session(&session_id).await;
        }
    }

    pub async fn list_active_sessions(
        &self,
        params: StreamingListActiveSessionsParams,
    ) -> Result<StreamingListActiveSessionsResult, String> {
        let target_type = params.target_type.unwrap_or_else(|| "cloud".to_string());
        let sessions = self
            .inner
            .sessions
            .read()
            .await
            .values()
            .filter(|session| session.snapshot.target_type == target_type)
            .map(|session| session.snapshot.clone())
            .collect::<Vec<_>>();

        Ok(StreamingListActiveSessionsResult { sessions })
    }

    async fn monitor_session_loop(&self, session_id: String, cancelled: Arc<AtomicBool>) {
        loop {
            if cancelled.load(Ordering::Relaxed) {
                return;
            }

            let should_continue = self.monitor_session_tick(&session_id).await;
            if !should_continue {
                return;
            }

            tokio::time::sleep(std::time::Duration::from_millis(
                SESSION_MONITOR_INTERVAL_MS,
            ))
            .await;
        }
    }

    async fn monitor_session_tick(&self, session_id: &str) -> bool {
        let mut session = match self.get_session_record(session_id).await {
            Some(session) => session,
            None => return false,
        };

        let session_api = match self.create_session_api(&session.snapshot.target_type).await {
            Ok(api) => api,
            Err(_) => return true,
        };

        let state_response = session_api.get_stream_state(session_id).await;
        let (state, error_details) = match state_response {
            Ok(value) => value,
            Err(error) if error.status == Some(404) => {
                self.clear_session(session_id).await;
                return false;
            }
            Err(_) => return true,
        };

        session.monitor_attempt_count += 1;
        if session.last_observed_state == state {
            session.repeated_state_count += 1;
        } else {
            session.last_observed_state = state.clone();
            session.state_observed_at_ms = Some(now_ms());
            session.repeated_state_count = 1;
        }

        if let Some(timeout_error) = get_state_timeout_error(&session, state.as_deref()) {
            session.snapshot.player_state = "failed".to_string();
            session.snapshot.stream_state = state;
            session.snapshot.error_details = Some(timeout_error);
            self.upsert_session(session_id, session).await;
            return false;
        }

        match state.as_deref() {
            Some("Provisioned") => {
                session.snapshot.player_state = "started".to_string();
                session.snapshot.stream_state = state;
                session.snapshot.queue = None;
                session.snapshot.error_details = None;
                self.upsert_session(session_id, session).await;
                false
            }
            Some("Provisioning") => {
                session.snapshot.player_state = "pending".to_string();
                session.snapshot.stream_state = state;
                session.snapshot.error_details = None;
                self.upsert_session(session_id, session).await;
                true
            }
            Some("ReadyToConnect") => {
                session.snapshot.player_state = "pending".to_string();
                session.snapshot.stream_state = state;
                session.snapshot.error_details = None;
                self.upsert_session(session_id, session.clone()).await;

                let transfer_token = match self.inner.auth_bridge.get_transfer_token().await {
                    Ok(token) => token,
                    Err(_) => return true,
                };

                let _ = session_api
                    .send_connect_token(session_id, &transfer_token)
                    .await;
                true
            }
            Some("WaitingForResources") => {
                let queue = match session_api
                    .get_waiting_times(&session.snapshot.target_id)
                    .await
                {
                    Ok(queue) => StreamingQueueSnapshot { details: queue },
                    Err(_) => session.snapshot.queue.unwrap_or(StreamingQueueSnapshot {
                        details: StreamingQueueDetails::default(),
                    }),
                };

                session.snapshot.player_state = "queued".to_string();
                session.snapshot.stream_state = state;
                session.snapshot.queue = Some(queue);
                session.snapshot.error_details = None;
                self.upsert_session(session_id, session).await;
                true
            }
            Some("Failed") => {
                session.snapshot.player_state = "failed".to_string();
                session.snapshot.stream_state = state;
                session.snapshot.error_details = error_details;
                self.upsert_session(session_id, session).await;
                false
            }
            _ => {
                session.snapshot.player_state = "pending".to_string();
                session.snapshot.stream_state = state;
                self.upsert_session(session_id, session).await;
                true
            }
        }
    }

    async fn create_session_api(
        &self,
        target_type: &str,
    ) -> Result<crate::mods::streaming::session_api::StreamingSessionApi, String> {
        let token = self
            .inner
            .auth_bridge
            .get_streaming_token(target_type)
            .await?
            .ok_or_else(|| format!("Streaming token is unavailable for {target_type}"))?;

        self.inner
            .api_provider
            .create_session_api(&token, target_type)
            .await
    }

    async fn create_signaling_api(
        &self,
        target_type: &str,
    ) -> Result<crate::mods::streaming::signaling_api::StreamingSignalingApi, String> {
        let token = self
            .inner
            .auth_bridge
            .get_streaming_token(target_type)
            .await?
            .ok_or_else(|| format!("Streaming token is unavailable for {target_type}"))?;

        self.inner
            .api_provider
            .create_signaling_api(&token, target_type)
            .await
    }

    async fn get_session_record(&self, session_id: &str) -> Option<StreamingSessionRecord> {
        let sessions = self.inner.sessions.read().await;
        sessions.get(session_id).cloned()
    }

    async fn upsert_session(&self, session_id: &str, record: StreamingSessionRecord) {
        self.inner
            .sessions
            .write()
            .await
            .insert(session_id.to_string(), record);
    }

    async fn clear_session(&self, session_id: &str) {
        let removed = self.inner.sessions.write().await.remove(session_id);
        if let Some(record) = removed {
            record.cancelled.store(true, Ordering::Relaxed);
        }
    }
}

fn has_usable_ice_candidates(candidates: &[StreamingIceCandidate]) -> bool {
    candidates.iter().any(|candidate| {
        let normalized = candidate.candidate.trim();
        !normalized.is_empty() && normalized != "a=end-of-candidates"
    })
}

fn get_state_timeout_error(
    session: &StreamingSessionRecord,
    state: Option<&str>,
) -> Option<StreamingErrorDetails> {
    if state != Some("Provisioning") && state != Some("ReadyToConnect") {
        return None;
    }

    let started = session
        .state_observed_at_ms
        .unwrap_or(session.created_at_ms);
    let elapsed = now_ms().saturating_sub(started);
    if elapsed < SESSION_STALL_TIMEOUT_MS {
        return None;
    }

    Some(StreamingErrorDetails {
        code: Some(Value::String("SessionStateTimeout".to_string())),
        message: Some(format!(
            "Streaming session stayed in {} for {}ms.",
            state.unwrap_or("unknown"),
            elapsed
        )),
    })
}

fn should_ignore_keepalive_error(error: &StreamingHttpError) -> bool {
    if error.status == Some(404) {
        return true;
    }

    if error.status != Some(400) {
        return false;
    }

    let Some(body) = &error.body else {
        return false;
    };

    let parsed = serde_json::from_str::<Value>(body).unwrap_or(Value::Null);
    let code = parsed
        .get("code")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let message = parsed
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_default();

    code == "SessionUnexpectedState"
        && (message.contains("ServerSdpExchangeCommandSent") || message.contains("UnexpectedState"))
}

fn to_err(error: StreamingHttpError) -> String {
    match (error.status, error.body) {
        (Some(status), Some(body)) => {
            format!("{} (status: {}, body: {})", error.message, status, body)
        }
        (Some(status), None) => format!("{} (status: {})", error.message, status),
        _ => error.message,
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}
