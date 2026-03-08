use crate::mods::streaming::types::StreamingIceCandidate;
use crate::AppState;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ExchangeOfferParams {
    session_id: String,
    channel: StreamHostChannel,
    sdp: String,
    restart: Option<bool>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
enum StreamHostChannel {
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

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ExchangeIceParams {
    session_id: String,
    candidates: Vec<StreamingIceCandidate>,
    restart: Option<bool>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct KeepAliveRemoteSessionParams {
    session_id: String,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CloseRemoteSessionParams {
    session_id: String,
    reason: Option<String>,
}

pub async fn handle_stream_host_rpc(
    method: &str,
    params: Option<Value>,
    app_handle: AppHandle,
) -> Result<Value, String> {
    let state = app_handle.state::<AppState>();
    let service = state.streaming.clone();

    match method {
        "exchangeOffer" => {
            let payload = serde_json::from_value::<ExchangeOfferParams>(
                params.ok_or("Missing params for streamHost.exchangeOffer")?,
            )
            .map_err(|error| error.to_string())?;
            if payload.session_id.trim().is_empty() {
                return Err("sessionId must not be empty".to_string());
            }
            if payload.sdp.trim().is_empty() {
                return Err("sdp must not be empty".to_string());
            }
            let _ = payload.restart;

            let answer_sdp = service
                .exchange_offer_sdp(
                    payload.session_id,
                    Some(payload.channel.as_str().to_string()),
                    payload.sdp,
                )
                .await?;
            Ok(json!({ "answerSdp": answer_sdp }))
        }
        "exchangeIce" => {
            let payload = serde_json::from_value::<ExchangeIceParams>(
                params.ok_or("Missing params for streamHost.exchangeIce")?,
            )
            .map_err(|error| error.to_string())?;
            if payload.session_id.trim().is_empty() {
                return Err("sessionId must not be empty".to_string());
            }
            let _ = payload.restart;

            let result = service
                .exchange_ice_candidates(payload.session_id, payload.candidates)
                .await?;
            Ok(json!({ "candidates": result }))
        }
        "keepAliveRemoteSession" => {
            let payload = serde_json::from_value::<KeepAliveRemoteSessionParams>(
                params.ok_or("Missing params for streamHost.keepAliveRemoteSession")?,
            )
            .map_err(|error| error.to_string())?;
            if payload.session_id.trim().is_empty() {
                return Err("sessionId must not be empty".to_string());
            }

            let accepted = service
                .keep_alive_remote_session(payload.session_id)
                .await?;
            Ok(json!({ "accepted": accepted }))
        }
        "closeRemoteSession" => {
            let payload = serde_json::from_value::<CloseRemoteSessionParams>(
                params.ok_or("Missing params for streamHost.closeRemoteSession")?,
            )
            .map_err(|error| error.to_string())?;
            if payload.session_id.trim().is_empty() {
                return Err("sessionId must not be empty".to_string());
            }
            let _ = payload.reason;

            let closed = service.close_remote_session(payload.session_id).await?;
            Ok(json!({ "closed": closed }))
        }
        _ => Err(format!("Unknown method in streamHost: {}", method)),
    }
}
