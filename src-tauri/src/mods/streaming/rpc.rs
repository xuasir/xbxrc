use crate::error::AppResult;
use crate::mods::streaming::types::*;
use crate::AppState;
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

#[derive(Debug, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "camelCase")]
pub enum StreamingCommand {
    StartSession(StreamingStartSessionParams),
    GetSessionProgress(StreamingGetSessionProgressParams),
    CloseSession(StreamingCloseSessionParams),
    ExchangeOffer(StreamingExchangeOfferParams),
    SubmitIce(StreamingSubmitIceParams),
    PollIce(StreamingPollIceParams),
    ListActiveSessions(StreamingListActiveSessionsParams),
    DecideRecovery(StreamingDecideRecoveryParams),
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
        StreamingCommand::StartSession(payload) => {
            Ok(serde_json::to_value(service.start_session(payload).await?)?)
        }
        StreamingCommand::GetSessionProgress(payload) => Ok(serde_json::to_value(
            service.get_session_progress(payload).await?,
        )?),
        StreamingCommand::CloseSession(payload) => {
            Ok(serde_json::to_value(service.close_session(payload).await?)?)
        }
        StreamingCommand::ExchangeOffer(payload) => Ok(serde_json::to_value(
            service.exchange_offer(payload).await?,
        )?),
        StreamingCommand::SubmitIce(payload) => {
            Ok(serde_json::to_value(service.submit_ice(payload).await?)?)
        }
        StreamingCommand::PollIce(payload) => {
            Ok(serde_json::to_value(service.poll_ice(payload).await?)?)
        }
        StreamingCommand::ListActiveSessions(payload) => Ok(serde_json::to_value(
            service.list_active_sessions(payload).await?,
        )?),
        StreamingCommand::DecideRecovery(payload) => Ok(serde_json::to_value(
            service.decide_recovery(payload).await?,
        )?),
    }
}
