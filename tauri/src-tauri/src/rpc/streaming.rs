use crate::mods::streaming::types::*;
use crate::AppState;
use serde_json::Value;
use tauri::{AppHandle, Manager};

pub async fn handle_streaming_rpc(
    method: &str,
    params: Option<Value>,
    app_handle: AppHandle,
) -> Result<Value, String> {
    let state = app_handle.state::<AppState>();
    let service = state.streaming.clone();

    match method {
        "getFallbackTurnServer" => {
            let target_type = params
                .as_ref()
                .and_then(|payload| payload.get("targetType"))
                .and_then(Value::as_str)
                .ok_or("Missing targetType parameter")?;
            if target_type != "home" && target_type != "cloud" {
                return Err("targetType must be home or cloud".to_string());
            }

            let result = service.get_fallback_turn_server(target_type).await?;
            serde_json::to_value(result).map_err(|error| error.to_string())
        }
        "createSession" => {
            let payload = serde_json::from_value::<StreamingCreateSessionParams>(
                params.ok_or("Missing params for streaming.createSession")?,
            )
            .map_err(|error| error.to_string())?;
            let result = service.create_session(payload).await?;
            serde_json::to_value(result).map_err(|error| error.to_string())
        }
        "getSession" => {
            let payload = serde_json::from_value::<StreamingGetSessionParams>(
                params.ok_or("Missing params for streaming.getSession")?,
            )
            .map_err(|error| error.to_string())?;
            let result = service.get_session(payload).await?;
            serde_json::to_value(result).map_err(|error| error.to_string())
        }
        "closeSession" => {
            let payload = serde_json::from_value::<StreamingCloseSessionParams>(
                params.ok_or("Missing params for streaming.closeSession")?,
            )
            .map_err(|error| error.to_string())?;
            service.close_session(payload).await
        }
        "exchangeOffer" => {
            let payload = serde_json::from_value::<StreamingExchangeOfferParams>(
                params.ok_or("Missing params for streaming.exchangeOffer")?,
            )
            .map_err(|error| error.to_string())?;
            let result = service.exchange_offer(payload).await?;
            serde_json::to_value(result).map_err(|error| error.to_string())
        }
        "exchangeIce" => {
            let payload = serde_json::from_value::<StreamingExchangeIceParams>(
                params.ok_or("Missing params for streaming.exchangeIce")?,
            )
            .map_err(|error| error.to_string())?;
            let result = service.exchange_ice(payload).await?;
            serde_json::to_value(result).map_err(|error| error.to_string())
        }
        "sendKeepAlive" => {
            let payload = serde_json::from_value::<StreamingKeepAliveParams>(
                params.ok_or("Missing params for streaming.sendKeepAlive")?,
            )
            .map_err(|error| error.to_string())?;
            let result = service.send_keepalive(payload).await?;
            serde_json::to_value(result).map_err(|error| error.to_string())
        }
        "listActiveSessions" => {
            let payload = match params {
                Some(value) => serde_json::from_value::<StreamingListActiveSessionsParams>(value)
                    .map_err(|error| error.to_string())?,
                None => StreamingListActiveSessionsParams::default(),
            };
            let result = service.list_active_sessions(payload).await?;
            serde_json::to_value(result).map_err(|error| error.to_string())
        }
        _ => Err(format!("Unknown method in streaming: {}", method)),
    }
}
