use crate::error::AppResult;
use crate::mods::streaming::types::*;
use crate::AppState;
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

#[derive(Debug, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "camelCase")]
pub enum StreamingCommand {
    GetFallbackTurnServer { target_type: String },
    CreateSession(StreamingCreateSessionParams),
    GetSession(StreamingGetSessionParams),
    CloseSession(StreamingCloseSessionParams),
    ExchangeOffer(StreamingExchangeOfferParams),
    ExchangeIce(StreamingExchangeIceParams),
    SendKeepAlive(StreamingKeepAliveParams),
    ListActiveSessions(StreamingListActiveSessionsParams),
}

#[derive(Debug, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "camelCase")]
pub enum StreamHostCommand {
    ExchangeOffer(ExchangeOfferParams),
    ExchangeIce(ExchangeIceParams),
    KeepAliveRemoteSession(KeepAliveRemoteSessionParams),
    CloseRemoteSession(CloseRemoteSessionParams),
}

pub async fn handle_rpc(
    method: &str,
    params: Option<Value>,
    app_handle: AppHandle,
) -> AppResult<Value> {
    let state = app_handle.state::<AppState>();
    let service = state.streaming.clone();

    let json_cmd = match params {
        Some(p) => json!({ "method": method, "params": p }),
        None => json!({ "method": method }),
    };

    let command: StreamingCommand = serde_json::from_value(json_cmd).map_err(|e| {
        crate::error::AppError::InvalidParams(format!("Invalid streaming command params: {}", e))
    })?;

    match command {
        StreamingCommand::GetFallbackTurnServer { target_type } => {
            if target_type != "home" && target_type != "cloud" {
                return Err(crate::error::AppError::InvalidParams(
                    "targetType must be home or cloud".to_string(),
                ));
            }
            Ok(serde_json::to_value(
                service.get_fallback_turn_server(&target_type).await?,
            )?)
        }
        StreamingCommand::CreateSession(payload) => Ok(serde_json::to_value(
            service.create_session(payload).await?,
        )?),
        StreamingCommand::GetSession(payload) => {
            Ok(serde_json::to_value(service.get_session(payload).await?)?)
        }
        StreamingCommand::CloseSession(payload) => {
            Ok(serde_json::to_value(service.close_session(payload).await?)?)
        }
        StreamingCommand::ExchangeOffer(payload) => Ok(serde_json::to_value(
            service.exchange_offer(payload).await?,
        )?),
        StreamingCommand::ExchangeIce(payload) => {
            Ok(serde_json::to_value(service.exchange_ice(payload).await?)?)
        }
        StreamingCommand::SendKeepAlive(payload) => Ok(serde_json::to_value(
            service.send_keepalive(payload).await?,
        )?),
        StreamingCommand::ListActiveSessions(payload) => Ok(serde_json::to_value(
            service.list_active_sessions(payload).await?,
        )?),
    }
}

pub async fn handle_stream_host_rpc(
    method: &str,
    params: Option<Value>,
    app_handle: AppHandle,
) -> AppResult<Value> {
    let state = app_handle.state::<AppState>();
    let service = state.streaming.clone();

    let json_cmd = match params {
        Some(p) => json!({ "method": method, "params": p }),
        None => json!({ "method": method }),
    };

    let command: StreamHostCommand = serde_json::from_value(json_cmd).map_err(|e| {
        crate::error::AppError::InvalidParams(format!("Invalid streamHost command params: {}", e))
    })?;

    match command {
        StreamHostCommand::ExchangeOffer(payload) => {
            if payload.session_id.trim().is_empty() {
                return Err(crate::error::AppError::InvalidParams(
                    "sessionId must not be empty".to_string(),
                ));
            }
            if payload.sdp.trim().is_empty() {
                return Err(crate::error::AppError::InvalidParams(
                    "sdp must not be empty".to_string(),
                ));
            }

            let answer_sdp = service
                .exchange_offer_sdp(
                    payload.session_id,
                    Some(payload.channel.as_str().to_string()),
                    payload.sdp,
                )
                .await?;
            Ok(json!({ "answerSdp": answer_sdp }))
        }
        StreamHostCommand::ExchangeIce(payload) => {
            if payload.session_id.trim().is_empty() {
                return Err(crate::error::AppError::InvalidParams(
                    "sessionId must not be empty".to_string(),
                ));
            }

            let result = service
                .exchange_ice_candidates(payload.session_id, payload.candidates)
                .await?;
            Ok(json!({ "candidates": result }))
        }
        StreamHostCommand::KeepAliveRemoteSession(payload) => {
            if payload.session_id.trim().is_empty() {
                return Err(crate::error::AppError::InvalidParams(
                    "sessionId must not be empty".to_string(),
                ));
            }

            let accepted = service
                .keep_alive_remote_session(payload.session_id)
                .await?;
            Ok(json!({ "accepted": accepted }))
        }
        StreamHostCommand::CloseRemoteSession(payload) => {
            if payload.session_id.trim().is_empty() {
                return Err(crate::error::AppError::InvalidParams(
                    "sessionId must not be empty".to_string(),
                ));
            }

            let closed = service.close_remote_session(payload.session_id).await?;
            Ok(json!({ "closed": closed }))
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExchangeOfferParams {
    pub session_id: String,
    pub channel: StreamHostChannel,
    pub sdp: String,
    pub _restart: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StreamHostChannel {
    Media,
    Chat,
}

impl StreamHostChannel {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Media => "media",
            Self::Chat => "chat",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExchangeIceParams {
    pub session_id: String,
    pub candidates: Vec<StreamingIceCandidate>,
    pub _restart: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KeepAliveRemoteSessionParams {
    pub session_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CloseRemoteSessionParams {
    pub session_id: String,
    pub _reason: Option<String>,
}
